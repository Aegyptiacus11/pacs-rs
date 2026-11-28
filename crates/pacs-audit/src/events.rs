//! The events pacs-rs reports, each built by its table of PS3.15 A.5.3.

use std::time::SystemTime;

use crate::{
    AccessPoint, Action, Code, Message, NameOrQuery, Object, Outcome, Participant, SopClass, Source,
};

/// CID 400, Audit Event ID.
pub mod event {
    use crate::Code;
    pub const APPLICATION_ACTIVITY: Code = Code::dcm("110100", "Application Activity");
    pub const BEGIN_TRANSFERRING: Code = Code::dcm("110102", "Begin Transferring DICOM Instances");
    pub const INSTANCES_ACCESSED: Code = Code::dcm("110103", "DICOM Instances Accessed");
    pub const INSTANCES_TRANSFERRED: Code = Code::dcm("110104", "DICOM Instances Transferred");
    pub const STUDY_DELETED: Code = Code::dcm("110105", "DICOM Study Deleted");
    pub const EXPORT: Code = Code::dcm("110106", "Export");
    pub const QUERY: Code = Code::dcm("110112", "Query");
    pub const SECURITY_ALERT: Code = Code::dcm("110113", "Security Alert");
    pub const USER_AUTHENTICATION: Code = Code::dcm("110114", "User Authentication");
}

/// CID 401 and CID 403: what kind of event, within its Event ID.
pub mod kind {
    use crate::Code;
    pub const APPLICATION_START: Code = Code::dcm("110120", "Application Start");
    pub const APPLICATION_STOP: Code = Code::dcm("110121", "Application Stop");
    pub const LOGIN: Code = Code::dcm("110122", "Login");
    pub const LOGOUT: Code = Code::dcm("110123", "Logout");
    pub const NODE_AUTHENTICATION: Code = Code::dcm("110126", "Node Authentication");
    pub const NETWORK_CONFIGURATION: Code = Code::dcm("110128", "Network Configuration");
    pub const USE_OF_RESTRICTED_FUNCTION: Code = Code::dcm("110132", "Use of Restricted Function");
    pub const AUDIT_RECORDING_STOPPED: Code = Code::dcm("110133", "Audit Recording Stopped");
    pub const AUDIT_RECORDING_STARTED: Code = Code::dcm("110134", "Audit Recording Started");
    pub const SECURITY_ROLES_CHANGED: Code = Code::dcm("110136", "Security Roles Changed");
    pub const USER_SECURITY_ATTRIBUTES_CHANGED: Code =
        Code::dcm("110137", "User Security Attributes Changed");
}

/// CID 402, Audit Active Participant Role ID Code.
pub mod role {
    use crate::Code;
    pub const APPLICATION: Code = Code::dcm("110150", "Application");
    pub const APPLICATION_LAUNCHER: Code = Code::dcm("110151", "Application Launcher");
    pub const DESTINATION: Code = Code::dcm("110152", "Destination Role ID");
    pub const SOURCE: Code = Code::dcm("110153", "Source Role ID");
    pub const DESTINATION_MEDIA: Code = Code::dcm("110154", "Destination Media");
}

/// `ParticipantObjectIDTypeCode` 12 of RFC 3881: a URI.
pub const URI: Code = Code::rfc3881("12", "URI");
/// `ParticipantObjectIDTypeCode` 2 of RFC 3881: a patient number.
pub const PATIENT_NUMBER: Code = Code::rfc3881("2", "Patient Number");
/// The ID of a node a security alert is about (A.5.3.11).
pub const NODE_ID: Code = Code::dcm("110182", "Node ID");
/// The ID of a DICOM query's participant object is the UID of the SOP Class queried
/// (A.5.3.10).
pub const SOP_CLASS_UID: Code = Code::dcm("110181", "SOP Class UID");
/// The ID of a study participant object is its Study Instance UID.
pub const STUDY_INSTANCE_UID: Code = Code::dcm("110180", "Study Instance UID");

