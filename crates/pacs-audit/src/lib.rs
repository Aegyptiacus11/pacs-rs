//! Audit trail messages: who did what to which data, in the format a hospital's audit
//! repository reads.
//!
//! A [`Message`] is one event of DICOM's Audit Trail Message Format Profile (PS3.15 A.5):
//! [`Message::to_xml`] writes it by the schema of A.5.1, and [`syslog::line`] wraps that in
//! the syslog message IHE's Record Audit Event [ITI-20] sends. The events pacs-rs reports
//! are built in [`events`], each from its table of A.5.3.
//!
//! This crate formats; it sends and stores nothing.

pub mod events;
pub mod syslog;
mod time;
mod xml;

use std::time::SystemTime;

pub use time::rfc3339;

/// A coded value: `csd-code`, `codeSystemName` and `originalText` of the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    pub code: &'static str,
    pub system: &'static str,
    pub text: &'static str,
}

impl Code {
    /// A code of the DICOM Controlled Terminology (PS3.16 Annex D).
    pub const fn dcm(code: &'static str, text: &'static str) -> Self {
        Code {
            code,
            system: "DCM",
            text,
        }
    }

    /// One of the integer values the schema inherited from RFC 3881, as ITI TF-2: 3.20.7.1.3
    /// asks for it: the integer, "RFC-3881", and RFC 3881's description.
    pub const fn rfc3881(code: &'static str, text: &'static str) -> Self {
        Code {
            code,
            system: "RFC-3881",
            text,
        }
    }
}

/// `EventActionCode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Read,
    Update,
    Delete,
    Execute,
}

impl Action {
    fn letter(self) -> &'static str {
        match self {
            Action::Create => "C",
            Action::Read => "R",
            Action::Update => "U",
            Action::Delete => "D",
            Action::Execute => "E",
        }
    }
}

/// `EventOutcomeIndicator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    MinorFailure,
    SeriousFailure,
    /// The reporting application is now unavailable.
    MajorFailure,
}

impl Outcome {
    fn number(self) -> &'static str {
        match self {
            Outcome::Success => "0",
            Outcome::MinorFailure => "4",
            Outcome::SeriousFailure => "8",
            Outcome::MajorFailure => "12",
        }
    }
}

/// Where on the network a participant is: `NetworkAccessPointTypeCode` and
/// `NetworkAccessPointID`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessPoint {
    /// A machine name, DNS names included (type 1).
    Machine(String),
    /// An IP address (type 2).
    Ip(String),
}

/// `ActiveParticipant`: a person or a process that took part in the event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    pub user_id: String,
    pub alternative_user_id: Option<String>,
    pub user_name: Option<String>,
    pub is_requestor: bool,
    pub roles: Vec<Code>,
    pub access_point: Option<AccessPoint>,
}

impl Participant {
    pub fn new(user_id: impl Into<String>, is_requestor: bool) -> Self {
        Participant {
            user_id: user_id.into(),
            alternative_user_id: None,
            user_name: None,
            is_requestor,
            roles: Vec::new(),
            access_point: None,
        }
    }

    pub fn role(mut self, role: Code) -> Self {
        self.roles.push(role);
        self
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.user_name = Some(name.into());
        self
    }

    pub fn alternative(mut self, id: impl Into<String>) -> Self {
        self.alternative_user_id = Some(id.into());
        self
    }

    pub fn at(mut self, access_point: Option<AccessPoint>) -> Self {
        self.access_point = access_point;
        self
    }
}

/// `AuditSourceIdentification`: the system that reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub enterprise_site_id: Option<String>,
    /// One digit of Table A.5.1.2.1-1; pacs-rs is `4`, an application server.
    pub type_code: &'static str,
}

impl Source {
    pub fn application_server(id: impl Into<String>) -> Self {
        Source {
            id: id.into(),
            enterprise_site_id: None,
            type_code: "4",
        }
    }
}

/// What names a participant object: its name, or the query that was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameOrQuery {
    Name(String),
    Query(Vec<u8>),
}

/// One SOP class of a study and how many of its instances the event touched
/// (`ParticipantObjectDescription/SOPClass`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SopClass {
    pub uid: String,
    pub instances: u64,
}

/// `ParticipantObjectIdentification`: data, a patient or a resource the event was about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub id: String,
    /// `ParticipantObjectTypeCode`: 1 person, 2 system object, 3 organization, 4 other.
    pub type_code: Option<&'static str>,
    /// `ParticipantObjectTypeCodeRole`, Table A.5.1.2.2-1.
    pub role: Option<&'static str>,
    /// `ParticipantObjectDataLifeCycle`, Table A.5.1.2.3-1.
    pub life_cycle: Option<&'static str>,
    pub id_type: Code,
    pub name: NameOrQuery,
    /// `ParticipantObjectDetail`: a type and a value, written in base64.
    pub details: Vec<(String, Vec<u8>)>,
    pub sop_classes: Vec<SopClass>,
}

/// One audit message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub event_id: Code,
    pub event_types: Vec<Code>,
    pub action: Option<Action>,
    pub time: SystemTime,
    pub outcome: Outcome,
    pub outcome_description: Option<String>,
    /// At least one.
    pub participants: Vec<Participant>,
    pub source: Source,
    pub objects: Vec<Object>,
}
