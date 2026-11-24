//! What the operations answered with several responses share (C-FIND, C-MOVE, C-GET): the
//! refusal of a request that cannot be served, and the look at what the peer sends while
//! it is being answered, a C-CANCEL-RQ above all (PS3.7 9.3.2.3, 9.3.3.3, 9.3.4.3).

use dicom_core::Tag;
use dicom_ul::Pdu;
use dicom_ul::association::AsyncAssociation;
use dicom_ul::pdu::{PDataValue, PDataValueType};
use std::future::Future as _;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::command::{Command, field, tag};
use crate::message::MessageError;
use crate::status::Status;

/// Why a request is answered with a Failure and nothing else.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Refusal {
    pub status: Status,
    pub comment: String,
    /// Offending Element (0000,0901): where the error is, when it is in one element.
    pub offending: Option<Tag>,
}

pub(super) fn refusal(
    status: Status,
    comment: impl Into<String>,
    offending: Option<Tag>,
) -> Refusal {
    Refusal {
        status,
        comment: comment.into(),
        offending,
    }
}

/// What the peer sent while an operation was being answered, if anything.
pub(super) enum Interruption {
    Nothing,
    Cancel,
    Released,
    Aborted,
}

/// Look, without waiting, at what the peer has sent while `operation` (its name, for an
/// error) is answered. Nothing is lost when nothing whole has arrived: the association
/// keeps what it has read of a PDU.
pub(super) async fn interruption<A, S>(
    association: &mut A,
    message_id: u16,
    operation: &str,
) -> Result<Interruption, MessageError>
where
    A: AsyncAssociation<S> + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    // Polled once, not timed: a timeout of zero still waits for the runtime's next timer tick,
    // a millisecond, and that between every two responses.
    let pdu = {
        let mut receiving = std::pin::pin!(AsyncAssociation::receive(association));
        let polled = std::future::poll_fn(|context| {
            std::task::Poll::Ready(receiving.as_mut().poll(context))
        })
        .await;
        match polled {
            std::task::Poll::Pending => return Ok(Interruption::Nothing),
            std::task::Poll::Ready(pdu) => pdu?,
        }
    };
    let unexpected = |what: &str| {
        MessageError::Protocol(format!("{what} while a {operation} is being answered"))
    };
    match pdu {
        Pdu::AbortRQ { .. } => Ok(Interruption::Aborted),
        Pdu::ReleaseRQ => {
            AsyncAssociation::send(association, &Pdu::ReleaseRP).await?;
            Ok(Interruption::Released)
        }
        Pdu::PData { data } => {
            // a C-CANCEL-RQ is a command of a few elements, in one fragment (PS3.7 9.3.2.3)
            let [
                PDataValue {
                    value_type: PDataValueType::Command,
                    is_last: true,
                    data,
                    ..
                },
            ] = data.as_slice()
            else {
                return Err(unexpected("a message that is not one command"));
            };
            let command = Command::decode(data)?;
            if command.command_field()? != field::C_CANCEL_RQ {
                // no asynchronous operations window is negotiated: one operation at a time
                return Err(unexpected("another request"));
            }
            // a cancel of another operation has nothing to cancel
            Ok(
                if command.us(tag::MESSAGE_ID_BEING_RESPONDED_TO)? == message_id {
                    Interruption::Cancel
                } else {
                    Interruption::Nothing
                },
            )
        }
        _ => Err(unexpected("an association PDU")),
    }
}

/// How the association stands after an operation that came to `T`.
#[derive(Debug)]
pub(super) enum After<T> {
    /// The final response is sent: the next message can be read.
    Open(T),
    /// The peer released the association in the middle, which was answered.
    Released(T),
    /// The peer aborted it.
    Aborted(T),
}
