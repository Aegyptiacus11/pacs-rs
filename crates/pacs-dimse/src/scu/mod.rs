//! pacs-rs as an association requestor: the association every operation opens, here, and
//! the operations, one module each: [`echo`] (C-ECHO), [`find`] (C-FIND), [`retrieve`]
//! (C-MOVE), [`get`] (C-GET), [`store`] (C-STORE).

pub mod echo;
pub mod find;
pub mod get;
pub mod retrieve;
pub mod store;

use std::time::Duration;

use dicom_encoding::TransferSyntax;
use dicom_object::InMemDicomObject;
use dicom_ul::ClientAssociationOptions;
use dicom_ul::association::AsyncClientAssociation;
use dicom_ul::association::client::AsyncTlsStream;
use dicom_ul::pdu::PresentationContextResultReason;
use tokio::net::TcpStream;

use crate::ae::{AeTitle, ClientTls, RemoteAe};
use crate::command::Command;
use crate::message::{self, Incoming, MessageError};

/// The most memory a remote AE's response Identifier may take to read, by estimate
/// ([`pacs_core::wire::read_cost`]): a few hundred thousand values or tens of thousands of elements,
/// far beyond what an Identifier holds.
const MAX_IDENTIFIER_COST: u64 = 64 << 20;

/// An Identifier from a remote AE, which is not trusted to tell the truth about lengths.
pub(crate) fn read_identifier(
    bytes: &[u8],
    ts: &TransferSyntax,
) -> Result<InMemDicomObject, String> {
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
    #[error("{0} is reached over TLS, and pacs-rs has no TLS configuration for DIMSE")]
    NoTls(String),
}

/// An association pacs-rs opened, over TCP or over TLS: the same messages either way.
pub(crate) enum Link {
    Plain(Box<AsyncClientAssociation<TcpStream>>),
    Tls(Box<AsyncClientAssociation<AsyncTlsStream>>),
}

impl Link {
    pub async fn send(
        &mut self,
        context: u8,
        command: &Command,
        data: Option<&[u8]>,
    ) -> Result<(), MessageError> {
        match self {
            Link::Plain(association) => {
                message::send(association.as_mut(), context, command, data).await
            }
            Link::Tls(association) => {
                message::send(association.as_mut(), context, command, data).await
            }
        }
    }

    pub async fn receive(&mut self, max_data: usize) -> Result<Incoming, MessageError> {
        match self {
            Link::Plain(association) => message::receive(association.as_mut(), max_data).await,
            Link::Tls(association) => message::receive(association.as_mut(), max_data).await,
        }
    }

    /// The address of the peer at the other end of the connection.
    pub fn peer_address(&mut self) -> Option<std::net::SocketAddr> {
        match self {
            Link::Plain(association) => association.inner_stream().peer_addr().ok(),
            Link::Tls(association) => association.inner_stream().get_ref().0.peer_addr().ok(),
        }
    }

    /// Release the association. A peer that is gone already is not an error to report:
    /// what the operation found out, it found out before.
    pub async fn release(self) {
        let _ = match self {
            Link::Plain(association) => association.release().await,
            Link::Tls(association) => association.release().await,
        };
    }

    /// Abort the association, as for [`Link::release`].
    pub async fn abort(self) {
        let _ = match self {
            Link::Plain(association) => association.abort().await,
            Link::Tls(association) => association.abort().await,
        };
    }
}

/// An association with one presentation context accepted.
pub(crate) struct Connected {
    pub association: Link,
    pub context: u8,
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
    let context = (
        abstract_syntax.to_owned(),
        transfer_syntaxes
            .iter()
            .map(|ts| (*ts).to_owned())
            .collect(),
    );
    let (association, accepted) =
        associate(remote, calling, &[context], timeout, response_timeout).await?;
    let Some(context) = accepted.into_iter().next() else {
        association.abort().await;
        return Err(ConnectError::NotAccepted(abstract_syntax));
    };
    Ok(Connected {
        association,
        context: context.id,
    })
}

/// Associate with `remote` as `calling`, proposing each of `contexts`, an abstract syntax
/// with its transfer syntaxes: the association and the contexts the peer accepted, which
/// may be none. Timeouts as for [`connect`].
pub(crate) async fn associate(
    remote: &RemoteAe,
    calling: &AeTitle,
    contexts: &[(String, Vec<String>)],
    timeout: Duration,
    response_timeout: Duration,
) -> Result<(Link, Vec<store::Accepted>), ConnectError> {
    associate_with_roles(remote, calling, contexts, &[], timeout, response_timeout).await
}

/// [`associate`], proposing for each SOP class of `scp_roles` that pacs-rs takes the SCP
/// role and not the SCU one (SCP/SCU Role Selection, PS3.7 D.3.3.4): how a C-GET's
/// requester receives the instances it asked for (PS3.4 C.5.3).
pub(crate) async fn associate_with_roles(
    remote: &RemoteAe,
    calling: &AeTitle,
    contexts: &[(String, Vec<String>)],
    scp_roles: &[String],
    timeout: Duration,
    response_timeout: Duration,
) -> Result<(Link, Vec<store::Accepted>), ConnectError> {
    let mut options = ClientAssociationOptions::new()
        .calling_ae_title(calling.as_str().to_owned())
        .called_ae_title(remote.ae_title.as_str().to_owned())
        .connection_timeout(timeout)
        .read_timeout(response_timeout)
        .write_timeout(timeout);
    for (abstract_syntax, transfer_syntaxes) in contexts {
        options =
            options.with_presentation_context(abstract_syntax.clone(), transfer_syntaxes.clone());
    }
    for sop_class in scp_roles {
        options = options.with_role_selection(sop_class.clone(), false, true);
    }
    let refused = |source| ConnectError::Association {
        ae: remote.ae_title.to_string(),
        host: remote.host.clone(),
        port: remote.port,
        source: Box::new(source),
    };
    let address = (remote.host.as_str(), remote.port);
    let (association, negotiated) = if remote.secure {
        let Some(ClientTls(config)) = &remote.tls else {
            return Err(ConnectError::NoTls(remote.id.clone()));
        };
        // the certificate must be good for the host the AE is configured under
        let mut association = options
            .tls_config(config.clone())
            .server_name(&remote.host)
            .establish_tls_async(address)
            .await
            .map_err(refused)?;
        crate::without_delay(association.inner_stream().get_ref().0);
        let contexts = association.presentation_contexts().to_vec();
        (Link::Tls(Box::new(association)), contexts)
    } else {
        let mut association = options.establish_async(address).await.map_err(refused)?;
        crate::without_delay(association.inner_stream());
        let contexts = association.presentation_contexts().to_vec();
        (Link::Plain(Box::new(association)), contexts)
    };
    let accepted = negotiated
        .iter()
        .filter(|pc| pc.reason == PresentationContextResultReason::Acceptance)
        .map(|pc| store::Accepted {
            id: pc.id,
            abstract_syntax: pc.abstract_syntax.trim_end_matches('\0').to_owned(),
            transfer_syntax: pc.transfer_syntax.trim_end_matches('\0').to_owned(),
        })
        .collect();
    Ok((association, accepted))
}
