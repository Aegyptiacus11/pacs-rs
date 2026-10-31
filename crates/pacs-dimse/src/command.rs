//! The Command Set of a DIMSE message: group 0000, Implicit VR Little Endian, elements in
//! increasing tag order, led by Command Group Length (PS3.7 6.3.1). The elements and their
//! VRs are those of the Command Dictionary, PS3.7 Table E.1-1.

use std::collections::BTreeMap;

use crate::status::Status;

/// Command Field (0000,0100) values, PS3.7 Table E.1-1.
pub mod field {
    pub const C_STORE_RQ: u16 = 0x0001;
    pub const C_STORE_RSP: u16 = 0x8001;
    pub const C_GET_RQ: u16 = 0x0010;
    pub const C_FIND_RQ: u16 = 0x0020;
    pub const C_MOVE_RQ: u16 = 0x0021;
    pub const C_ECHO_RQ: u16 = 0x0030;
    pub const C_ECHO_RSP: u16 = 0x8030;
    pub const C_CANCEL_RQ: u16 = 0x0FFF;
    /// Set in every response's Command Field, clear in every request's.
    pub const RESPONSE_BIT: u16 = 0x8000;
}

/// Command element tags (the element number; the group is always 0000).
pub mod tag {
    pub const COMMAND_GROUP_LENGTH: u16 = 0x0000;
    pub const AFFECTED_SOP_CLASS_UID: u16 = 0x0002;
    pub const COMMAND_FIELD: u16 = 0x0100;
    pub const MESSAGE_ID: u16 = 0x0110;
    pub const MESSAGE_ID_BEING_RESPONDED_TO: u16 = 0x0120;
    pub const PRIORITY: u16 = 0x0700;
    pub const COMMAND_DATA_SET_TYPE: u16 = 0x0800;
    pub const STATUS: u16 = 0x0900;
    pub const ERROR_COMMENT: u16 = 0x0902;
    pub const AFFECTED_SOP_INSTANCE_UID: u16 = 0x1000;
    pub const MOVE_ORIGINATOR_AE_TITLE: u16 = 0x1030;
    pub const MOVE_ORIGINATOR_MESSAGE_ID: u16 = 0x1031;
}

/// Command Data Set Type (0000,0800) when no data set follows (PS3.7 E.1-1, "Null");
/// any other value means one does.
pub const NO_DATA_SET: u16 = 0x0101;

/// Priority (0000,0700): MEDIUM (PS3.7 E.1-1).
pub const PRIORITY_MEDIUM: u16 = 0x0000;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CommandError {
    #[error("the command set ends inside an element")]
    Truncated,
    #[error("element (0000,{0:04X}) is in group {1:04X}, not 0000")]
    WrongGroup(u16, u16),
    #[error("element (0000,{0:04X}) comes after (0000,{1:04X}); tags must increase")]
    OutOfOrder(u16, u16),
    #[error("Command Group Length is {stated}, but {actual} bytes follow it")]
    GroupLength { stated: u32, actual: usize },
    #[error("required element (0000,{0:04X}) is missing")]
    Missing(u16),
    #[error("element (0000,{0:04X}) has {1} bytes, not the 2 of a US value")]
    NotUs(u16, usize),
}

/// A Command Set, element number to raw value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Command {
    elements: BTreeMap<u16, Vec<u8>>,
}

impl Command {
    pub fn new() -> Self {
        Self::default()
    }

    /// A US element (2 bytes, little endian).
    pub fn with_us(mut self, element: u16, value: u16) -> Self {
        self.elements.insert(element, value.to_le_bytes().to_vec());
        self
    }

    /// A UI element, padded to even length with one NUL (PS3.5 6.2, UI).
    pub fn with_ui(mut self, element: u16, uid: &str) -> Self {
        let mut value = uid.as_bytes().to_vec();
        if value.len() % 2 == 1 {
            value.push(0);
        }
        self.elements.insert(element, value);
        self
    }

    /// A text element (AE, LO), padded to even length with one space (PS3.5 6.2).
    pub fn with_text(mut self, element: u16, text: &str) -> Self {
        let mut value = text.as_bytes().to_vec();
        if value.len() % 2 == 1 {
            value.push(b' ');
        }
        self.elements.insert(element, value);
        self
    }

    pub fn get(&self, element: u16) -> Option<&[u8]> {
        self.elements.get(&element).map(Vec::as_slice)
    }

    pub fn us(&self, element: u16) -> Result<u16, CommandError> {
        let value = self.get(element).ok_or(CommandError::Missing(element))?;
        match value {
            [a, b] => Ok(u16::from_le_bytes([*a, *b])),
            _ => Err(CommandError::NotUs(element, value.len())),
        }
    }

    /// A UI value without its padding.
    pub fn ui(&self, element: u16) -> Result<&str, CommandError> {
        let value = self.get(element).ok_or(CommandError::Missing(element))?;
        Ok(std::str::from_utf8(value)
            .unwrap_or("")
            .trim_end_matches(['\0', ' ']))
    }

