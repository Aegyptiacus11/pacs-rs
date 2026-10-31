//! DIMSE for pacs-rs, on dicom-ul's associations: C-ECHO as SCP and SCU, C-STORE as SCP,
//! C-FIND and C-MOVE as SCU.
//!
//! Behaviour follows DICOM 2026d: messages and statuses from PS3.7 (6.3, 9.1, 9.3, Annexes
//! C and E), the Verification, Storage and Query/Retrieve Service Classes from PS3.4 (Annexes
//! A, B and C), the Upper Layer from PS3.8 and file meta information from PS3.10.
//! `docs/interop.md` says what comes next.
//!
//! Layers, lowest first: [`ae`], [`status`], [`command`] (Command Sets) and [`message`]
//! (messages over an association) are DIMSE itself; [`sop_classes`] and [`query`] are what
//! the services exchange; [`scp`] accepts associations, [`scu`] opens them.

use std::time::Duration;

pub mod ae;
pub mod command;
pub mod message;
pub mod query;
pub mod scp;
pub mod scu;
pub mod sop_classes;
pub mod status;

pub use ae::{AeError, AeTitle, RemoteAe};
pub use query::{DateRange, QueryError, StudyQuery};
pub use scp::{Destination, Scp};
pub use scu::ConnectError;
pub use scu::echo::{EchoError, echo};
pub use scu::find::{FindError, FindOutcome, find_studies};
pub use scu::retrieve::{MoveError, MoveOutcome, Progress, move_studies};
pub use status::{Status, StatusClass};

/// Implicit VR Little Endian, the default transfer syntax every AE accepts (PS3.5 10.1).
pub const IMPLICIT_VR_LE: &str = "1.2.840.10008.1.2";

/// Explicit VR Little Endian (PS3.5 A.2).
pub const EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";

/// The Verification SOP Class (PS3.4 A.4).
pub const VERIFICATION: &str = "1.2.840.10008.1.1";

/// pacs-rs's Implementation Class UID, written into the file meta information of what it
/// receives (PS3.10 7.1). A UUID-derived UID (PS3.5 B.2). Association negotiation still
/// carries dicom-ul's own, which pacs-rs cannot set.
pub const IMPLEMENTATION_CLASS_UID: &str = "2.25.198281755599061360028760269505564383885";

/// Implementation Version Name (SH, at most 16 characters).
pub const IMPLEMENTATION_VERSION_NAME: &str = concat!("PACS-RS ", env!("CARGO_PKG_VERSION"));

/// What pacs-rs's DIMSE side is configured with.
#[derive(Debug, Clone)]
pub struct Config {
    /// pacs-rs's own AE title: the calling AE title of its requests, and a called AE title
    /// it accepts.
    pub ae_title: AeTitle,
    /// Further called AE titles accepted on the same port, with the same presentation
    /// contexts.
    pub extra_ae_titles: Vec<AeTitle>,
    /// The AEs pacs-rs can call.
    pub remotes: Vec<RemoteAe>,
    /// Refuse associations whose calling AE title is not one of `remotes`.
    pub known_callers_only: bool,
    /// Associations served at once; one more is rejected as a local limit exceeded.
    pub max_associations: usize,
    /// The longest wait for the peer: to connect, for its next PDU, for a response.
    pub timeout: Duration,
    /// The longest wait between two C-MOVE responses: a remote AE may send none while it
    /// sends a large study.
    pub move_timeout: Duration,
    /// The largest data set kept; a larger C-STORE is read through and refused.
    pub max_data_set_bytes: usize,
}

impl Config {
    pub fn new(ae_title: AeTitle) -> Self {
        Config {
            ae_title,
            extra_ae_titles: Vec::new(),
            remotes: Vec::new(),
            known_callers_only: false,
            max_associations: 16,
            timeout: Duration::from_secs(30),
            move_timeout: Duration::from_secs(600),
            max_data_set_bytes: 1 << 30,
        }
    }

    pub fn remote(&self, id: &str) -> Option<&RemoteAe> {
        self.remotes.iter().find(|r| r.id == id)
    }
}
