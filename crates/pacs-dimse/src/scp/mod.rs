//! pacs-rs as an association acceptor: Verification SCP (PS3.4 Annex A), Storage SCP
//! (PS3.4 Annex B) for the SOP classes of [`crate::sop_classes::STORAGE`], and Query and
//! Retrieve SCP (PS3.4 Annex C) on the Study Root model.
//!
//! Every accepted AE title shares one set of presentation contexts; once an association is
//! up, its called AE title picks where received instances go and which studies a query
//! sees ([`Scp::destination`]). The association loop and the Verification SCP are here,
//! the Storage SCP in [`store`], the Query SCP in [`find`], the Retrieve SCP in
//! [`retrieve`].

mod find;
mod operation;
mod retrieve;
mod store;

use operation::After;

pub use store::{Destination, part10};

use pacs_archive::observed::{Stored, stored_during};

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

/// An association as whoever accounts for it sees it: where the peer is, and the AE
/// titles it called from and called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub address: std::net::SocketAddr,
    pub calling: String,
    pub called: String,
}

/// A C-FIND that was answered, or refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queried<'a> {
    /// The Query/Retrieve Information Model asked: the C-FIND's SOP Class UID.
    pub sop_class: &'a str,
    /// The transfer syntax the Identifier is written in.
    pub transfer_syntax: &'a str,
    /// The Request Identifier as received; empty when the request had none.
    pub identifier: &'a [u8],
    /// The final status: Success, Cancel, or the Failure it was refused with.
    pub status: Status,
    /// How many matches were sent.
    pub matches: usize,
}

/// A retrieval that was answered, or refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retrieved<'a> {
    /// The Query/Retrieve Information Model asked: the C-MOVE's SOP Class UID.
    pub sop_class: &'a str,
    /// The AE title the instances were to go to: the Move Destination, as it was named.
    pub destination: &'a str,
    /// Its host, when it is a destination this server sends to.
    pub destination_host: Option<&'a str>,
    /// The final status.
    pub status: Status,
    /// What the destination stored, a study at a time; none when nothing was sent.
    pub studies: &'a [SentStudy],
}

/// The instances of one study that a destination stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentStudy {
    pub study_instance_uid: String,
    pub patient_id: Option<String>,
    pub patient_name: Option<String>,
    /// SOP Class UID and how many instances of it were stored.
    pub sop_classes: Vec<(String, u32)>,
}

