//! A pacs-rs SCP on a free loopback port over an in-memory archive, and a bare client that
//! sends whatever command it is given.
#![allow(dead_code)]

pub mod scripted;
pub mod storage;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use dicom_ul::ClientAssociationOptions;
use dicom_ul::association::AsyncClientAssociation;
use dicom_ul::pdu::PresentationContextResultReason;
use pacs_archive::{Archive, Compression};
use pacs_dimse::command::{Command, NO_DATA_SET, PRIORITY_MEDIUM, field, tag};
use pacs_dimse::message::{self, DataSet, Incoming};
use pacs_dimse::{AeTitle, Config, Destination, Scp};
use pacs_index::Index;
use pacs_store::Store;
use tokio::net::{TcpListener, TcpStream};

pub const CT: &str = "1.2.840.10008.5.1.4.1.1.2";
pub const MR: &str = "1.2.840.10008.5.1.4.1.1.4";
pub const EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";
pub const IMPLICIT_VR_LE: &str = "1.2.840.10008.1.2";

pub async fn archive() -> Archive {
    Archive::new(Store::memory(), Index::in_memory().await.unwrap())
        .with_compression(Compression::Off)
}

pub fn config() -> Config {
    let mut config = Config::new(AeTitle::new("PACSRS").unwrap());
    config.timeout = Duration::from_secs(5);
    config
}

/// Serve `config` and `archive` on 127.0.0.1, on a port of the system's choosing.
pub async fn start(config: Config, archive: Archive) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let scp = Arc::new(Scp::new(config, Destination::Archive(archive)));
    tokio::spawn(scp.serve(listener, std::future::pending()));
    addr
}

/// Serve `scp` on 127.0.0.1, on a port of the system's choosing.
pub async fn start_scp(scp: Scp) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(Arc::new(scp).serve(listener, std::future::pending()));
    addr
}

/// The data set of a Part 10 file: what follows the preamble, "DICM" and group 0002.
pub fn data_set(file: &[u8]) -> Vec<u8> {
    assert_eq!(&file[128..132], b"DICM");
    let length = u32::from_le_bytes(file[140..144].try_into().unwrap()) as usize;
    file[144 + length..].to_vec()
}

pub struct Client {
    pub association: AsyncClientAssociation<TcpStream>,
    next_id: u16,
}

impl Client {
    /// Propose each abstract syntax with `transfer_syntaxes`.
    pub async fn connect(
        addr: SocketAddr,
        calling: &str,
        called: &str,
        abstract_syntaxes: &[&str],
        transfer_syntaxes: &[&str],
    ) -> Result<Client, dicom_ul::association::Error> {
        let mut options = ClientAssociationOptions::new()
            .calling_ae_title(calling.to_owned())
            .called_ae_title(called.to_owned())
            .read_timeout(Duration::from_secs(10));
        for syntax in abstract_syntaxes {
            options = options.with_presentation_context(
                syntax.to_string(),
                transfer_syntaxes.iter().map(|t| t.to_string()).collect(),
            );
        }
        let association = options.establish_async(addr).await?;
        Ok(Client {
            association,
            next_id: 1,
        })
    }

    pub fn context(&self, abstract_syntax: &str) -> u8 {
        self.association
            .presentation_contexts()
            .iter()
            .find(|pc| {
                pc.abstract_syntax == abstract_syntax
                    && pc.reason == PresentationContextResultReason::Acceptance
            })
            .unwrap_or_else(|| panic!("{abstract_syntax} was not accepted"))
            .id
    }

    pub fn message_id(&mut self) -> u16 {
        self.next_id += 1;
        self.next_id
    }

    /// Send `command` (and `data`) on presentation context `context`; the next thing the
    /// SCP sends back.
    pub async fn exchange(
        &mut self,
        context: u8,
        command: &Command,
        data: Option<&[u8]>,
    ) -> Incoming {
        message::send(&mut self.association, context, command, data)
            .await
            .unwrap();
        message::receive(&mut self.association, 1 << 20)
            .await
            .unwrap()
    }

    /// The response to `command`, which must be one.
    pub async fn response(
        &mut self,
        context: u8,
        command: &Command,
        data: Option<&[u8]>,
    ) -> Command {
        match self.exchange(context, command, data).await {
            Incoming::Message(m) => {
                assert_eq!(m.data, DataSet::None, "a response carries no data set here");
                m.command
            }
            other => panic!("expected a response, got {other:?}"),
        }
    }

