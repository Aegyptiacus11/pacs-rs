//! pacs-rs as an association acceptor: Verification SCP (PS3.4 Annex A) and Storage SCP
//! (PS3.4 Annex B) for the SOP classes of [`crate::sop_classes::STORAGE`].
//!
//! Every accepted AE title shares one set of presentation contexts; once an association is
//! up, its called AE title picks where received instances go ([`Scp::destination`]). The
//! association loop and the Verification SCP are here, the Storage SCP in [`store`].

mod store;

pub use store::{Destination, part10};

use std::sync::Arc;

use dicom_ul::association::Association;
use dicom_ul::association::server::AccessControl;
use dicom_ul::pdu::{
    AssociationRJ, AssociationRJResult, AssociationRJServiceProviderPresentationReason,
    AssociationRJServiceUserReason, AssociationRJSource, PresentationContextResultReason,
    UserIdentity,
};
use dicom_ul::{Pdu, ServerAssociationOptions};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use crate::ae::AeTitle;
use crate::command::{self, Command, field, tag};
use crate::message::{self, DataSet, Incoming, MessageError};
use crate::status::Status;
use crate::{Config, VERIFICATION};

/// The DIMSE service provider: configuration, the destination of received instances, and
/// the cap on associations served at once.
#[derive(Debug)]
pub struct Scp {
    config: Config,
    destination: Destination,
    /// Further called AE titles with destinations of their own (projects).
    routes: Vec<(AeTitle, Destination)>,
    slots: Arc<Semaphore>,
}

impl Scp {
    pub fn new(config: Config, destination: Destination) -> Self {
        let slots = Arc::new(Semaphore::new(config.max_associations));
        Scp {
            config,
            destination,
            routes: Vec::new(),
            slots,
        }
    }