/// A study an event was about, and whose it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Study {
    pub study_instance_uid: String,
    pub patient_id: Option<String>,
    pub patient_name: Option<String>,
    /// The SOP classes concerned and how many instances of each; empty when the event
    /// does not count them.
    pub sop_classes: Vec<SopClass>,
}

/// The participant objects of an event about `studies` of one patient: each study, then
/// the patient, as the tables of A.5.3.6 to A.5.3.8 lay them out. A patient without an ID
/// or a name (de-identified data may have neither) is written as `unknown`: the fields
/// are mandatory and may not be empty.
fn study_objects(studies: &[Study]) -> Vec<Object> {
    let mut objects: Vec<Object> = studies
        .iter()
        .map(|study| Object {
            id: study.study_instance_uid.clone(),
            type_code: Some("2"),
            // 3: Report, what the tables ask of a study
            role: Some("3"),
            life_cycle: None,
            id_type: STUDY_INSTANCE_UID,
            name: NameOrQuery::Name(study.study_instance_uid.clone()),
            details: Vec::new(),
            sop_classes: study.sop_classes.clone(),
        })
        .collect();
    if let Some(first) = studies.first() {
        let known = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("unknown")
                .to_owned()
        };
        objects.push(Object {
            id: known(&first.patient_id),
            type_code: Some("1"),
            role: Some("1"),
            life_cycle: None,
            id_type: PATIENT_NUMBER,
            name: NameOrQuery::Name(known(&first.patient_name)),
            details: Vec::new(),
            sop_classes: Vec::new(),
        });
    }
    objects
}

/// DICOM Instances Transferred (A.5.3.7): instances of one study went from `sender` to
/// `receiver`, one of which is this server.
///
/// `action` is Create when the receiver did not hold them, Update when it replaced its
/// copies, and Read when it already held them or when this server is the sender.
pub fn instances_transferred(
    source: &Source,
    time: SystemTime,
    action: Action,
    outcome: Outcome,
    sender: Participant,
    receiver: Participant,
    study: &Study,
) -> Message {
    Message {
        event_id: event::INSTANCES_TRANSFERRED,
        event_types: Vec::new(),
        action: Some(action),
        time,
        outcome,
        outcome_description: None,
        participants: vec![sender.role(role::SOURCE), receiver.role(role::DESTINATION)],
        source: source.clone(),
        objects: study_objects(std::slice::from_ref(study)),
    }
}

/// Begin Transferring DICOM Instances (A.5.3.3): `sender` was asked to send `studies`, of
/// one patient, to `receiver`; their arrival is another event, DICOM Instances
/// Transferred. `others` are whoever else took part, the person who asked above all.
pub fn begin_transferring(
    source: &Source,
    time: SystemTime,
    outcome: Outcome,
    sender: Participant,
    receiver: Participant,
    others: Vec<Participant>,
    studies: &[Study],
) -> Message {
    let mut participants = vec![sender.role(role::SOURCE), receiver.role(role::DESTINATION)];
    participants.extend(others);
    Message {
        event_id: event::BEGIN_TRANSFERRING,
        event_types: Vec::new(),
        action: Some(Action::Execute),
        time,
        outcome,
        outcome_description: None,
        participants,
        source: source.clone(),
        objects: study_objects(studies),
    }
}

/// DICOM Instances Accessed (A.5.3.6): `who` viewed, used or changed instances of
/// `studies`, all of one patient; with [`Action::Delete`], removed some of them and not
/// the whole study.
pub fn instances_accessed(
    source: &Source,
    time: SystemTime,
    action: Action,
    outcome: Outcome,
    who: Vec<Participant>,
    studies: &[Study],
) -> Message {
    Message {
        event_id: event::INSTANCES_ACCESSED,
        event_types: Vec::new(),
        action: Some(action),
        time,
        outcome,
        outcome_description: None,
        participants: who,
        source: source.clone(),
        objects: study_objects(studies),
    }
}

/// DICOM Study Deleted (A.5.3.8): `who` deleted `studies`, all of one patient, with every
/// instance in them.
pub fn study_deleted(
    source: &Source,
    time: SystemTime,
    outcome: Outcome,
    who: Vec<Participant>,
    studies: &[Study],
) -> Message {
    Message {
        event_id: event::STUDY_DELETED,
        event_types: Vec::new(),
        action: Some(Action::Delete),
        time,
        outcome,
        outcome_description: None,
        participants: who,
        source: source.clone(),
        objects: study_objects(studies),
    }
}

