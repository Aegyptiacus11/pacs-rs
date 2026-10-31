//! A pacs-rs SCP on a free loopback port over an in-memory archive, and a bare client that
//! sends whatever command it is given.
#![allow(dead_code)]

pub mod scripted;

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