    pub async fn echo(&mut self) -> Command {
        let id = self.message_id();
        let context = self.context(pacs_dimse::VERIFICATION);
        self.response(context, &pacs_dimse::command::c_echo_rq(id), None)
            .await
    }

    /// A C-STORE-RQ of `file`'s data set, as `sop_class` and `sop_instance`, on the context
    /// of `context_class`.
    pub async fn store_as(
        &mut self,
        context_class: &str,
        sop_class: &str,
        sop_instance: &str,
        data: &[u8],
    ) -> Command {
        let id = self.message_id();
        let context = self.context(context_class);
        self.response(
            context,
            &c_store_rq(id, sop_class, sop_instance),
            Some(data),
        )
        .await
    }

    pub async fn release(self) {
        self.association.release().await.unwrap();
    }
}

pub fn c_store_rq(message_id: u16, sop_class: &str, sop_instance: &str) -> Command {
    Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, sop_class)
        .with_us(tag::COMMAND_FIELD, field::C_STORE_RQ)
        .with_us(tag::MESSAGE_ID, message_id)
        .with_us(tag::PRIORITY, PRIORITY_MEDIUM)
        .with_us(tag::COMMAND_DATA_SET_TYPE, 0x0000)
        .with_ui(tag::AFFECTED_SOP_INSTANCE_UID, sop_instance)
}

pub fn no_data(command: Command) -> Command {
    command.with_us(tag::COMMAND_DATA_SET_TYPE, NO_DATA_SET)
}

/// How a C-FIND went: the Identifier and status of each Pending response, and the final
/// response.
pub struct Answer {
    pub matches: Vec<dicom_object::InMemDicomObject>,
    pub pending: Vec<pacs_dimse::Status>,
    pub last: Command,
}

impl Answer {
    pub fn status(&self) -> pacs_dimse::Status {
        self.last.status().unwrap()
    }

    /// The value of `tag` in each match, in order; `None` where a match lacks it.
    pub fn values(&self, tag: dicom_core::Tag) -> Vec<Option<String>> {
        self.matches.iter().map(|found| value(found, tag)).collect()
    }
}

/// An element's value as text, unpadded; `None` when the object lacks it.
pub fn value(object: &dicom_object::InMemDicomObject, tag: dicom_core::Tag) -> Option<String> {
    let element = object.get(tag)?;
    Some(
        element
            .to_str()
            .unwrap()
            .trim_end_matches([' ', '\0'])
            .to_owned(),
    )
}

/// An Identifier of `level` with these keys, each a tag, a VR and its value.
pub fn identifier(
    level: &str,
    keys: &[(dicom_core::Tag, dicom_core::VR, &str)],
) -> dicom_object::InMemDicomObject {
    use dicom_core::{DataElement, PrimitiveValue, VR};
    let mut identifier = dicom_object::InMemDicomObject::new_empty();
    identifier.put(DataElement::new(
        dicom_dictionary_std::tags::QUERY_RETRIEVE_LEVEL,
        VR::CS,
        PrimitiveValue::from(level),
    ));
    for (tag, vr, value) in keys {
        identifier.put(DataElement::new(*tag, *vr, PrimitiveValue::from(*value)));
    }
    identifier
}

pub fn c_find_rq(message_id: u16, sop_class: &str) -> Command {
    Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, sop_class)
        .with_us(tag::COMMAND_FIELD, field::C_FIND_RQ)
        .with_us(tag::MESSAGE_ID, message_id)
        .with_us(tag::PRIORITY, PRIORITY_MEDIUM)
        .with_us(tag::COMMAND_DATA_SET_TYPE, 0x0102)
}

