//! A scripted SCP for the SCU tests: it accepts one association for one abstract syntax,
//! reads the request, then does what the test's script says, and records what it saw.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::InMemDicomObject;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::ServerAssociationOptions;
use pacs_dimse::RemoteAe;
use pacs_dimse::command::{Command, NO_DATA_SET, tag};
use pacs_dimse::message::{self, DataSet, Incoming};

/// One thing the SCP does after the request.
pub enum Act {
    /// Send this command, with this Identifier as its data set if any; Command Data Set
    /// Type is set to match.
    Send(Command, Option<InMemDicomObject>),
    /// Send this command with these bytes as its data set, whatever they say.
    SendRaw(Command, Vec<u8>),
    /// Read the next message, the C-CANCEL-RQ, and record it.
    ExpectCancel,
    /// Say nothing more.
    Silence,
}

/// What the SCP saw.
#[derive(Default)]
pub struct Seen {
    pub request: Option<Command>,
    pub identifier: Option<InMemDicomObject>,
    pub transfer_syntax: String,
    pub cancel: Option<Command>,
    /// How the association ended: "release" or "abort".
    pub end: Option<&'static str>,
}

/// Serve one association on a free port, called `ae_title`, for `abstract_syntax` in
/// `transfer_syntaxes`; `script` turns the request's Message ID into what to do.
pub async fn serve<S>(
    ae_title: &'static str,
    abstract_syntax: &'static str,
    transfer_syntaxes: &'static [&'static str],
    script: S,
) -> (RemoteAe, Arc<Mutex<Seen>>)
where
    S: FnOnce(u16) -> Vec<Act> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let record = seen.clone();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut options = ServerAssociationOptions::new()
            .ae_title(ae_title)
            .with_abstract_syntax(abstract_syntax)
            .read_timeout(Duration::from_secs(5));
        for ts in transfer_syntaxes {
            options = options.with_transfer_syntax(*ts);
        }
        let mut association = options.establish_async(socket).await.unwrap();
        let context = &association.presentation_contexts()[0];
        let (context_id, ts_uid) = (context.id, context.transfer_syntax.clone());
        let ts = TransferSyntaxRegistry.get(&ts_uid).unwrap();
        let Incoming::Message(rq) = message::receive(&mut association, 1 << 20).await.unwrap()
        else {
            panic!("no request");
        };
        let DataSet::Received(bytes) = rq.data else {
            panic!("no Identifier")
        };
        let message_id = rq.command.message_id().unwrap();
        {
            let mut seen = record.lock().unwrap();
            seen.identifier = Some(InMemDicomObject::read_dataset_with_ts(&bytes[..], ts).unwrap());
            seen.request = Some(rq.command);
            seen.transfer_syntax = ts_uid.clone();
        }
        for act in script(message_id) {
            match act {
                Act::Send(command, identifier) => {
                    let data = identifier.map(|i| {
                        let mut out = Vec::new();
                        i.write_dataset_with_ts(&mut out, ts).unwrap();
                        out
                    });
                    let command = command.with_us(
                        tag::COMMAND_DATA_SET_TYPE,
                        if data.is_some() { 0x0102 } else { NO_DATA_SET },
                    );
                    message::send(&mut association, context_id, &command, data.as_deref())
                        .await
                        .unwrap();
                }
                Act::SendRaw(command, data) => {
                    let command = command.with_us(tag::COMMAND_DATA_SET_TYPE, 0x0102);
                    message::send(&mut association, context_id, &command, Some(&data))
                        .await
                        .unwrap();
                }
                Act::ExpectCancel => {
                    let Incoming::Message(m) = message::receive(&mut association, 0).await.unwrap()
                    else {
                        panic!("no C-CANCEL")
                    };
                    record.lock().unwrap().cancel = Some(m.command);
                }
                Act::Silence => {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    return;
                }
            }
        }
        let end = match message::receive(&mut association, 0).await {
            Ok(Incoming::Released) => "release",
            _ => "abort",
        };
        record.lock().unwrap().end = Some(end);
    });
    let remote = format!("scp={ae_title}@127.0.0.1:{port}").parse().unwrap();
    (remote, seen)
}

/// A response: Command Field, Message ID Being Responded To and Status, for `sop_class`.
pub fn response(sop_class: &str, command_field: u16, message_id: u16, status: u16) -> Command {
    Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, sop_class)
        .with_us(tag::COMMAND_FIELD, command_field)
        .with_us(tag::MESSAGE_ID_BEING_RESPONDED_TO, message_id)
        .with_us(tag::STATUS, status)
}

/// Wait for the SCP to record how the association ended.
pub async fn ended(seen: &Arc<Mutex<Seen>>) -> Option<&'static str> {
    for _ in 0..50 {
        if let Some(end) = seen.lock().unwrap().end {
            return Some(end);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    None
}