    /// Accept `ae_title` too, its instances going to `destination`.
    pub fn with_route(mut self, ae_title: AeTitle, destination: Destination) -> Self {
        self.routes.push((ae_title, destination));
        self
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Where instances sent to `called_ae_title` go: its route's destination, else the
    /// default one, for pacs-rs's own AE title and the extra ones.
    pub fn destination(&self, called_ae_title: &str) -> &Destination {
        self.routes
            .iter()
            .find(|(title, _)| title.matches(called_ae_title))
            .map(|(_, d)| d)
            .unwrap_or(&self.destination)
    }

    fn options(
        &self,
    ) -> ServerAssociationOptions<'static, Titles, dicom_ul::association::server::DefaultNegotiation>
    {
        let mut accepted = vec![self.config.ae_title.clone()];
        accepted.extend(self.config.extra_ae_titles.iter().cloned());
        accepted.extend(self.routes.iter().map(|(title, _)| title.clone()));
        let callers = self.config.known_callers_only.then(|| {
            self.config
                .remotes
                .iter()
                .map(|r| r.ae_title.clone())
                .collect()
        });
        let mut options = ServerAssociationOptions::new()
            .ae_access_control(Titles { accepted, callers })
            .ae_title(self.config.ae_title.as_str().to_owned())
            .read_timeout(self.config.timeout)
            .write_timeout(self.config.timeout)
            .with_abstract_syntax(VERIFICATION);
        for (uid, _) in crate::sop_classes::STORAGE {
            options = options.with_abstract_syntax(*uid);
        }
        options
    }

    /// Accept associations on `listener` until `shutdown` completes, each in its own task.
    pub async fn serve(
        self: Arc<Self>,
        listener: TcpListener,
        shutdown: impl Future<Output = ()> + Send,
    ) {
        let options = Arc::new(self.options());
        tokio::pin!(shutdown);
        loop {
            let (socket, peer) = tokio::select! {
                () = &mut shutdown => return,
                accepted = listener.accept() => match accepted {
                    Ok(accepted) => accepted,
                    Err(e) => {
                        tracing::warn!(error = %e, "DIMSE accept failed");
                        continue;
                    }
                },
            };
            let scp = self.clone();
            let options = options.clone();
            tokio::spawn(async move {
                let Ok(_slot) = scp.slots.clone().try_acquire_owned() else {
                    tracing::warn!(%peer, "association rejected: the limit of {} is reached", scp.config.max_associations);
                    reject_busy(socket, scp.config.timeout).await;
                    return;
                };
                scp.association(&options, socket, peer).await;
            });
        }
    }

    async fn association(
        &self,
        options: &ServerAssociationOptions<
            'static,
            Titles,
            dicom_ul::association::server::DefaultNegotiation,
        >,
        socket: TcpStream,
        peer: std::net::SocketAddr,
    ) {
        let mut association = match options.establish_async(socket).await {
            Ok(association) => association,
            Err(e) => {
                tracing::info!(%peer, error = %e, "association not established");
                return;
            }
        };
        let calling = association.peer_ae_title().trim().to_owned();
        let called = association.called_ae_title().trim().to_owned();
        let destination = self.destination(&called).clone();
        tracing::info!(%peer, calling, called, "association established");
        let mut served = Counts::default();
        loop {
            match message::receive(&mut association, self.config.max_data_set_bytes).await {
                Ok(Incoming::Message(message)) => {
                    let Some(context) = association
                        .presentation_contexts()
                        .iter()
                        .find(|pc| {
                            pc.id == message.context_id
                                && pc.reason == PresentationContextResultReason::Acceptance
                        })
                        .cloned()
                    else {
                        // PS3.8 9.3.5: P-DATA only on accepted presentation contexts.
                        tracing::warn!(%peer, calling, "a message on presentation context {} that was not accepted", message.context_id);
                        let _ = association.abort().await;
                        break;
                    };
                    let request = Request {
                        context_id: message.context_id,
                        abstract_syntax: context.abstract_syntax.trim_end_matches('\0'),
                        transfer_syntax: context.transfer_syntax.trim_end_matches('\0'),
                        calling: &calling,
                        called: &called,
                    };
                    let response = match handle(
                        &request,
                        &message.command,
                        message.data,
                        &destination,
                    )
                    .await
                    {
                        Handled::Respond(response) => response,
                        Handled::Nothing => continue,
                        Handled::Abort(reason) => {
                            tracing::warn!(%peer, calling, reason, "association aborted");
                            let _ = association.abort().await;
                            break;
                        }
                    };
                    served.count(&message.command, &response);
                    if let Err(e) =
                        message::send(&mut association, request.context_id, &response, None).await
                    {
                        tracing::info!(%peer, calling, error = %e, "the response could not be sent");
                        break;
                    }
                }
                Ok(Incoming::Released) => {
                    tracing::info!(%peer, calling, echoes = served.echoes, stored = served.stored, failed = served.failed, "association released");
                    break;
                }
                Ok(Incoming::Aborted) => {
                    tracing::info!(%peer, calling, stored = served.stored, "association aborted by the peer");
                    break;
                }
                Err(MessageError::Ul(e)) => {
                    // a timeout or a dropped connection: PS3.8 9.1.5, A-ABORT on what is left
                    tracing::info!(%peer, calling, error = %e, "association ended");
                    let _ = association.abort().await;
                    break;
                }
                Err(e) => {
                    tracing::warn!(%peer, calling, error = %e, "association aborted");
                    let _ = association.abort().await;
                    break;
                }
            }
        }
    }
}

/// Accepted when the called AE title is one of pacs-rs's and, with `callers` set, the
/// calling AE title is one of those (PS3.8 Table 9-21, reasons 7 and 3).
#[derive(Debug, Clone)]
pub struct Titles {
    accepted: Vec<AeTitle>,
    callers: Option<Vec<AeTitle>>,
}

impl AccessControl for Titles {
    fn check_access(
        &self,
        _this_ae_title: &str,
        calling_ae_title: &str,
        called_ae_title: &str,
        _user_identity: Option<&UserIdentity>,
    ) -> Result<(), AssociationRJServiceUserReason> {
        if !self.accepted.iter().any(|t| t.matches(called_ae_title)) {
            return Err(AssociationRJServiceUserReason::CalledAETitleNotRecognized);
        }
        if let Some(callers) = &self.callers
            && !callers.iter().any(|t| t.matches(calling_ae_title))
        {
            return Err(AssociationRJServiceUserReason::CallingAETitleNotRecognized);
        }
        Ok(())
    }
}

/// Over the limit: read the A-ASSOCIATE-RQ and answer A-ASSOCIATE-RJ, rejected-transient,
/// service-provider (presentation), local-limit-exceeded (PS3.8 9.3.4, Table 9-21).
async fn reject_busy(mut socket: TcpStream, timeout: std::time::Duration) {
    let reject = async {
        let mut buffer = bytes::BytesMut::new();
        let request = dicom_ul::association::read_pdu_from_wire_async(
            &mut socket,
            &mut buffer,
            dicom_ul::pdu::MAXIMUM_PDU_SIZE,
            false,
        )
        .await;
        if !matches!(request, Ok(Pdu::AssociationRQ(_))) {
            return;
        }
        let mut out = Vec::new();
        let rj = Pdu::AssociationRJ(AssociationRJ {
            result: AssociationRJResult::Transient,
            source: AssociationRJSource::ServiceProviderPresentation(
                AssociationRJServiceProviderPresentationReason::LocalLimitExceeded,
            ),
        });
        if dicom_ul::write_pdu(&mut out, &rj).is_ok() {
            let _ = socket.write_all(&out).await;
            let _ = socket.shutdown().await;
        }
    };
    let _ = tokio::time::timeout(timeout, reject).await;
}

pub(crate) struct Request<'a> {
    pub context_id: u8,
    pub abstract_syntax: &'a str,
    pub transfer_syntax: &'a str,
    pub calling: &'a str,
    pub called: &'a str,
}

