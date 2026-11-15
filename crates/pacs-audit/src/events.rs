//! The events pacs-rs reports, each built by its table of PS3.15 A.5.3.

use std::time::SystemTime;

use crate::{
    AccessPoint, Action, Code, Message, NameOrQuery, Object, Outcome, Participant, Source,
};

/// CID 400, Audit Event ID.
pub mod event {
    use crate::Code;
    pub const APPLICATION_ACTIVITY: Code = Code::dcm("110100", "Application Activity");
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
}

/// `ParticipantObjectIDTypeCode` 12 of RFC 3881: a URI.
pub const URI: Code = Code::rfc3881("12", "URI");

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

/// Where a caller is, from its socket address when the server knows it.
pub fn access_point(ip: Option<std::net::IpAddr>) -> Option<AccessPoint> {
    ip.map(|ip| AccessPoint::Ip(ip.to_string()))
}
