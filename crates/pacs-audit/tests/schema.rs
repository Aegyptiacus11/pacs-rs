//! Every kind of message this crate builds, validated against DICOM's schema (PS3.15
//! A.5.1.1) by `xmllint`, and their exact text for one of them. The validation runs when
//! `PACS_TEST_XMLLINT` names the program.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pacs_audit::events::{self, Study, kind};
use pacs_audit::syslog::{self, Sender};
use pacs_audit::{AccessPoint, Action, Message, Outcome, Participant, SopClass, Source};

fn when() -> SystemTime {
    UNIX_EPOCH + Duration::new(1_794_773_045, 123_000_000)
}

fn source() -> Source {
    Source::application_server("pacs-rs")
}

fn ada() -> Participant {
    Participant::new("ada@pacs-rs", true)
        .named("ada")
        .at(Some(AccessPoint::Ip("192.0.2.7".into())))
}

fn server() -> Participant {
    Participant::new("pacs-server@host", false)
}

/// A remote AE, as a participant.
fn hospital() -> Participant {
    Participant::new("HOSPITAL", false)
        .alternative("AETITLES=HOSPITAL")
        .at(Some(AccessPoint::Machine("pacs.hospital.example".into())))
}

fn ct() -> Study {
    Study {
        study_instance_uid: "1.2.826.0.1.3680043.2.1125.1".into(),
        patient_id: Some("P-0042".into()),
        patient_name: Some("DOE^JANE <&>".into()),
        sop_classes: vec![
            SopClass {
                uid: "1.2.840.10008.5.1.4.1.1.2".into(),
                instances: 187,
            },
            SopClass {
                uid: "1.2.840.10008.5.1.4.1.1.7".into(),
                instances: 1,
            },
        ],
    }
}

/// A study of a patient with neither ID nor name, its instances not counted.
fn nameless() -> Study {
    Study {
        study_instance_uid: "1.2.3".into(),
        patient_id: None,
        patient_name: Some("  ".into()),
        sop_classes: Vec::new(),
    }
}

/// The events about studies.
fn study_messages() -> Vec<(&'static str, Message)> {
    let media = || {
        Participant::new("/studies/abc/archive", false)
            .at(Some(AccessPoint::Ip("192.0.2.7".into())))
    };
    vec![
        (
            "received",
            events::instances_transferred(
                &source(),
                when(),
                Action::Create,
                Outcome::Success,
                ada(),
                server(),
                &ct(),
            ),
        ),
        (
            "sent",
            events::instances_transferred(
                &source(),
                when(),
                Action::Read,
                Outcome::Success,
                server(),
                ada(),
                &nameless(),
            ),
        ),
        (
            "accessed",
            events::instances_accessed(
                &source(),
                when(),
                Action::Read,
                Outcome::Success,
                vec![ada()],
                &[nameless()],
            ),
        ),
        (
            "partly-deleted",
            events::instances_accessed(
                &source(),
                when(),
                Action::Delete,
                Outcome::Success,
                vec![ada()],
                &[ct()],
            ),
        ),
        (
            "study-deleted",
            events::study_deleted(
                &source(),
                when(),
                Outcome::Success,
                vec![ada()],
                &[ct(), nameless()],
            ),
        ),
        (
            "export",
            events::export(
                &source(),
                when(),
                Outcome::Success,
                ada(),
                server(),
                media(),
                &[ct()],
            ),
        ),
        (
            "query",
            events::query(
                &source(),
                when(),
                Outcome::Success,
                ada(),
                server(),
                Vec::new(),
                "/dicom-web/studies",
                b"PatientName=DOE*&limit=25",
            ),
        ),
        (
            "query-of-everything",
            events::query(
                &source(),
                when(),
                Outcome::Success,
                ada(),
                server(),
                Vec::new(),
                "/studies",
                b"/studies",
            ),
        ),
        (
            "query-of-a-remote-ae",
            events::query(
                &source(),
                when(),
                Outcome::SeriousFailure,
                server().alternative("AETITLES=PACSRS"),
                hospital(),
                vec![ada()],
                "/modalities/hospital/studies",
                b"/modalities/hospital/studies?PatientID=P-0042",
            ),
        ),
        (
            "begin-transferring",
            events::begin_transferring(
                &source(),
                when(),
                Outcome::Success,
                hospital(),
                server().alternative("AETITLES=RESEARCH"),
                vec![ada()],
                &[nameless()],
            ),
        ),
        (
            "received-by-c-store",
            events::instances_transferred(
                &source(),
                when(),
                Action::Create,
                Outcome::Success,
                hospital(),
                server().alternative("AETITLES=PACSRS"),
                &ct(),
            ),
        ),
        (
            "node-refused",
            events::node_authentication_failure(
                &source(),
                when(),
                "pacs-server@host",
                "203.0.113.9",
                "association rejected: calling AE title not recognized",
            ),
        ),
    ]
}