enum Handled {
    Respond(Command),
    /// No response is due (a C-CANCEL-RQ with nothing to cancel).
    Nothing,
    Abort(&'static str),
}

async fn handle(
    request: &Request<'_>,
    command: &Command,
    data: DataSet,
    destination: &Destination,
) -> Handled {
    let Ok(field) = command.command_field() else {
        return Handled::Abort("a command without Command Field");
    };
    if field & field::RESPONSE_BIT != 0 {
        return Handled::Abort("a response, but pacs-rs has sent no request");
    }
    if field == field::C_CANCEL_RQ {
        return Handled::Nothing;
    }
    if command.message_id().is_err() {
        return Handled::Abort("a request without Message ID");
    }
    let respond = |status: Status, comment: Option<&str>| {
        Handled::Respond(command::response(command, status, comment))
    };
    // PS3.7 9.1.1.1.9 and 9.1.5.1.4: Refused: SOP Class not supported when pacs-rs has no
    // such SOP class; Mistyped argument when it is not the one agreed for this context.
    let affected = command.ui(tag::AFFECTED_SOP_CLASS_UID).unwrap_or("");
    if !is_supported(affected) {
        return respond(
            Status::SOP_CLASS_NOT_SUPPORTED,
            Some(&format!("SOP Class {affected} is not supported")),
        );
    }
    if affected != request.abstract_syntax {
        return respond(
            Status::MISTYPED_ARGUMENT,
            Some(&format!(
                "Affected SOP Class UID {affected} is not this presentation context's {}",
                request.abstract_syntax
            )),
        );
    }
    match field {
        field::C_ECHO_RQ if affected == VERIFICATION => respond(Status::SUCCESS, None),
        field::C_STORE_RQ if affected != VERIFICATION => {
            let (status, comment) = store::store(request, command, data, destination).await;
            respond(status, comment.as_deref())
        }
        // PS3.7 9.1.5.1.4, 9.1.1.1.9: an operation the SOP class does not have, or one
        // pacs-rs does not provide.
        _ => respond(Status::UNRECOGNIZED_OPERATION, None),
    }
}

fn is_supported(sop_class: &str) -> bool {
    sop_class == VERIFICATION
        || crate::sop_classes::STORAGE
            .iter()
            .any(|(uid, _)| *uid == sop_class)
}

#[derive(Default)]
struct Counts {
    echoes: usize,
    stored: usize,
    failed: usize,
}

impl Counts {
    fn count(&mut self, request: &Command, response: &Command) {
        let ok = response.status().is_ok_and(|s| s == Status::SUCCESS);
        match request.command_field() {
            Ok(field::C_ECHO_RQ) => self.echoes += 1,
            Ok(field::C_STORE_RQ) if ok => self.stored += 1,
            _ => self.failed += usize::from(!ok),
        }
    }
}
