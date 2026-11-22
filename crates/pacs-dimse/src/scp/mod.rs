//! pacs-rs as an association acceptor: Verification SCP (PS3.4 Annex A), Storage SCP
//! (PS3.4 Annex B) for the SOP classes of [`crate::sop_classes::STORAGE`], and Query and
//! Retrieve SCP (PS3.4 Annex C) on the Patient Root and Study Root models.
//!
//! Every accepted AE title shares one set of presentation contexts; once an association is
//! up, its called AE title picks where received instances go and which studies a query
//! sees ([`Scp::destination`]). The association loop and the Verification SCP are here,
//! the Storage SCP in [`store`], the Query SCP in [`find`], the Retrieve SCP in
//! [`retrieve`].

mod find;
mod operation;
mod retrieve;
mod services;
pub(crate) mod store;

use operation::After;

pub use services::{Capabilities, Service};
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

use crate::Config;
use crate::ae::AeTitle;
use crate::command::{self, Command, field, tag};
use crate::message::{self, DataSet, Incoming, MessageError};
use crate::status::Status;

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
    /// `C-MOVE` or `C-GET`.
    pub operation: &'static str,
    /// The Query/Retrieve Information Model asked: the request's SOP Class UID.
    pub sop_class: &'a str,
    /// The AE title the instances were to go to: the Move Destination as it was named,
    /// or for a C-GET the AE that asked.
    pub destination: &'a str,
    /// Its host, when it is a destination this server sends to; none for a C-GET, whose
    /// destination is the peer itself.
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
    /// A C-MOVE or a C-GET is over.
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

    /// The capabilities of `called_ae_title`, when the settings give it some of its own.
    pub fn capabilities(&self, called_ae_title: &str) -> Option<&Capabilities> {
        self.config
            .capabilities
            .iter()
            .find(|c| c.ae_title.matches(called_ae_title))
    }

    /// What an association negotiates: the SOP classes of the services `capabilities`
    /// offers, in its transfer syntaxes; every service in every transfer syntax pacs-rs
    /// reads without.
    fn options(
        &self,
        capabilities: Option<&Capabilities>,
    ) -> ServerAssociationOptions<'static, Titles, Roles> {
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
            .with_negotiation(Roles);
        for service in Service::ALL {
            if capabilities.is_none_or(|c| c.allows(service)) {
                for uid in service.sop_classes() {
                    options = options.with_abstract_syntax(uid);
                }
            }
        }
        if let Some(syntaxes) = capabilities.and_then(|c| c.transfer_syntaxes.as_ref()) {
            for uid in syntaxes {
                options = options.with_transfer_syntax(uid.clone());
            }
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
        let options = Arc::new(Negotiated {
            all: self.options(None),
            of_titles: self
                .config
                .capabilities
                .iter()
                .map(|c| (c.ae_title.clone(), self.options(Some(c))))
                .collect(),
        });
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
                    pacs_core::metrics::add(&pacs_core::metrics::ASSOCIATIONS_REFUSED);
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
        options: &Negotiated,
        socket: TcpStream,
        peer: std::net::SocketAddr,
    ) {
        let refused = |e: dicom_ul::association::Error| {
            pacs_core::metrics::add(&pacs_core::metrics::ASSOCIATIONS_REFUSED);
            tracing::info!(%peer, error = %e, "association not established");
            if let Some(observer) = &self.observer {
                observer.refused(peer, &e.to_string());
            }
        };
        // With TLS configured the port speaks TLS only (PS3.15 B.12: strict TLS, no
        // upgrade from plain): a caller that does not open with a handshake the
        // configuration accepts is refused here, before any DICOM.
        if self.config.tls.is_some() {
            // the called AE title is inside the TLS session dicom-ul opens: every service is
            // negotiated, and an AE title's own capabilities hold per operation
            match options.all.establish_tls_async(socket).await {
                Ok(association) => self.established(association, peer).await,
                Err(e) => refused(e),
            }
        } else {
            let called = called_ae_title(&socket, self.config.timeout).await;
            let chosen = called
                .and_then(|called| {
                    options
                        .of_titles
                        .iter()
                        .find(|(title, _)| title.matches(&called))
                })
                .map_or(&options.all, |(_, options)| options);
            match chosen.establish_async(socket).await {
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
        let open = pacs_core::metrics::OpenAssociation::new(&peer.to_string(), &calling, &called);
        // what the archive takes in while the association lasts: its messages are served
        // on this task, one after the other
        let messages = self.messages(association, peer, &calling, &called, &destination, &open);
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

    /// Answer a C-MOVE or a C-GET and tell the observer. Whether the association goes on.
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
        let authorized = self.answers_queries_of(calling);
        let get = retrieve::is_get(request.abstract_syntax);
        let operation = if get { "C-GET" } else { "C-MOVE" };
        let after = if get {
            retrieve::c_get(
                association,
                request,
                command,
                identifier,
                destination,
                authorized,
            )
            .await
        } else {
            retrieve::c_move(
                association,
                request,
                command,
                identifier,
                destination,
                &self.config,
                authorized,
            )
            .await
        };
        let (retrieved, open) = match after {
            Ok(After::Open(retrieved)) => (retrieved, true),
            Ok(After::Released(retrieved) | After::Aborted(retrieved)) => (retrieved, false),
            Err(e) => {
                tracing::warn!(%peer, calling, operation, error = %e, "association aborted during a retrieval");
                let _ = association.abort().await;
                return false;
            }
        };
        let counts = &retrieved.counts;
        tracing::info!(%peer, calling, called = request.called, destination = %retrieved.destination, status = %retrieved.status, completed = counts.completed, failed = counts.failed, warning = counts.warning, remaining = counts.remaining, "{operation}");
        if let Some(observer) = &self.observer {
            let studies = sent_studies(&index, &retrieved.sent).await;
            observer.retrieved(
                &Peer {
                    address: peer,
                    calling: calling.to_owned(),
                    called: request.called.to_owned(),
                },
                &Retrieved {
                    operation,
                    sop_class: request.abstract_syntax,
                    destination: &retrieved.destination,
                    destination_host: retrieved.destination_host.as_deref(),
                    status: retrieved.status,
                    studies: &studies,
                },
            );
        }
        if !open {
            tracing::info!(%peer, calling, operation, "association ended by the peer during a retrieval");
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
        open: &pacs_core::metrics::OpenAssociation,
    ) where
        A: dicom_ul::association::AsyncAssociation<S> + Association + Send,
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        let mut served = Counts::default();
        loop {
            // what it did so far, for the administration page
            open.served(
                served.stored as u64,
                served.found as u64,
                served.retrieved as u64,
            );
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
                    let capabilities = self.capabilities(called);
                    let handled = handle(
                        &request,
                        &message.command,
                        message.data,
                        destination,
                        capabilities,
                    )
                    .await;
                    let response = match handled {
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
                        Handled::Retrieve(identifier) => {
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

/// The negotiations an association may get: every service, or an AE title's own.
struct Negotiated {
    all: ServerAssociationOptions<'static, Titles, Roles>,
    of_titles: Vec<(AeTitle, ServerAssociationOptions<'static, Titles, Roles>)>,
}

/// The Called AE Title of the A-ASSOCIATE-RQ waiting on `socket`, read without taking it
/// off the connection (PS3.8 9.3.2: PDU type 01H, then the PDU length, the protocol version
/// and two reserved bytes, then the 16 bytes of the title). `None` for anything else, which
/// the negotiation answers as it should.
async fn called_ae_title(socket: &TcpStream, wait: std::time::Duration) -> Option<String> {
    let mut header = [0u8; 26];
    let read = async {
        loop {
            match socket.peek(&mut header).await {
                Ok(n) if n >= header.len() => return true,
                // the connection closed, or failed
                Ok(0) | Err(_) => return false,
                // the rest of the header is on its way
                Ok(_) => tokio::time::sleep(std::time::Duration::from_millis(5)).await,
            }
        }
    };
    if !tokio::time::timeout(wait, read).await.unwrap_or(false) || header[0] != 0x01 {
        return None;
    }
    let title = String::from_utf8_lossy(&header[10..26]);
    Some(title.trim_matches([' ', '\0']).to_owned())
}

/// SCP/SCU Role Selection (PS3.7 D.3.3.4): for a Storage SOP class the requester may be
/// the SCP as well as, or instead of, the SCU, which is how it receives what a C-GET sends
/// (PS3.4 C.5.3). Whatever it proposes for one is accepted; for any other SOP class no item
/// is returned, and the default roles hold.
#[derive(Debug, Clone, Copy)]
pub struct Roles;

impl dicom_ul::association::server::Negotiation for Roles {
    fn negotiate_roles(
        &self,
        sop_class_uid: &str,
        scu_role: bool,
        scp_role: bool,
    ) -> Option<dicom_ul::pdu::RequestorRoles> {
        is_storage(sop_class_uid.trim_end_matches('\0')).then_some(dicom_ul::pdu::RequestorRoles {
            scu: scu_role,
            scp: scp_role,
        })
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
    /// A C-FIND-RQ with this Identifier, which the Query SCP answers with as many
    /// responses as it has matches.
    Find(DataSet),
    /// A C-MOVE-RQ or a C-GET-RQ with this Identifier, which the Retrieve SCP answers as
    /// its sub-operations go.
    Retrieve(DataSet),
    Abort(&'static str),
}

async fn handle(
    request: &Request<'_>,
    command: &Command,
    data: DataSet,
    destination: &Destination,
    capabilities: Option<&Capabilities>,
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
    // such SOP class, or the AE title called offers none of its service; Mistyped argument
    // when it is not the one agreed for this context.
    let affected = command.ui(tag::AFFECTED_SOP_CLASS_UID).unwrap_or("");
    let Some(service) = Service::of(affected) else {
        return respond(
            Status::SOP_CLASS_NOT_SUPPORTED,
            Some(&format!("SOP Class {affected} is not supported")),
        );
    };
    if let Some(capabilities) = capabilities {
        // negotiated already on a plain connection; over TLS every service is, and the AE
        // title's own capabilities hold here
        if !capabilities.allows(service) {
            return respond(
                Status::SOP_CLASS_NOT_SUPPORTED,
                Some(&format!(
                    "{} offers no {} service",
                    capabilities.ae_title,
                    service.name()
                )),
            );
        }
        if !capabilities.allows_transfer_syntax(request.transfer_syntax) {
            return respond(
                Status::SOP_CLASS_NOT_SUPPORTED,
                Some(&format!(
                    "{} does not take transfer syntax {}",
                    capabilities.ae_title, request.transfer_syntax
                )),
            );
        }
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
    // PS3.7 9.1.5.1.4, 9.1.1.1.9: an operation the SOP class does not have
    if field != service.operation() {
        return respond(Status::UNRECOGNIZED_OPERATION, None);
    }
    match service {
        Service::Verification => respond(Status::SUCCESS, None),
        Service::Storage => {
            let (status, comment) = store::store(request, command, data, destination).await;
            respond(status, comment.as_deref())
        }
        Service::Find => Handled::Find(data),
        Service::Move | Service::Get => Handled::Retrieve(data),
    }
}

pub(crate) fn is_storage(sop_class: &str) -> bool {
    crate::sop_classes::STORAGE
        .iter()
        .any(|(uid, _)| *uid == sop_class)
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
