//! pacs-rs as an association acceptor: Verification SCP (PS3.4 Annex A) and Storage SCP
//! (PS3.4 Annex B) for the SOP classes of [`crate::sop_classes::STORAGE`].
//!
//! Every accepted AE title shares one set of presentation contexts; once an association is
//! up, its called AE title picks where received instances go ([`Scp::destination`]).

use std::sync::Arc;

use bytes::Bytes;
use dicom_ul::association::Association;
use dicom_ul::association::server::AccessControl;
use dicom_ul::pdu::{
    AssociationRJ, AssociationRJResult, AssociationRJServiceProviderPresentationReason,
    AssociationRJServiceUserReason, AssociationRJSource, PresentationContextResultReason,
    UserIdentity,
};
use dicom_ul::{Pdu, ServerAssociationOptions};
use pacs_archive::{Archive, IngestError};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use crate::ae::AeTitle;
use crate::command::{self, Command, field, tag};
use crate::message::{self, DataSet, Incoming, MessageError};
use crate::status::Status;
use crate::{Config, IMPLEMENTATION_CLASS_UID, IMPLEMENTATION_VERSION_NAME, VERIFICATION};

/// Where the instances an association receives go.
#[derive(Debug, Clone)]
pub enum Destination {
    /// Into the archive, exactly as a REST or STOW-RS upload: compressed, stored, indexed;
    /// an instance sent again is kept as first stored unless overwriting is on.
    Archive(Archive),
    // Research mode (docs/interop.md, step 5) adds a staging destination here: instances
    // kept apart, never indexed or served, de-identified, and only then ingested.
}

impl Destination {
    async fn store(&self, file: Bytes) -> Result<(), IngestError> {
        match self {
            Destination::Archive(archive) => archive.ingest(file).await.map(|_| ()),
        }
    }
}

/// The DIMSE service provider: configuration, the destination of received instances, and
/// the cap on associations served at once.
#[derive(Debug)]
pub struct Scp {
    config: Config,
    destination: Destination,
    slots: Arc<Semaphore>,
}