/// Export (A.5.3.4): data left the system's control, here as a file a caller downloads.
///
/// `remote` receives it and `exporter` is the local process; one of the two is the
/// requestor (A.5.3.4.1). `media` is where it went: for a download, the URI asked for,
/// with the receiver's network access point, which the table then requires.
pub fn export(
    source: &Source,
    time: SystemTime,
    outcome: Outcome,
    remote: Participant,
    exporter: Participant,
    mut media: Participant,
    studies: &[Study],
) -> Message {
    media.is_requestor = false;
    Message {
        event_id: event::EXPORT,
        event_types: Vec::new(),
        action: Some(Action::Read),
        time,
        outcome,
        outcome_description: None,
        participants: vec![
            remote.role(role::DESTINATION),
            exporter.role(role::SOURCE),
            media.role(role::DESTINATION_MEDIA),
        ],
        source: source.clone(),
        objects: study_objects(studies),
    }
}

/// Query (A.5.3.10): `issuer` asked `responder` a question; the answer is not recorded.
///
/// For a query that is not a DICOM query the table lets the object say what was asked in
/// the protocol's own terms: `what` is the URI queried and `query` its parameters as sent.
/// `others` are third parties, such as the person a process asks for.
#[allow(clippy::too_many_arguments)]
pub fn query(
    source: &Source,
    time: SystemTime,
    outcome: Outcome,
    issuer: Participant,
    responder: Participant,
    others: Vec<Participant>,
    what: &str,
    query: &[u8],
) -> Message {
    let mut participants = vec![issuer.role(role::SOURCE), responder.role(role::DESTINATION)];
    participants.extend(others);
    Message {
        event_id: event::QUERY,
        event_types: Vec::new(),
        action: Some(Action::Execute),
        time,
        outcome,
        outcome_description: None,
        participants,
        source: source.clone(),
        objects: vec![Object {
            id: what.to_owned(),
            type_code: Some("2"),
            role: Some("3"),
            life_cycle: None,
            id_type: URI,
            name: NameOrQuery::Query(query.to_vec()),
            details: Vec::new(),
            sop_classes: Vec::new(),
        }],
    }
}

/// Query (A.5.3.10), of a DICOM query: `issuer` asked `responder` with a C-FIND on
/// `sop_class`, the Information Model queried. `identifier` is the Data Set of the query
/// as it was sent, in `transfer_syntax`, which the table has written beside it.
#[allow(clippy::too_many_arguments)]
pub fn dicom_query(
    source: &Source,
    time: SystemTime,
    outcome: Outcome,
    issuer: Participant,
    responder: Participant,
    sop_class: &str,
    transfer_syntax: &str,
    identifier: &[u8],
) -> Message {
    Message {
        event_id: event::QUERY,
        event_types: Vec::new(),
        action: Some(Action::Execute),
        time,
        outcome,
        outcome_description: None,
        participants: vec![issuer.role(role::SOURCE), responder.role(role::DESTINATION)],
        source: source.clone(),
        objects: vec![Object {
            id: sop_class.to_owned(),
            type_code: Some("2"),
            role: Some("3"),
            life_cycle: None,
            id_type: SOP_CLASS_UID,
            name: NameOrQuery::Query(identifier.to_vec()),
            details: vec![("TransferSyntax".into(), transfer_syntax.as_bytes().to_vec())],
            sop_classes: Vec::new(),
        }],
    }
}

