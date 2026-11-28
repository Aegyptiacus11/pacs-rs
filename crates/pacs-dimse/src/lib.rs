//! DIMSE for pacs-rs, on dicom-ul's associations: C-ECHO, C-STORE, C-FIND and C-MOVE, each
//! as SCP and as SCU, and C-GET as SCP.
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
pub mod conformance;
pub mod directory;
pub mod message;
pub mod project;
pub mod query;
pub mod scp;
pub mod scu;
pub mod sop_classes;
pub mod status;
#[cfg(feature = "tools")]
pub mod tools;

pub use ae::{AeError, AeTitle, ClientTls, RemoteAe};
pub use directory::Directory;
pub use project::{Project, ProjectError, Received};
pub use query::{DateRange, QueryError, StudyQuery};
pub use scp::{
    Capabilities, Destination, Observer, Peer, Queried, Retrieved, Scp, SentStudy, Service,
};
pub use scu::ConnectError;
pub use scu::echo::{EchoError, echo};
pub use scu::find::{FindError, FindOutcome, find_studies};
pub use scu::get::get_studies;
pub use scu::retrieve::{MoveError, MoveOutcome, Progress, move_studies};
pub use scu::store::{Accepted, Originator, Prepared, SendError, Sender, send_instances};
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
    /// The AEs pacs-rs can call, and those instances may be sent to (the destinations a
    /// C-MOVE may name: none unless set, as an archive this server only reads from is among
    /// the remotes too, and must not be written to because someone asked). Shared and live:
    /// a clone of the configuration sees what is changed in it.
    pub directory: Directory,
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
    /// Where a C-STORE's data set larger than `from_bytes` is written as it arrives, so that
    /// memory does not grow with it; `None` keeps every data set in memory, up to
    /// `max_data_set_bytes`.
    pub spool: Option<SpoolTo>,
    /// What some of pacs-rs's AE titles offer, each of its own: services and transfer
    /// syntaxes. An AE title not listed offers every service in every transfer syntax.
    pub capabilities: Vec<Capabilities>,
    /// With it, the listener speaks TLS and nothing else (PS3.15 B.12): a connection that
    /// does not open with a TLS handshake this configuration accepts is closed.
    pub tls: Option<std::sync::Arc<rustls::ServerConfig>>,
}

/// Where large received data sets are spooled ([`Config::spool`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpoolTo {
    pub dir: std::path::PathBuf,
    pub from_bytes: usize,
}

impl Config {
    pub fn new(ae_title: AeTitle) -> Self {
        Config {
            ae_title,
            extra_ae_titles: Vec::new(),
            directory: Directory::default(),
            known_callers_only: false,
            max_associations: 16,
            timeout: Duration::from_secs(30),
            move_timeout: Duration::from_secs(600),
            max_data_set_bytes: 1 << 30,
            spool: None,
            capabilities: Vec::new(),
            tls: None,
        }
    }

    pub fn remote(&self, id: &str) -> Option<RemoteAe> {
        self.directory.remote(id)
    }

    /// See [`Directory::move_destination`].
    pub fn move_destination(&self, ae_title: &str) -> Option<RemoteAe> {
        self.directory.move_destination(ae_title)
    }
}

/// Send what is written at once, without Nagle's algorithm (`TCP_NODELAY`).
///
/// A DIMSE message goes out as several small PDUs, a command and then its data set. Left to
/// Nagle, the second waits for the peer to acknowledge the first, and a peer that delays
/// its acknowledgements (most do, about 40 ms) holds every exchange back by that much: a
/// C-FIND with one match took 60 ms where it takes 15. A socket that refuses the option is
/// used as it is, with a warning.
pub(crate) fn without_delay(socket: &tokio::net::TcpStream) {
    if let Err(error) = socket.set_nodelay(true) {
        tracing::warn!(%error, "TCP_NODELAY could not be set on a DIMSE connection");
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn a_dimse_connection_sends_without_delay() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (calling, accepted) =
            tokio::join!(tokio::net::TcpStream::connect(address), listener.accept());
        let (calling, (accepted, _)) = (calling.unwrap(), accepted.unwrap());
        // as the system gives them: Nagle's algorithm on
        assert!(!calling.nodelay().unwrap() && !accepted.nodelay().unwrap());
        super::without_delay(&calling);
        super::without_delay(&accepted);
        assert!(calling.nodelay().unwrap() && accepted.nodelay().unwrap());
    }
}
