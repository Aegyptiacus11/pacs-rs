//! pacs-rs as an association requestor: the association, and C-ECHO (PS3.7 9.1.5, 9.3.5),
//! proof that a remote AE accepts pacs-rs and answers. C-FIND comes next.

use std::time::{Duration, Instant};

use dicom_ul::ClientAssociationOptions;
use dicom_ul::association::AsyncClientAssociation;
use dicom_ul::pdu::PresentationContextResultReason;
use tokio::net::TcpStream;

use crate::ae::{AeTitle, RemoteAe};
use crate::command::{self, field, tag};
use crate::message::{self, Incoming, MessageError};
use crate::status::Status;
use crate::{IMPLICIT_VR_LE, VERIFICATION};

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
}

/// Associate with `remote` as `calling`, proposing `abstract_syntax` with
/// `transfer_syntaxes` in one presentation context; `timeout` bounds the connection and
/// every later wait for a PDU.
pub(crate) async fn connect(
    remote: &RemoteAe,
    calling: &AeTitle,
    abstract_syntax: &'static str,
    transfer_syntaxes: &[&str],
    timeout: Duration,
) -> Result<Connected, ConnectError> {
    let association = ClientAssociationOptions::new()
        .calling_ae_title(calling.as_str().to_owned())
        .called_ae_title(remote.ae_title.as_str().to_owned())
        .with_presentation_context(abstract_syntax, transfer_syntaxes.to_vec())
        .connection_timeout(timeout)
        .read_timeout(timeout)
        .write_timeout(timeout)
        .establish_async((remote.host.as_str(), remote.port))
        .await
        .map_err(|source| ConnectError::Association {
            ae: remote.ae_title.to_string(),
            host: remote.host.clone(),
            port: remote.port,
            source: Box::new(source),
        })?;
    let Some(context) = association
        .presentation_contexts()
        .iter()
        .find(|pc| pc.reason == PresentationContextResultReason::Acceptance)
        .map(|pc| pc.id)
    else {
        let _ = association.abort().await;
        return Err(ConnectError::NotAccepted(abstract_syntax));
    };
    Ok(Connected {
        association,
        context,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum EchoError {
    #[error(transparent)]
    Connect(#[from] ConnectError),
    #[error("no response within {0:?}")]
    Timeout(Duration),
    #[error(transparent)]
    Message(#[from] MessageError),
    #[error("the remote AE released or aborted the association instead of answering")]
    Ended,
    #[error("the response is not a C-ECHO-RSP to message {0}")]
    WrongResponse(u16),
    #[error("the remote AE answered with status {0}")]
    Status(Status),
}

/// A successful C-ECHO.
#[derive(Debug, Clone)]
pub struct Echo {
    /// From the first byte sent to the association's release.
    pub elapsed: Duration,
}

/// Open an association with `remote` as `calling`, propose the Verification SOP Class in
/// Implicit VR Little Endian (the default transfer syntax every AE accepts, PS3.5 10.1),
/// send one C-ECHO-RQ, check the C-ECHO-RSP, and release.
pub async fn echo(
    remote: &RemoteAe,
    calling: &AeTitle,
    timeout: Duration,
) -> Result<Echo, EchoError> {
    let started = Instant::now();
    let Connected {
        mut association,
        context,
        ..
    } = connect(remote, calling, VERIFICATION, &[IMPLICIT_VR_LE], timeout).await?;
    let message_id = 1;
    let exchange = async {
        message::send(
            &mut association,
            context,
            &command::c_echo_rq(message_id),
            None,
        )
        .await?;
        match message::receive(&mut association, 0).await? {
            Incoming::Message(response) => Ok(response),
            Incoming::Released | Incoming::Aborted => Err(EchoError::Ended),
        }
    };
    let response = match tokio::time::timeout(timeout, exchange).await {
        Ok(Ok(response)) => response,
        Ok(Err(e)) => {
            let _ = association.abort().await;
            return Err(e);
        }
        Err(_) => {
            let _ = association.abort().await;
            return Err(EchoError::Timeout(timeout));
        }
    };
    let rsp = &response.command;
    if rsp.command_field().ok() != Some(field::C_ECHO_RSP)
        || rsp.us(tag::MESSAGE_ID_BEING_RESPONDED_TO).ok() != Some(message_id)
    {
        let _ = association.abort().await;
        return Err(EchoError::WrongResponse(message_id));
    }
    let status = rsp.status().map_err(MessageError::from)?;
    // the association is released whatever the status: the exchange itself went well
    let _ = association.release().await;
    if status != Status::SUCCESS {
        return Err(EchoError::Status(status));
    }
    Ok(Echo {
        elapsed: started.elapsed(),
    })
}
