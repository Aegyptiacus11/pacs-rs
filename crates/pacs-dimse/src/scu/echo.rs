//! C-ECHO SCU (PS3.7 9.1.5, 9.3.5): proof that a remote AE accepts pacs-rs and answers.

use std::time::{Duration, Instant};

use super::{ConnectError, Connected, connect};
use crate::ae::{AeTitle, RemoteAe};
use crate::command::{self, field, tag};
use crate::message::{self, Incoming, MessageError};
use crate::status::Status;
use crate::{IMPLICIT_VR_LE, VERIFICATION};

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
    } = connect(
        remote,
        calling,
        VERIFICATION,
        &[IMPLICIT_VR_LE],
        timeout,
        timeout,
    )
    .await?;
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
