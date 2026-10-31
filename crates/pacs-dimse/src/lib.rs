//! DIMSE for pacs-rs, on dicom-ul's associations: C-ECHO as SCP and SCU, C-STORE as SCP.
//!
//! Behaviour follows DICOM 2026d: messages and statuses from PS3.7 (6.3, 9.1, 9.3, Annexes
//! C and E), the Verification and Storage Service Classes from PS3.4 (Annexes A and B), the
//! Upper Layer from PS3.8 and file meta information from PS3.10. `docs/interop.md` says what
//! comes next.

pub mod ae;
pub mod command;
pub mod status;

pub use ae::{AeError, AeTitle, RemoteAe};
pub use status::{Status, StatusClass};

/// The Verification SOP Class (PS3.4 A.4).
pub const VERIFICATION: &str = "1.2.840.10008.1.1";
