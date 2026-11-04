//! pacs-rs as an association requestor: the association every operation opens, here, and
//! the operations, one module each: [`echo`] (C-ECHO), [`find`] (C-FIND), [`retrieve`]
//! (C-MOVE).

pub mod echo;
pub mod find;
pub mod retrieve;

use std::time::Duration;

use dicom_encoding::TransferSyntax;
use dicom_object::InMemDicomObject;
use dicom_ul::ClientAssociationOptions;
use dicom_ul::association::AsyncClientAssociation;
use dicom_ul::pdu::PresentationContextResultReason;
use tokio::net::TcpStream;

use crate::ae::{AeTitle, RemoteAe};

/// The most memory a remote AE's response Identifier may take to read, by estimate
/// ([`pacs_core::wire::read_cost`]): a few hundred thousand values or tens of thousands of elements,
/// far beyond what an Identifier holds.
const MAX_IDENTIFIER_COST: u64 = 64 << 20;

/// A response Identifier from a remote AE, which is not trusted to tell the truth about lengths.
fn read_identifier(bytes: &[u8], ts: &TransferSyntax) -> Result<InMemDicomObject, String> {
    let checked = pacs_core::wire::checked_data_set(bytes, ts.uid(), MAX_IDENTIFIER_COST)
        .map_err(|e| e.to_string())?;
    InMemDicomObject::read_dataset_with_ts(&*checked, ts).map_err(|e| e.to_string())
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("no association with {ae}@{host}:{port}: {source}")]
    Association {
        ae: String,
        host: String,
        port: u16,
        source: Box<dicom_ul::association::Error>,
    },
    #[error("the remote AE did not accept {0}")]
    NotAccepted(&'static str),
}

/// An association with one presentation context accepted.
pub(crate) struct Connected {
    pub association: AsyncClientAssociation<TcpStream>,
    pub context: u8,
    pub transfer_syntax: String,
}

/// Associate with `remote` as `calling`, proposing `abstract_syntax` with
/// `transfer_syntaxes` in one presentation context; `timeout` bounds the connection and each
/// write, `response_timeout` every later wait for a PDU.
pub(crate) async fn connect(
    remote: &RemoteAe,
    calling: &AeTitle,
    abstract_syntax: &'static str,
    transfer_syntaxes: &[&str],
    timeout: Duration,
    response_timeout: Duration,
) -> Result<Connected, ConnectError> {
    let association = ClientAssociationOptions::new()
        .calling_ae_title(calling.as_str().to_owned())
        .called_ae_title(remote.ae_title.as_str().to_owned())
        .with_presentation_context(abstract_syntax, transfer_syntaxes.to_vec())
        .connection_timeout(timeout)
        .read_timeout(response_timeout)
        .write_timeout(timeout)
        .establish_async((remote.host.as_str(), remote.port))
        .await
        .map_err(|source| ConnectError::Association {
            ae: remote.ae_title.to_string(),
            host: remote.host.clone(),
            port: remote.port,
            source: Box::new(source),
        })?;
    let Some((context, transfer_syntax)) = association
        .presentation_contexts()
        .iter()
        .find(|pc| pc.reason == PresentationContextResultReason::Acceptance)
        .map(|pc| (pc.id, pc.transfer_syntax.trim_end_matches('\0').to_owned()))
    else {
        let _ = association.abort().await;
        return Err(ConnectError::NotAccepted(abstract_syntax));
    };
    Ok(Connected {
        association,
        context,
        transfer_syntax,
    })
}