/// Told what associations did, for the server's audit trail. Called on the association's
/// own task, after the fact: it must not block for long.
pub trait Observer: Send + Sync + std::fmt::Debug {
    /// A C-FIND is over.
    fn queried(&self, peer: &Peer, query: &Queried<'_>);
    /// A C-MOVE is over.
    fn retrieved(&self, peer: &Peer, retrieval: &Retrieved<'_>);
    /// The association is over, and these instances were stored through it, in order.
    /// Never called with none.
    fn received(&self, peer: &Peer, stored: Vec<Stored>);
    /// A connection from `address` did not become an association, for `reason`: an AE
    /// title that is not recognized, a request that cannot be read, the limit reached.
    fn refused(&self, address: std::net::SocketAddr, reason: &str);
}

/// The DIMSE service provider: configuration, the destination of received instances, and
/// the cap on associations served at once.
#[derive(Debug)]
pub struct Scp {
    config: Config,
    destination: Destination,
    /// Further called AE titles with destinations of their own (projects).
    routes: Vec<(AeTitle, Destination)>,
    slots: Arc<Semaphore>,
    observer: Option<Arc<dyn Observer>>,
}

impl Scp {
    pub fn new(config: Config, destination: Destination) -> Self {
        let slots = Arc::new(Semaphore::new(config.max_associations));
        Scp {
            config,
            destination,
            routes: Vec::new(),
            slots,
            observer: None,
        }
    }

    /// Tell `observer` what associations did.
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = Some(observer);
        self
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
            .with_abstract_syntax(VERIFICATION)
            .with_abstract_syntax(crate::query::STUDY_ROOT_FIND)
            .with_abstract_syntax(crate::scu::retrieve::STUDY_ROOT_MOVE);
        for (uid, _) in crate::sop_classes::STORAGE {
            options = options.with_abstract_syntax(*uid);
        }
        if let Some(tls) = &self.config.tls {
            options = options.tls_config(tls.clone());
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
                    if let Some(observer) = &scp.observer {
                        observer
                            .refused(peer, "the limit of associations served at once is reached");
                    }
                    // Over TLS there is no handshake to spare for a rejection: the
                    // connection is closed, which a caller retries just the same.
                    if scp.config.tls.is_none() {
                        reject_busy(socket, scp.config.timeout).await;
                    }
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
        let refused = |e: dicom_ul::association::Error| {
            tracing::info!(%peer, error = %e, "association not established");
            if let Some(observer) = &self.observer {
                observer.refused(peer, &e.to_string());
            }
        };
        // With TLS configured the port speaks TLS only (PS3.15 B.12: strict TLS, no
        // upgrade from plain): a caller that does not open with a handshake the
        // configuration accepts is refused here, before any DICOM.
        if self.config.tls.is_some() {
            match options.establish_tls_async(socket).await {
                Ok(association) => self.established(association, peer).await,
                Err(e) => refused(e),
            }
        } else {
            match options.establish_async(socket).await {
                Ok(association) => self.established(association, peer).await,
                Err(e) => refused(e),
            }
        }
    }

    async fn established<S>(
        &self,
        association: dicom_ul::association::server::AsyncServerAssociation<S>,
        peer: std::net::SocketAddr,
    ) where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        let calling = association.peer_ae_title().trim().to_owned();
        let called = association.called_ae_title().trim().to_owned();
        let destination = self.destination(&called).clone();
        tracing::info!(%peer, calling, called, "association established");
        // what the archive takes in while the association lasts: its messages are served
        // on this task, one after the other
        let messages = self.messages(association, peer, &calling, &called, &destination);
        let ((), stored) = stored_during(messages).await;
        if let Some(observer) = &self.observer
            && !stored.is_empty()
        {
            let peer = Peer {
                address: peer,
                calling,
                called,
            };
            observer.received(&peer, stored);
        }
    }

    /// Whether `calling` may query: it is one of the AEs this server is configured with.
    /// An AE title is a claim, not a proof; who may connect at all is the listener's
    /// business (TLS, known callers).
    fn answers_queries_of(&self, calling: &str) -> bool {
        self.config
            .remotes
            .iter()
            .any(|remote| remote.ae_title.matches(calling))
    }

    /// Answer a C-FIND and tell the observer. Whether the association goes on.
    async fn find<A, S>(
        &self,
        association: &mut A,
        peer: std::net::SocketAddr,
        request: &Request<'_>,
        command: &Command,
        identifier: DataSet,
        destination: &Destination,
    ) -> bool
    where
        A: dicom_ul::association::AsyncAssociation<S> + Association + Send,
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        let calling = request.calling;
        let after = find::find(
            association,
            request,
            command,
            identifier,
            &destination.index(),
            self.answers_queries_of(calling),
        )
        .await;
        let (found, open) = match after {
            Ok(After::Open(found)) => (found, true),
            Ok(After::Released(found) | After::Aborted(found)) => (found, false),
            Err(e) => {
                tracing::warn!(%peer, calling, error = %e, "association aborted during a C-FIND");
                let _ = association.abort().await;
                return false;
            }
        };
        tracing::info!(%peer, calling, called = request.called, status = %found.status, matches = found.matches, "C-FIND");
        if let Some(observer) = &self.observer {
            observer.queried(
                &Peer {
                    address: peer,
                    calling: calling.to_owned(),
                    called: request.called.to_owned(),
                },
                &Queried {
                    sop_class: request.abstract_syntax,
                    transfer_syntax: request.transfer_syntax,
                    identifier: &found.identifier,
                    status: found.status,
                    matches: found.matches,
                },
            );
        }
        if !open {
            tracing::info!(%peer, calling, "association ended by the peer during a C-FIND");
        }
        open
    }

