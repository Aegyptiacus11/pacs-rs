//! DIMSE status codes: the classes of PS3.7 Annex C and the values pacs-rs answers with.

use std::fmt;

/// The Status (0000,0900) of a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Status(pub u16);

/// The Status Class of a code, PS3.7 Annex C (table at the head of the annex).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusClass {
    Success,
    Warning,
    Failure,
    Cancel,
    Pending,
    /// A value the standard does not allow ("Implementations shall not use Status Codes with
    /// values not listed").
    Unknown,
}

impl Status {
    /// Success (PS3.7 C.1.1).
    pub const SUCCESS: Status = Status(0x0000);
    /// Cancel: matching terminated due to a cancel request (PS3.4 Table C.4-1).
    pub const CANCEL: Status = Status(0xFE00);
    /// Warning: C-MOVE sub-operations complete, one or more failed or warned (Table C.4-2).
    pub const SUB_OPERATIONS_WARNING: Status = Status(0xB000);
    /// Refused: Move Destination unknown (PS3.4 Table C.4-2).
    pub const MOVE_DESTINATION_UNKNOWN: Status = Status(0xA801);
    /// Pending: a match, every Optional Key supported (PS3.4 Table C.4-1).
    pub const PENDING: Status = Status(0xFF00);
    /// Pending: a match, but one or more Optional Keys were not supported (Table C.4-1).
    pub const PENDING_OPTIONAL_KEYS: Status = Status(0xFF01);

    // General codes, PS3.7 C.5 and the C-STORE (9.1.1.1.9) and C-ECHO (9.1.5.1.4) lists.
    /// Refused: SOP Class not supported.
    pub const SOP_CLASS_NOT_SUPPORTED: Status = Status(0x0122);
    /// Unrecognized operation.
    pub const UNRECOGNIZED_OPERATION: Status = Status(0x0211);
    /// Mistyped argument: a parameter not agreed for use on the association.
    pub const MISTYPED_ARGUMENT: Status = Status(0x0212);

    // C-STORE codes of PS3.4 Table B.2-1. The A7xx, A9xx and Cxxx ranges are pacs-rs's to
    // fill (PS3.4 B.2.3); each cause has its own code, and Error Comment (0000,0902) says
    // more.
    /// Refused: Out of resources (A7xx): the store did not take the file.
    pub const OUT_OF_RESOURCES_STORE: Status = Status(0xA700);
    /// Refused: Out of resources (A7xx): the index did not take the instance.
    pub const OUT_OF_RESOURCES_INDEX: Status = Status(0xA701);
    /// Refused: Out of resources (A7xx): the data set is larger than pacs-rs accepts.
    pub const OUT_OF_RESOURCES_TOO_LARGE: Status = Status(0xA702);
    /// Error: Data Set does not match SOP Class (A9xx): SOP Class UID (0008,0016) is not the
    /// request's Affected SOP Class UID.
    pub const DATA_SET_DOES_NOT_MATCH_SOP_CLASS: Status = Status(0xA900);
    /// Error: Cannot understand (Cxxx): the data set cannot be parsed, or lacks the UIDs an
    /// instance must have.
    pub const CANNOT_UNDERSTAND: Status = Status(0xC000);
    /// Error: Cannot understand (Cxxx): SOP Instance UID (0008,0018) is not the request's
    /// Affected SOP Instance UID.
    pub const CANNOT_UNDERSTAND_INSTANCE_UID: Status = Status(0xC001);
    /// Error: Cannot understand (Cxxx): the UIDs contradict what the archive holds (an
    /// instance or series already stored under another series or study).
    pub const CANNOT_UNDERSTAND_CONFLICT: Status = Status(0xC002);
    /// Error: Cannot understand (Cxxx): sent to a project whose profile quarantines it (it may
    /// carry identification that cannot be removed), with no quarantine to keep it in.
    pub const CANNOT_UNDERSTAND_QUARANTINED: Status = Status(0xC003);
    /// Error: Cannot understand (Cxxx): the project could not de-identify it.
    pub const CANNOT_UNDERSTAND_DEIDENTIFICATION: Status = Status(0xC004);

    /// The class this code is in, by the convention of PS3.7 Annex C.
    pub fn class(self) -> StatusClass {
        match self.0 {
            0x0000 => StatusClass::Success,
            0x0001 | 0x0107 | 0x0116 | 0xB000..=0xBFFF => StatusClass::Warning,
            0xA000..=0xAFFF | 0xC000..=0xCFFF | 0x0100..=0x02FF => StatusClass::Failure,
            0xFE00 => StatusClass::Cancel,
            0xFF00 | 0xFF01 => StatusClass::Pending,
            _ => StatusClass::Unknown,
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04X}H ({:?})", self.0, self.class())
    }
}