/// One of each event, with values that would break a careless writer.
fn messages() -> Vec<(&'static str, Message)> {
    let hostile = "<script>&\"quotes\" 'and'\nlines\t\u{0}";
    let mut all = study_messages();
    all.extend(other_messages(hostile));
    all
}

fn other_messages(hostile: &'static str) -> Vec<(&'static str, Message)> {
    vec![
        (
            "start",
            events::application_activity(&source(), when(), true, "pacs-server@host", &[]),
        ),
        (
            "stop-with-ae-titles",
            events::application_activity(
                &source(),
                when(),
                false,
                "pacs-server@host",
                &["PACSRS".into(), "RESEARCH".into()],
            ),
        ),
        (
            "login",
            events::user_authentication(
                &source(),
                when(),
                true,
                ada(),
                "pacs-rs",
                Outcome::Success,
                None,
            ),
        ),
        (
            "login-refused",
            events::user_authentication(
                &source(),
                when(),
                true,
                Participant::new(hostile, true).at(Some(AccessPoint::Ip("2001:db8::1".into()))),
                "pacs-rs",
                Outcome::MinorFailure,
                Some(hostile.into()),
            ),
        ),
        (
            "logout",
            events::user_authentication(
                &source(),
                when(),
                false,
                ada(),
                "pacs-rs",
                Outcome::Success,
                None,
            ),
        ),
        (
            "alert-with-performer",
            events::security_alert(
                &source(),
                when(),
                kind::USER_SECURITY_ATTRIBUTES_CHANGED,
                Outcome::Success,
                "pacs-server@host",
                Some(ada()),
                "/users/7f3a",
                hostile,
            ),
        ),
        (
            "alert-by-the-server",
            events::security_alert(
                &source(),
                when(),
                kind::AUDIT_RECORDING_STARTED,
                Outcome::Success,
                "pacs-server@host",
                None,
                "/audit",
                "audit recording started",
            ),
        ),
    ]
}

fn one(name: &str) -> Message {
    let (_, message) = messages()
        .into_iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no message {name}"));
    message
}

fn schema() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/dicom-audit-message.rng")
}