impl Client {
    /// The transfer syntax agreed for `abstract_syntax`.
    pub fn transfer_syntax(
        &self,
        abstract_syntax: &str,
    ) -> &'static dicom_encoding::TransferSyntax {
        use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
        let uid = self
            .association
            .presentation_contexts()
            .iter()
            .find(|pc| pc.abstract_syntax == abstract_syntax)
            .unwrap()
            .transfer_syntax
            .clone();
        dicom_transfer_syntax_registry::TransferSyntaxRegistry
            .get(uid.trim_end_matches('\0'))
            .unwrap()
    }

    /// The responses to a request already sent, to the final one.
    pub async fn answer(&mut self, sop_class: &str) -> Answer {
        let ts = self.transfer_syntax(sop_class);
        let mut answer = Answer {
            matches: Vec::new(),
            pending: Vec::new(),
            last: Command::new(),
        };
        loop {
            let message = match message::receive(&mut self.association, 1 << 20)
                .await
                .unwrap()
            {
                Incoming::Message(message) => message,
                other => panic!("expected a response, got {other:?}"),
            };
            let status = message.command.status().unwrap();
            match message.data {
                DataSet::Received(bytes) => {
                    assert_eq!(
                        status.class(),
                        pacs_dimse::StatusClass::Pending,
                        "only a Pending response has an Identifier"
                    );
                    answer.pending.push(status);
                    answer.matches.push(
                        dicom_object::InMemDicomObject::read_dataset_with_ts(&*bytes, ts).unwrap(),
                    );
                }
                DataSet::None => {
                    assert_ne!(status.class(), pacs_dimse::StatusClass::Pending);
                    answer.last = message.command;
                    return answer;
                }
                DataSet::Dropped { size } => panic!("an Identifier of {size} bytes"),
            }
        }
    }

    /// A C-FIND of `identifier` on `sop_class`, to its final response.
    pub async fn find(
        &mut self,
        sop_class: &str,
        identifier: &dicom_object::InMemDicomObject,
    ) -> Answer {
        let mut bytes = Vec::new();
        identifier
            .write_dataset_with_ts(&mut bytes, self.transfer_syntax(sop_class))
            .unwrap();
        self.find_bytes(sop_class, &bytes).await
    }

    /// A C-FIND whose Identifier is these bytes, whatever they say.
    pub async fn find_bytes(&mut self, sop_class: &str, identifier: &[u8]) -> Answer {
        let id = self.message_id();
        let context = self.context(sop_class);
        message::send(
            &mut self.association,
            context,
            &c_find_rq(id, sop_class),
            Some(identifier),
        )
        .await
        .unwrap();
        let answer = self.answer(sop_class).await;
        assert_eq!(answer.last.command_field(), Ok(field::C_FIND_RSP));
        assert_eq!(answer.last.us(tag::MESSAGE_ID_BEING_RESPONDED_TO), Ok(id));
        answer
    }
}

/// Store every slice of `series` in `archive`, as an upload would.
pub async fn ingest(archive: &Archive, series: &pacs_core::synthetic::SyntheticSeries) {
    for bytes in series.all_bytes() {
        archive.ingest(bytes.into()).await.unwrap();
    }
}

/// Between a client and the server, to hold the server in the middle of an answer: what
/// the server sends goes through up to `first` bytes, then waits until the client has
/// sent its command number `open_at` (2: a cancel, or a second request, after the first
/// request). What the client sends always goes through. The socket to the server takes little in, so a server that
/// is not read from soon cannot write, however fast it is.
pub async fn gate(server: SocketAddr, first: usize, open_at: usize) -> SocketAddr {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (client, _) = listener.accept().await.unwrap();
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.set_recv_buffer_size(4096).unwrap();
        let upstream = socket.connect(server).await.unwrap();
        let (mut from_client, mut to_client) = client.into_split();
        let (mut from_server, mut to_server) = upstream.into_split();
        // a permit waits for whoever asks later: the command may come before the wait
        let open = Arc::new(tokio::sync::Notify::new());
        let command_seen = open.clone();
        tokio::spawn(async move {
            let mut commands = 0;
            loop {
                // a PDU: its type, a reserved byte, its length, its body (PS3.8 9.3.1)
                let mut header = [0u8; 6];
                if from_client.read_exact(&mut header).await.is_err() {
                    let _ = to_server.shutdown().await;
                    return;
                }
                let length = u32::from_be_bytes([header[2], header[3], header[4], header[5]]);
                let mut body = vec![0u8; length as usize];
                if from_client.read_exact(&mut body).await.is_err()
                    || to_server.write_all(&header).await.is_err()
                    || to_server.write_all(&body).await.is_err()
                {
                    return;
                }
                // P-DATA-TF: values of a length, a context and a header whose first bit
                // says command and second says last fragment (PS3.8 9.3.5, E.2)
                let mut values = &body[..];
                while header[0] == 0x04 && values.len() >= 6 {
                    let length =
                        u32::from_be_bytes([values[0], values[1], values[2], values[3]]) as usize;
                    if values[5] & 0b11 == 0b11 {
                        commands += 1;
                        if commands == open_at {
                            command_seen.notify_one();
                        }
                    }
                    values = &values[4 + length..];
                }
            }
        });
        let mut passed = 0;
        let mut buffer = vec![0u8; 2048];
        while passed < first {
            let room = buffer.len().min(first - passed);
            let n = from_server.read(&mut buffer[..room]).await.unwrap();
            if n == 0 {
                return;
            }
            to_client.write_all(&buffer[..n]).await.unwrap();
            passed += n;
        }
        open.notified().await;
        // from here on, everything
        let _ = tokio::io::copy(&mut from_server, &mut to_client).await;
    });
    addr
}

