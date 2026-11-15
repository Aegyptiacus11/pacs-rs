//! An audit message as the syslog message of IHE's Record Audit Event [ITI-20]
//! (ITI TF-2: 3.20.4.1.2): RFC 5424, facility 10 (security), MSGID `IHE+RFC-3881`, no
//! structured data, and the DICOM XML as the message, in UTF-8 without a byte order mark.

use crate::{Message, Outcome, rfc3339};

/// The MSGID every audit message carries.
pub const MSGID: &str = "IHE+RFC-3881";
/// Security and authorization messages.
const FACILITY: u8 = 10;

/// Who sends: the HOSTNAME, APP-NAME and PROCID of the syslog header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sender {
    pub hostname: String,
    pub app_name: String,
    pub process_id: String,
}

/// A header field: printable ASCII without spaces, at most `max` characters, `-` when
/// there is nothing to say (RFC 5424, 6.2).
fn field(value: &str, max: usize) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| if c.is_ascii_graphic() { c } else { '_' })
        .take(max)
        .collect();
    if cleaned.is_empty() {
        "-".into()
    } else {
        cleaned
    }
}

/// `message` as one syslog message, without the line break or frame that ends it: those
/// are the transport's.
///
/// The severity is 5 (notice) for a success and 4 (warning) for a failure, the two values
/// ITI-20 names; the timestamp is when the message is written, the event's own time is in
/// the XML.
pub fn line(message: &Message, sender: &Sender, written: std::time::SystemTime) -> String {
    let severity = match message.outcome {
        Outcome::Success => 5,
        _ => 4,
    };
    format!(
        "<{}>1 {} {} {} {} {MSGID} - {}",
        FACILITY * 8 + severity,
        rfc3339(written),
        field(&sender.hostname, 255),
        field(&sender.app_name, 48),
        field(&sender.process_id, 128),
        message.to_xml()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_fields_are_printable_and_bounded() {
        assert_eq!(field("pacs.example", 255), "pacs.example");
        assert_eq!(field("my host\n", 255), "my_host_");
        assert_eq!(field("hôte", 255), "h_te");
        assert_eq!(field("", 48), "-");
        assert_eq!(field(&"x".repeat(60), 48).len(), 48);
    }
}