/// Whether `xml` is valid by the schema, and what xmllint said.
fn validate(xmllint: &str, dir: &Path, name: &str, xml: &str) -> (bool, String) {
    let file = dir.join(format!("{name}.xml"));
    std::fs::write(&file, xml).unwrap();
    let output = Command::new(xmllint)
        .arg("--noout")
        .arg("--relaxng")
        .arg(schema())
        .arg(&file)
        .output()
        .unwrap_or_else(|e| panic!("PACS_TEST_XMLLINT names {xmllint:?}, which did not run: {e}"));
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn every_message_is_valid_by_the_dicom_schema() {
    let Ok(xmllint) = std::env::var("PACS_TEST_XMLLINT") else {
        eprintln!("PACS_TEST_XMLLINT not set; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for (name, message) in messages() {
        let xml = message.to_xml();
        let (valid, said) = validate(&xmllint, dir.path(), name, &xml);
        assert!(valid, "{name}: {said}\n{xml}");
    }

    // The validator does refuse: an outcome the schema does not list, an element out of
    // its place, and a message without a participant.
    let good = one("login").to_xml();
    for (name, bad) in [
        (
            "bad-outcome",
            good.replace("EventOutcomeIndicator=\"0\"", "EventOutcomeIndicator=\"3\""),
        ),
        (
            "bad-order",
            good.replace("<AuditMessage>", "<AuditMessage><AuditSourceIdentification AuditSourceID=\"x\"/>"),
        ),
        (
            "no-participant",
            events::application_activity(&source(), when(), true, "", &[])
                .to_xml()
                .replace("<ActiveParticipant UserIsRequestor=\"false\"><RoleIDCode csd-code=\"110150\" codeSystemName=\"DCM\" originalText=\"Application\"/></ActiveParticipant>", ""),
        ),
    ] {
        assert_ne!(bad, good, "{name}: the message was not changed");
        let (valid, _) = validate(&xmllint, dir.path(), name, &bad);
        assert!(!valid, "{name} passed validation:\n{bad}");
    }
}

#[test]
fn a_login_as_xml_and_as_a_syslog_message() {
    let message = one("login");
    let xml = message.to_xml();
    assert_eq!(
        xml,
        concat!(
            "<AuditMessage>",
            "<EventIdentification EventActionCode=\"E\" EventDateTime=\"2026-11-15T20:04:05.123Z\" EventOutcomeIndicator=\"0\">",
            "<EventID csd-code=\"110114\" codeSystemName=\"DCM\" originalText=\"User Authentication\"/>",
            "<EventTypeCode csd-code=\"110122\" codeSystemName=\"DCM\" originalText=\"Login\"/>",
            "</EventIdentification>",
            "<ActiveParticipant UserID=\"ada@pacs-rs\" UserName=\"ada\" UserIsRequestor=\"true\" NetworkAccessPointID=\"192.0.2.7\" NetworkAccessPointTypeCode=\"2\"/>",
            "<ActiveParticipant UserID=\"pacs-rs\" UserIsRequestor=\"false\"/>",
            "<AuditSourceIdentification AuditSourceID=\"pacs-rs\"><AuditSourceTypeCode csd-code=\"4\"/></AuditSourceIdentification>",
            "</AuditMessage>"
        )
    );

    let sender = Sender {
        hostname: "pacs.example".into(),
        app_name: "pacs-rs".into(),
        process_id: "4242".into(),
    };
    let written = when() + Duration::from_millis(2);
    let line = syslog::line(&message, &sender, written);
    // facility 10, severity 5; version 1; no structured data; no byte order mark
    assert_eq!(
        line,
        format!("<85>1 2026-11-15T20:04:05.125Z pacs.example pacs-rs 4242 IHE+RFC-3881 - {xml}")
    );
    // a failure is a warning
    let refused = one("login-refused");
    assert!(syslog::line(&refused, &sender, written).starts_with("<84>1 "));
}

#[test]
fn a_message_is_one_line_whatever_it_holds() {
    for (name, message) in messages() {
        let xml = message.to_xml();
        assert!(
            !xml.contains(['\n', '\r', '\t', '\u{0}']),
            "{name}: {xml:?}"
        );
        assert!(
            !xml.contains("=\"\""),
            "{name} has an empty attribute: {xml}"
        );
    }
    // what was hostile is still there, as text
    let refused = one("login-refused").to_xml();
    assert!(refused.contains(
        "UserID=\"&lt;script&gt;&amp;&quot;quotes&quot; &apos;and&apos;&#10;lines&#9;\u{FFFD}\""
    ));
}

#[test]
fn an_alert_names_its_subject_and_says_what_happened() {
    let alert = one("alert-with-performer").to_xml();
    // "<script>&..." in base64: the detail is never read as XML
    assert!(
        alert.contains("<ParticipantObjectDetail type=\"Alert Description\" value=\"PHNjcmlwdD4m")
    );
    assert!(alert.contains("ParticipantObjectID=\"/users/7f3a\" ParticipantObjectTypeCode=\"2\" ParticipantObjectTypeCodeRole=\"13\""));
    assert!(alert.contains("<ParticipantObjectIDTypeCode csd-code=\"12\" codeSystemName=\"RFC-3881\" originalText=\"URI\"/>"));
    // the reporter, then the person who did it, who is not the requestor of an alert
    assert!(alert.contains("<ActiveParticipant UserID=\"pacs-server@host\" UserIsRequestor=\"false\"/><ActiveParticipant UserID=\"ada@pacs-rs\" UserName=\"ada\" UserIsRequestor=\"false\""));
    let own = one("alert-by-the-server").to_xml();
    assert!(
        own.contains("<ActiveParticipant UserID=\"pacs-server@host\" UserIsRequestor=\"true\"/>")
    );
}

#[test]
fn the_events_about_studies_name_the_study_and_the_patient() {
    let study = "ParticipantObjectID=\"1.2.826.0.1.3680043.2.1125.1\" ParticipantObjectTypeCode=\"2\" ParticipantObjectTypeCodeRole=\"3\"><ParticipantObjectIDTypeCode csd-code=\"110180\" codeSystemName=\"DCM\" originalText=\"Study Instance UID\"/>";
    let classes = "<ParticipantObjectDescription><SOPClass UID=\"1.2.840.10008.5.1.4.1.1.2\" NumberOfInstances=\"187\"/><SOPClass UID=\"1.2.840.10008.5.1.4.1.1.7\" NumberOfInstances=\"1\"/></ParticipantObjectDescription>";
    let patient = "<ParticipantObjectIdentification ParticipantObjectID=\"P-0042\" ParticipantObjectTypeCode=\"1\" ParticipantObjectTypeCodeRole=\"1\"><ParticipantObjectIDTypeCode csd-code=\"2\" codeSystemName=\"RFC-3881\" originalText=\"Patient Number\"/><ParticipantObjectName>DOE^JANE &lt;&amp;&gt;</ParticipantObjectName></ParticipantObjectIdentification>";

    // received: the caller is the source, the server the destination, and the action Create
    let received = one("received").to_xml();
    for part in [
        "<EventIdentification EventActionCode=\"C\"",
        "<EventID csd-code=\"110104\" codeSystemName=\"DCM\" originalText=\"DICOM Instances Transferred\"/>",
        "UserID=\"ada@pacs-rs\" UserName=\"ada\" UserIsRequestor=\"true\" NetworkAccessPointID=\"192.0.2.7\" NetworkAccessPointTypeCode=\"2\"><RoleIDCode csd-code=\"110153\" codeSystemName=\"DCM\" originalText=\"Source Role ID\"/>",
        "UserID=\"pacs-server@host\" UserIsRequestor=\"false\"><RoleIDCode csd-code=\"110152\" codeSystemName=\"DCM\" originalText=\"Destination Role ID\"/>",
        study,
        classes,
        patient,
    ] {
        assert!(received.contains(part), "no {part}\nin {received}");
    }
    // the study comes before the patient
    assert!(received.find(study).unwrap() < received.find(patient).unwrap());

    // sent: the roles the other way round; no counts, and a patient without a name or an ID
    let sent = one("sent").to_xml();
    assert!(sent.contains("EventActionCode=\"R\""));
    assert!(sent.contains(
        "UserID=\"pacs-server@host\" UserIsRequestor=\"false\"><RoleIDCode csd-code=\"110153\""
    ));
    assert!(!sent.contains("SOPClass"));
    assert!(sent.contains("ParticipantObjectID=\"unknown\" ParticipantObjectTypeCode=\"1\""));
    assert!(sent.contains("<ParticipantObjectName>unknown</ParticipantObjectName>"));

    let deleted = one("study-deleted").to_xml();
    assert!(deleted.contains("EventActionCode=\"D\"") && deleted.contains("csd-code=\"110105\""));
    // both studies, then the one patient
    assert_eq!(deleted.matches("csd-code=\"110180\"").count(), 2);
    assert_eq!(
        deleted.matches("originalText=\"Patient Number\"").count(),
        1
    );
    let partly = one("partly-deleted").to_xml();
    assert!(partly.contains("EventActionCode=\"D\"") && partly.contains("csd-code=\"110103\""));

    // an export: who received it is the requestor, the media never is
    let export = one("export").to_xml();
    assert!(export.contains("EventActionCode=\"R\"") && export.contains("csd-code=\"110106\""));
    assert!(export.contains("UserID=\"/studies/abc/archive\" UserIsRequestor=\"false\" NetworkAccessPointID=\"192.0.2.7\" NetworkAccessPointTypeCode=\"2\"><RoleIDCode csd-code=\"110154\" codeSystemName=\"DCM\" originalText=\"Destination Media\"/>"));

    // a query: what was asked, in base64, and no patient
    let query = one("query").to_xml();
    assert!(query.contains("EventActionCode=\"E\"") && query.contains("csd-code=\"110112\""));
    assert!(query.contains("<ParticipantObjectIdentification ParticipantObjectID=\"/dicom-web/studies\" ParticipantObjectTypeCode=\"2\" ParticipantObjectTypeCodeRole=\"3\"><ParticipantObjectIDTypeCode csd-code=\"12\" codeSystemName=\"RFC-3881\" originalText=\"URI\"/><ParticipantObjectQuery>UGF0aWVudE5hbWU9RE9FKiZsaW1pdD0yNQ==</ParticipantObjectQuery>"));
    assert!(!query.contains("Patient Number"));
}

#[test]
fn the_events_about_remote_aes() {
    let hospital = "UserID=\"HOSPITAL\" AlternativeUserID=\"AETITLES=HOSPITAL\" UserIsRequestor=\"false\" NetworkAccessPointID=\"pacs.hospital.example\" NetworkAccessPointTypeCode=\"1\">";
    // a search of a remote AE: this server asks, the remote answers, the person is the requestor
    let query = one("query-of-a-remote-ae").to_xml();
    assert!(query.contains("EventOutcomeIndicator=\"8\""));
    let (asks, answers, person) = (
        query.find("AlternativeUserID=\"AETITLES=PACSRS\"").unwrap(),
        query.find(hospital).unwrap(),
        query.find("UserID=\"ada@pacs-rs\"").unwrap(),
    );
    assert!(asks < answers && answers < person, "{query}");
    assert!(query[asks..answers].contains("originalText=\"Source Role ID\""));
    assert!(query[answers..person].contains("originalText=\"Destination Role ID\""));
    // the person has no role in the exchange, and is who asked for it
    assert!(
        query[person..]
            .starts_with("UserID=\"ada@pacs-rs\" UserName=\"ada\" UserIsRequestor=\"true\"")
    );
    assert!(
        !query[person..query.find("<AuditSourceIdentification").unwrap()].contains("RoleIDCode")
    );

    // a move: the remote AE will send, the server receive; the study, and a patient not known yet
    let begin = one("begin-transferring").to_xml();
    assert!(begin.contains("EventActionCode=\"E\"") && begin.contains("csd-code=\"110102\" codeSystemName=\"DCM\" originalText=\"Begin Transferring DICOM Instances\""));
    let (sends, receives) = (
        begin.find(hospital).unwrap(),
        begin.find("AETITLES=RESEARCH").unwrap(),
    );
    assert!(sends < receives);
    assert!(begin[sends..receives].contains("originalText=\"Source Role ID\""));
    assert!(
        begin.contains("ParticipantObjectID=\"1.2.3\"")
            && begin.contains("ParticipantObjectID=\"unknown\"")
    );

    // a node that was not let in
    let refused = one("node-refused").to_xml();
    assert!(
        refused.contains("EventOutcomeIndicator=\"4\"")
            && refused.contains("originalText=\"Node Authentication\"")
    );
    assert!(refused.contains("<ParticipantObjectIdentification ParticipantObjectID=\"203.0.113.9\" ParticipantObjectTypeCode=\"2\" ParticipantObjectTypeCodeRole=\"13\"><ParticipantObjectIDTypeCode csd-code=\"110182\" codeSystemName=\"DCM\" originalText=\"Node ID\"/><ParticipantObjectName>203.0.113.9</ParticipantObjectName><ParticipantObjectDetail type=\"Alert Description\""));
}