/// How a retrieval went: its Pending responses, the final one, and the SOP Instance UIDs
/// of the Failed SOP Instance UID List when the final response has one.
pub struct Retrieval {
    pub pending: Vec<Command>,
    pub last: Command,
    pub failed: Vec<String>,
}

impl Retrieval {
    pub fn status(&self) -> pacs_dimse::Status {
        self.last.status().unwrap()
    }

    /// The Remaining, Completed, Failed and Warning counts of a response; `None` for one
    /// it does not have.
    pub fn counts(response: &Command) -> [Option<u16>; 4] {
        [
            tag::NUMBER_OF_REMAINING_SUB_OPERATIONS,
            tag::NUMBER_OF_COMPLETED_SUB_OPERATIONS,
            tag::NUMBER_OF_FAILED_SUB_OPERATIONS,
            tag::NUMBER_OF_WARNING_SUB_OPERATIONS,
        ]
        .map(|count| response.us(count).ok())
    }
}

pub fn c_move_rq(message_id: u16, sop_class: &str, destination: &str) -> Command {
    Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, sop_class)
        .with_us(tag::COMMAND_FIELD, field::C_MOVE_RQ)
        .with_us(tag::MESSAGE_ID, message_id)
        .with_text(tag::MOVE_DESTINATION, destination)
        .with_us(tag::PRIORITY, PRIORITY_MEDIUM)
        .with_us(tag::COMMAND_DATA_SET_TYPE, 0x0102)
}

impl Client {
    /// The responses to a retrieval already sent, to the final one. A Pending response
    /// has no data set; the final one may have the list of what failed.
    pub async fn retrieval(&mut self, sop_class: &str) -> Retrieval {
        let ts = self.transfer_syntax(sop_class);
        let mut pending = Vec::new();
        loop {
            let message = match message::receive(&mut self.association, 1 << 20)
                .await
                .unwrap()
            {
                Incoming::Message(message) => message,
                other => panic!("expected a response, got {other:?}"),
            };
            let status = message.command.status().unwrap();
            if status.class() == pacs_dimse::StatusClass::Pending {
                assert_eq!(
                    message.data,
                    DataSet::None,
                    "a Pending response has no data set"
                );
                pending.push(message.command);
                continue;
            }
            let failed = match message.data {
                DataSet::None => Vec::new(),
                DataSet::Received(bytes) => {
                    let identifier =
                        dicom_object::InMemDicomObject::read_dataset_with_ts(&*bytes, ts).unwrap();
                    // nothing but the list (PS3.4 C.4.2.1.4.2)
                    assert_eq!(identifier.iter().count(), 1);
                    identifier
                        .get(dicom_dictionary_std::tags::FAILED_SOP_INSTANCE_UID_LIST)
                        .unwrap()
                        .to_multi_str()
                        .unwrap()
                        .iter()
                        .map(|uid| uid.trim_end_matches(['\0', ' ']).to_owned())
                        .collect()
                }
                DataSet::Dropped { size } => panic!("an Identifier of {size} bytes"),
            };
            return Retrieval {
                pending,
                last: message.command,
                failed,
            };
        }
    }

    /// A C-MOVE of `identifier` to `destination` on `sop_class`, to its final response.
    pub async fn c_move(
        &mut self,
        sop_class: &str,
        destination: &str,
        identifier: &dicom_object::InMemDicomObject,
    ) -> Retrieval {
        let mut bytes = Vec::new();
        identifier
            .write_dataset_with_ts(&mut bytes, self.transfer_syntax(sop_class))
            .unwrap();
        let id = self.message_id();
        let context = self.context(sop_class);
        message::send(
            &mut self.association,
            context,
            &c_move_rq(id, sop_class, destination),
            Some(&bytes),
        )
        .await
        .unwrap();
        let retrieval = self.retrieval(sop_class).await;
        assert_eq!(retrieval.last.command_field(), Ok(field::C_MOVE_RSP));
        assert_eq!(
            retrieval.last.us(tag::MESSAGE_ID_BEING_RESPONDED_TO),
            Ok(id)
        );
        retrieval
    }
}