    /// Answer a C-MOVE and tell the observer. Whether the association goes on.
    async fn retrieve<A, S>(
        &self,
        association: &mut A,
        peer: std::net::SocketAddr,
        request: &Request<'_>,
        command: &Command,
        identifier: DataSet,
        destination: &Destination,
    ) -> bool
    where
        A: dicom_ul::association::AsyncAssociation<S> + Association + Send,
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        let calling = request.calling;
        let index = destination.index();
        let after = retrieve::c_move(
            association,
            request,
            command,
            identifier,
            destination,
            &self.config,
            self.answers_queries_of(calling),
        )
        .await;
        let (retrieved, open) = match after {
            Ok(After::Open(retrieved)) => (retrieved, true),
            Ok(After::Released(retrieved) | After::Aborted(retrieved)) => (retrieved, false),
            Err(e) => {
                tracing::warn!(%peer, calling, error = %e, "association aborted during a C-MOVE");
                let _ = association.abort().await;
                return false;
            }
        };
        let counts = &retrieved.counts;
        tracing::info!(%peer, calling, called = request.called, destination = %retrieved.destination, status = %retrieved.status, completed = counts.completed, failed = counts.failed, warning = counts.warning, remaining = counts.remaining, "C-MOVE");
        if let Some(observer) = &self.observer {
            let studies = sent_studies(&index, &retrieved.sent).await;
            observer.retrieved(
                &Peer {
                    address: peer,
                    calling: calling.to_owned(),
                    called: request.called.to_owned(),
                },
                &Retrieved {
                    sop_class: request.abstract_syntax,
                    destination: &retrieved.destination,
                    destination_host: retrieved.destination_host.as_deref(),
                    status: retrieved.status,
                    studies: &studies,
                },
            );
        }
        if !open {
            tracing::info!(%peer, calling, "association ended by the peer during a C-MOVE");
        }
        open
    }

    /// Serve the messages of an established association until it ends.
    async fn messages<A, S>(
        &self,
        mut association: A,
        peer: std::net::SocketAddr,
        calling: &str,
        called: &str,
        destination: &Destination,
    ) where
        A: dicom_ul::association::AsyncAssociation<S> + Association + Send,
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
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
                        calling,
                        called,
                    };
                    let response =
                        match handle(&request, &message.command, message.data, destination).await {
                            Handled::Respond(response) => response,
                            Handled::Nothing => continue,
                            Handled::Find(identifier) => {
                                let open = self
                                    .find(
                                        &mut association,
                                        peer,
                                        &request,
                                        &message.command,
                                        identifier,
                                        destination,
                                    )
                                    .await;
                                served.found += 1;
                                if open {
                                    continue;
                                }
                                break;
                            }
                            Handled::Move(identifier) => {
                                let open = self
                                    .retrieve(
                                        &mut association,
                                        peer,
                                        &request,
                                        &message.command,
                                        identifier,
                                        destination,
                                    )
                                    .await;
                                served.retrieved += 1;
                                if open {
                                    continue;
                                }
                                break;
                            }
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
                    tracing::info!(%peer, calling, echoes = served.echoes, stored = served.stored, found = served.found, retrieved = served.retrieved, failed = served.failed, "association released");
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

/// What was sent, a study at a time in the order the studies were first sent from, each
/// with whose it is as `index` has it: for the audit trail, which names the patient.
async fn sent_studies(index: &pacs_index::Index, sent: &[retrieve::Sent]) -> Vec<SentStudy> {
    let mut studies: Vec<SentStudy> = Vec::new();
    for instance in sent {
        let at = match studies
            .iter()
            .position(|study| study.study_instance_uid == instance.study_instance_uid)
        {
            Some(at) => at,
            None => {
                // a study that went while it was being sent has no patient to name
                let known = index
                    .audited_study(&instance.study_instance_uid)
                    .await
                    .ok()
                    .flatten();
                studies.push(SentStudy {
                    study_instance_uid: instance.study_instance_uid.clone(),
                    patient_id: known.as_ref().and_then(|study| study.patient_id.clone()),
                    patient_name: known.and_then(|study| study.patient_name),
                    sop_classes: Vec::new(),
                });
                studies.len() - 1
            }
        };
        let classes = &mut studies[at].sop_classes;
        match classes
            .iter_mut()
            .find(|(sop_class, _)| *sop_class == instance.sop_class_uid)
        {
            Some((_, count)) => *count += 1,
            None => classes.push((instance.sop_class_uid.clone(), 1)),
        }
    }
    studies
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
    /// A C-FIND-RQ with this Identifier, which the Query SCP answers with as many
    /// responses as it has matches.
    Find(DataSet),
    /// A C-MOVE-RQ with this Identifier, which the Retrieve SCP answers as its
    /// sub-operations go.
    Move(DataSet),
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
        field::C_STORE_RQ if is_storage(affected) => {
            let (status, comment) = store::store(request, command, data, destination).await;
            respond(status, comment.as_deref())
        }
        field::C_FIND_RQ if find::is_find(affected) => Handled::Find(data),
        field::C_MOVE_RQ if retrieve::is_move(affected) => Handled::Move(data),
        // PS3.7 9.1.5.1.4, 9.1.1.1.9: an operation the SOP class does not have, or one
        // pacs-rs does not provide.
        _ => respond(Status::UNRECOGNIZED_OPERATION, None),
    }
}

fn is_storage(sop_class: &str) -> bool {
    crate::sop_classes::STORAGE
        .iter()
        .any(|(uid, _)| *uid == sop_class)
}

fn is_supported(sop_class: &str) -> bool {
    sop_class == VERIFICATION
        || find::is_find(sop_class)
        || retrieve::is_move(sop_class)
        || is_storage(sop_class)
}

#[derive(Default)]
struct Counts {
    echoes: usize,
    stored: usize,
    found: usize,
    retrieved: usize,
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