    /// A text value without leading or trailing spaces.
    pub fn text(&self, element: u16) -> Option<&str> {
        self.get(element)
            .and_then(|v| std::str::from_utf8(v).ok())
            .map(|s| s.trim_matches([' ', '\0']))
    }

    pub fn command_field(&self) -> Result<u16, CommandError> {
        self.us(tag::COMMAND_FIELD)
    }

    pub fn message_id(&self) -> Result<u16, CommandError> {
        self.us(tag::MESSAGE_ID)
    }

    pub fn status(&self) -> Result<Status, CommandError> {
        self.us(tag::STATUS).map(Status)
    }

    /// Whether a data set follows this command (Command Data Set Type other than 0101H).
    pub fn has_data_set(&self) -> Result<bool, CommandError> {
        Ok(self.us(tag::COMMAND_DATA_SET_TYPE)? != NO_DATA_SET)
    }

    /// The Command Set as sent: Command Group Length first, then every element in
    /// increasing tag order, each as tag, 32-bit length and value.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Vec::new();
        for (&element, value) in &self.elements {
            if element == tag::COMMAND_GROUP_LENGTH {
                continue;
            }
            push_element(&mut body, element, value);
        }
        let mut out = Vec::with_capacity(body.len() + 12);
        push_element(
            &mut out,
            tag::COMMAND_GROUP_LENGTH,
            &(body.len() as u32).to_le_bytes(),
        );
        out.extend_from_slice(&body);
        out
    }

    /// Parse a Command Set. Group Length, when present, must match what follows it.
    pub fn decode(bytes: &[u8]) -> Result<Self, CommandError> {
        let mut elements = BTreeMap::new();
        let mut rest = bytes;
        let mut previous: Option<u16> = None;
        let mut stated_length = None;
        while !rest.is_empty() {
            if rest.len() < 8 {
                return Err(CommandError::Truncated);
            }
            let group = u16::from_le_bytes([rest[0], rest[1]]);
            let element = u16::from_le_bytes([rest[2], rest[3]]);
            let length = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
            if group != 0 {
                return Err(CommandError::WrongGroup(element, group));
            }
            if let Some(previous) = previous
                && element <= previous
            {
                return Err(CommandError::OutOfOrder(element, previous));
            }
            let value = rest.get(8..8 + length).ok_or(CommandError::Truncated)?;
            if element == tag::COMMAND_GROUP_LENGTH && length == 4 {
                stated_length = Some((
                    u32::from_le_bytes([value[0], value[1], value[2], value[3]]),
                    rest.len() - 12,
                ));
            }
            elements.insert(element, value.to_vec());
            previous = Some(element);
            rest = &rest[8 + length..];
        }
        if let Some((stated, actual)) = stated_length
            && stated as usize != actual
        {
            return Err(CommandError::GroupLength { stated, actual });
        }
        Ok(Command { elements })
    }
}

fn push_element(out: &mut Vec<u8>, element: u16, value: &[u8]) {
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&element.to_le_bytes());
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value);
}

/// A C-ECHO-RQ, PS3.7 Table 9.3-12.
pub fn c_echo_rq(message_id: u16) -> Command {
    Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, crate::VERIFICATION)
        .with_us(tag::COMMAND_FIELD, field::C_ECHO_RQ)
        .with_us(tag::MESSAGE_ID, message_id)
        .with_us(tag::COMMAND_DATA_SET_TYPE, NO_DATA_SET)
}

/// The response to `request`: its Command Field with the response bit set, Message ID
/// Being Responded To, no data set, `status`, and the request's Affected SOP Class and
/// Instance UIDs when it has them (PS3.7 9.1.1.1.3-4, 9.1.5.1.3: equal to the request's).
pub fn response(request: &Command, status: Status, error_comment: Option<&str>) -> Command {
    let field = request.command_field().unwrap_or(0) | field::RESPONSE_BIT;
    let mut rsp = Command::new()
        .with_us(tag::COMMAND_FIELD, field)
        .with_us(
            tag::MESSAGE_ID_BEING_RESPONDED_TO,
            request.message_id().unwrap_or(0),
        )
        .with_us(tag::COMMAND_DATA_SET_TYPE, NO_DATA_SET)
        .with_us(tag::STATUS, status.0);
    for element in [tag::AFFECTED_SOP_CLASS_UID, tag::AFFECTED_SOP_INSTANCE_UID] {
        if let Ok(uid) = request.ui(element)
            && !uid.is_empty()
        {
            rsp = rsp.with_ui(element, uid);
        }
    }
    if let Some(comment) = error_comment {
        rsp = rsp.with_text(tag::ERROR_COMMENT, &error_comment_value(comment));
    }
    rsp
}

/// Error Comment is LO in the default repertoire: at most 64 characters, no backslash or
/// control characters (PS3.5 Table 6.2-1).
fn error_comment_value(comment: &str) -> String {
    comment
        .chars()
        .map(|c| match c {
            '\\' => ' ',
            c if c.is_ascii_control() => ' ',
            c if !c.is_ascii() => '?',
            c => c,
        })
        .take(64)
        .collect()
}