/// Application Activity (A.5.3.1): the server started or stopped.
///
/// `process` is the application's identity as its logs have it; `ae_titles` are the
/// DICOM AE titles it answers to, when it speaks DICOM.
pub fn application_activity(
    source: &Source,
    time: SystemTime,
    started: bool,
    process: &str,
    ae_titles: &[String],
) -> Message {
    let mut application = Participant::new(process, false).role(role::APPLICATION);
    if !ae_titles.is_empty() {
        // A.5.2.2: the AE titles of a DICOM application, in this form
        application = application.alternative(format!("AETITLES={}", ae_titles.join(";")));
    }
    Message {
        event_id: event::APPLICATION_ACTIVITY,
        event_types: vec![if started {
            kind::APPLICATION_START
        } else {
            kind::APPLICATION_STOP
        }],
        action: Some(Action::Execute),
        time,
        outcome: Outcome::Success,
        outcome_description: None,
        participants: vec![application],
        source: source.clone(),
        objects: Vec::new(),
    }
}

/// User Authentication (A.5.3.12): someone signed in or out, or tried to.
///
/// `user` is the person authenticated or claimed, with where they came from: the table
/// makes the network access point mandatory, so a caller that does not know it names the
/// server itself. `node` is the system that checked.
pub fn user_authentication(
    source: &Source,
    time: SystemTime,
    login: bool,
    user: Participant,
    node: &str,
    outcome: Outcome,
    description: Option<String>,
) -> Message {
    Message {
        event_id: event::USER_AUTHENTICATION,
        event_types: vec![if login { kind::LOGIN } else { kind::LOGOUT }],
        action: Some(Action::Execute),
        time,
        outcome,
        outcome_description: description,
        participants: vec![user, Participant::new(node, false)],
        source: source.clone(),
        objects: Vec::new(),
    }
}

/// Security Alert (A.5.3.11): something about the system's security happened, of the
/// kind `alert` (CID 403).
///
/// `reporter` is the process that reports; `performer` whoever did it, when someone did;
/// `subject` the URI of what the alert is about, and `description` what happened, in
/// words (the `Alert Description` detail the table requires).
#[allow(clippy::too_many_arguments)]
pub fn security_alert(
    source: &Source,
    time: SystemTime,
    alert: Code,
    outcome: Outcome,
    reporter: &str,
    performer: Option<Participant>,
    subject: &str,
    description: &str,
) -> Message {
    // the reporter is the requestor only when nobody else acted
    let mut participants = vec![Participant::new(reporter, performer.is_none())];
    if let Some(mut performer) = performer {
        // "Performing persons and/or processes": UserIsRequestor shall be false
        performer.is_requestor = false;
        participants.push(performer);
    }
    Message {
        event_id: event::SECURITY_ALERT,
        event_types: vec![alert],
        action: Some(Action::Execute),
        time,
        outcome,
        outcome_description: None,
        participants,
        source: source.clone(),
        objects: vec![Object {
            id: subject.to_owned(),
            type_code: Some("2"),
            // 13: Security Resource
            role: Some("13"),
            life_cycle: None,
            id_type: URI,
            name: NameOrQuery::Name(subject.to_owned()),
            details: vec![("Alert Description".into(), description.as_bytes().to_vec())],
            sop_classes: Vec::new(),
        }],
    }
}

/// Security Alert, Node Authentication (A.5.3.11): `node` tried to connect and was not
/// let in. The standard's own example of an alert; only failures are reported, as its
/// note expects.
pub fn node_authentication_failure(
    source: &Source,
    time: SystemTime,
    reporter: &str,
    node: &str,
    description: &str,
) -> Message {
    Message {
        event_id: event::SECURITY_ALERT,
        event_types: vec![kind::NODE_AUTHENTICATION],
        action: Some(Action::Execute),
        time,
        outcome: Outcome::MinorFailure,
        outcome_description: None,
        participants: vec![Participant::new(reporter, true)],
        source: source.clone(),
        objects: vec![Object {
            id: node.to_owned(),
            type_code: Some("2"),
            role: Some("13"),
            life_cycle: None,
            id_type: NODE_ID,
            name: NameOrQuery::Name(node.to_owned()),
            details: vec![("Alert Description".into(), description.as_bytes().to_vec())],
            sop_classes: Vec::new(),
        }],
    }
}

/// Where a caller is, from its socket address when the server knows it.
pub fn access_point(ip: Option<std::net::IpAddr>) -> Option<AccessPoint> {
    ip.map(|ip| AccessPoint::Ip(ip.to_string()))
}
