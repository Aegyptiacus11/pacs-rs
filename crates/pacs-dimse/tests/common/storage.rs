//! A Storage SCP for the tests of what pacs-rs sends: it accepts one association, records
//! each C-STORE-RQ, and answers as the test says.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::InMemDicomObject;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::ServerAssociationOptions;
use dicom_ul::association::Association;
use pacs_dimse::command::Command;
use pacs_dimse::message::{self, DataSet, Incoming};
use pacs_dimse::{RemoteAe, Status};

/// One C-STORE-RQ a peer received.
pub struct Received {
    pub command: Command,
    /// The AE title that sent it: the calling one of its association.
    pub calling: String,
    pub transfer_syntax: String,
    pub data_set: Vec<u8>,
}

impl Received {
    pub fn object(&self) -> InMemDicomObject {
        let ts = TransferSyntaxRegistry.get(&self.transfer_syntax).unwrap();
        InMemDicomObject::read_dataset_with_ts(&*self.data_set, ts).unwrap()
    }
}

/// What a peer does with the instances it is sent.
#[derive(Clone)]
pub enum Peer {
    /// Answer each with the next of these statuses, Success once they are used up.
    Answers(&'static [u16]),
    /// Abort when the first one comes.
    Aborts,
    /// Answer this many with Success at once, then each further one only after the
    /// `Notify` has been notified: for a test to act while a sub-operation is under way.
    Holds(usize, Arc<tokio::sync::Notify>),
}

/// A Storage SCP called STORESCP that accepts one association, for `sop_classes` in
/// `transfer_syntaxes`, and records each C-STORE-RQ until the association ends.
pub async fn storage_scp(
    sop_classes: &'static [&'static str],
    transfer_syntaxes: &'static [&'static str],
    peer: Peer,
) -> (RemoteAe, Arc<Mutex<Vec<Received>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = Arc::new(Mutex::new(Vec::new()));
    let record = received.clone();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut options = ServerAssociationOptions::new()
            .ae_title("STORESCP")
            .read_timeout(Duration::from_secs(10));
        for sop_class in sop_classes {
            options = options.with_abstract_syntax(*sop_class);
        }
        for ts in transfer_syntaxes {
            options = options.with_transfer_syntax(*ts);
        }
        let Ok(mut association) = options.establish_async(socket).await else {
            return;
        };
        let calling = association.peer_ae_title().trim().to_owned();
        let mut statuses = match &peer {
            Peer::Answers(statuses) => statuses.iter(),
            _ => [].iter(),
        };
        loop {
            let message = match message::receive(&mut association, 1 << 24).await {
                Ok(Incoming::Message(message)) => message,
                _ => return,
            };
            if matches!(peer, Peer::Aborts) {
                let _ = association.abort().await;
                return;
            }
            let transfer_syntax = association
                .presentation_contexts()
                .iter()
                .find(|pc| pc.id == message.context_id)
                .unwrap()
                .transfer_syntax
                .clone();
            let DataSet::Received(data_set) = message.data else {
                panic!("a C-STORE-RQ without a data set");
            };
            let status = statuses.next().copied().unwrap_or(0x0000);
            let response = pacs_dimse::command::response(&message.command, Status(status), None);
            let so_far = {
                let mut received = record.lock().unwrap();
                received.push(Received {
                    command: message.command,
                    calling: calling.clone(),
                    transfer_syntax,
                    data_set,
                });
                received.len()
            };
            if let Peer::Holds(free, go) = &peer
                && so_far > *free
            {
                go.notified().await;
            }
            if message::send(&mut association, message.context_id, &response, None)
                .await
                .is_err()
            {
                return;
            }
        }
    });
    let remote = format!("peer=STORESCP@127.0.0.1:{port}").parse().unwrap();
    (remote, received)
}