impl Scp {
    pub fn new(config: Config, destination: Destination) -> Self {
        let slots = Arc::new(Semaphore::new(config.max_associations));
        Scp {
            config,
            destination,
            slots,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Where instances sent to `called_ae_title` go. Every accepted title goes to the same
    /// destination today; research and clinical mode will split them here.
    pub fn destination(&self, _called_ae_title: &str) -> &Destination {
        &self.destination
    }

    fn options(
        &self,
    ) -> ServerAssociationOptions<'static, Titles, dicom_ul::association::server::DefaultNegotiation>
    {
        let mut accepted = vec![self.config.ae_title.clone()];
        accepted.extend(self.config.extra_ae_titles.iter().cloned());
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

struct Request<'a> {
    context_id: u8,
    abstract_syntax: &'a str,
    transfer_syntax: &'a str,
    calling: &'a str,
    called: &'a str,
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
            let (status, comment) = store(request, command, data, destination).await;
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

/// A C-STORE: the data set and the request's UIDs checked against each other, then the
/// instance handed to `destination`, with the statuses of PS3.4 Table B.2-1.
async fn store(
    request: &Request<'_>,
    command: &Command,
    data: DataSet,
    destination: &Destination,
) -> (Status, Option<String>) {
    let data = match data {
        DataSet::Received(data) => data,
        DataSet::None => {
            return (
                Status::CANNOT_UNDERSTAND,
                Some("C-STORE-RQ without a data set".into()),
            );
        }
        DataSet::Dropped { size } => {
            return (
                Status::OUT_OF_RESOURCES_TOO_LARGE,
                Some(format!("data set of {size} bytes is over the limit")),
            );
        }
    };
    let sop_class = request.abstract_syntax;
    let sop_instance = command.ui(tag::AFFECTED_SOP_INSTANCE_UID).unwrap_or("");
    let file = part10(
        &data,
        sop_class,
        sop_instance,
        request.transfer_syntax,
        request.calling,
        request.called,
    );
    let (found_class, found_instance) = match identifiers(&file) {
        Ok(found) => found,
        Err(e) => return (Status::CANNOT_UNDERSTAND, Some(e)),
    };
    // PS3.4 B.2.3: the data set must be an instance of the SOP class it is sent as
    if found_class != sop_class {
        return (
            Status::DATA_SET_DOES_NOT_MATCH_SOP_CLASS,
            Some(format!("SOP Class UID is {found_class}, not {sop_class}")),
        );
    }
    if found_instance != sop_instance {
        return (
            Status::CANNOT_UNDERSTAND_INSTANCE_UID,
            Some(format!(
                "SOP Instance UID is {found_instance}, not {sop_instance}"
            )),
        );
    }
    match destination.store(Bytes::from(file)).await {
        // PS3.4 B.2.2: Success means the instance is stored. One sent again is: the copy
        // kept is the first (or the new one, with overwriting on), as for a REST upload.
        Ok(()) => (Status::SUCCESS, None),
        Err(IngestError::Invalid(e)) => (Status::CANNOT_UNDERSTAND, Some(e.to_string())),
        Err(e @ (IngestError::Conflict(_) | IngestError::WrongStudy { .. })) => {
            (Status::CANNOT_UNDERSTAND_CONFLICT, Some(e.to_string()))
        }
        Err(IngestError::Store(e)) => {
            tracing::error!(error = %e, sop_instance, "C-STORE: the store refused the file");
            (
                Status::OUT_OF_RESOURCES_STORE,
                Some("the store refused the file".into()),
            )
        }
        Err(IngestError::Index(e)) => {
            tracing::error!(error = %e, sop_instance, "C-STORE: the index refused the instance");
            (
                Status::OUT_OF_RESOURCES_INDEX,
                Some("the index refused the instance".into()),
            )
        }
    }
}

/// SOP Class UID and SOP Instance UID of a Part 10 file, read up to the pixel data.
fn identifiers(file: &[u8]) -> Result<(String, String), String> {
    use dicom_dictionary_std::tags;
    let object = dicom_object::OpenFileOptions::new()
        .read_preamble(dicom_object::file::ReadPreamble::Always)
        .read_until(tags::PIXEL_DATA)
        .from_reader(file)
        .map_err(|e| format!("cannot read the data set: {e}"))?;
    let read = |tag, name| {
        object
            .element(tag)
            .ok()
            .and_then(|e| e.to_str().ok())
            .map(|s| s.trim_end_matches(['\0', ' ']).to_owned())
            .ok_or_else(|| format!("the data set has no {name}"))
    };
    Ok((
        read(tags::SOP_CLASS_UID, "SOP Class UID")?,
        read(tags::SOP_INSTANCE_UID, "SOP Instance UID")?,
    ))
}

/// A DICOM file of a received data set: 128-byte preamble, "DICM", the File Meta
/// Information group in Explicit VR Little Endian (PS3.10 7.1, Table 7.1-1), then the
/// data set exactly as received, in the negotiated transfer syntax.
pub fn part10(
    data_set: &[u8],
    sop_class: &str,
    sop_instance: &str,
    transfer_syntax: &str,
    calling: &str,
    called: &str,
) -> Vec<u8> {
    let mut meta = Vec::new();
    // File Meta Information Version: OB, 00H 01H
    meta.extend_from_slice(&[
        0x02, 0x00, 0x01, 0x00, b'O', b'B', 0, 0, 2, 0, 0, 0, 0x00, 0x01,
    ]);
    short_element(&mut meta, 0x0002, b"UI", &ui(sop_class));
    short_element(&mut meta, 0x0003, b"UI", &ui(sop_instance));
    short_element(&mut meta, 0x0010, b"UI", &ui(transfer_syntax));
    short_element(&mut meta, 0x0012, b"UI", &ui(IMPLEMENTATION_CLASS_UID));
    short_element(&mut meta, 0x0013, b"SH", &text(IMPLEMENTATION_VERSION_NAME));
    // Source: the AE that wrote the file, pacs-rs under the title it was called by;
    // Sending and Receiving: the two ends of the association
    short_element(&mut meta, 0x0016, b"AE", &text(called));
    short_element(&mut meta, 0x0017, b"AE", &text(calling));
    short_element(&mut meta, 0x0018, b"AE", &text(called));
    let mut file = vec![0u8; 128];
    file.extend_from_slice(b"DICM");
    short_element(&mut file, 0x0000, b"UL", &(meta.len() as u32).to_le_bytes());
    file.extend_from_slice(&meta);
    file.extend_from_slice(data_set);
    file
}

fn short_element(out: &mut Vec<u8>, element: u16, vr: &[u8; 2], value: &[u8]) {
    out.extend_from_slice(&0x0002u16.to_le_bytes());
    out.extend_from_slice(&element.to_le_bytes());
    out.extend_from_slice(vr);
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.extend_from_slice(value);
}

fn ui(uid: &str) -> Vec<u8> {
    let mut value = uid.as_bytes().to_vec();
    if value.len() % 2 == 1 {
        value.push(0);
    }
    value
}

fn text(s: &str) -> Vec<u8> {
    let mut value = s.as_bytes().to_vec();
    if value.len() % 2 == 1 {
        value.push(b' ');
    }
    value
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
