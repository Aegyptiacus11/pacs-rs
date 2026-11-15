//! Every kind of message this crate builds, validated against DICOM's schema (PS3.15
//! A.5.1.1) by `xmllint`, and their exact text for one of them. The validation runs when
//! `PACS_TEST_XMLLINT` names the program.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pacs_audit::events::{self, kind};
use pacs_audit::syslog::{self, Sender};
use pacs_audit::{AccessPoint, Message, Outcome, Participant, Source};

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

/// One of each event, with values that would break a careless writer.
fn messages() -> Vec<(&'static str, Message)> {
    let hostile = "<script>&\"quotes\" 'and'\nlines\t\u{0}";
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
    let good = messages().remove(2).1.to_xml();
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
    let message = messages().remove(2).1;
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
    let refused = messages().remove(3).1;
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
    let refused = messages().remove(3).1.to_xml();
    assert!(refused.contains(
        "UserID=\"&lt;script&gt;&amp;&quot;quotes&quot; &apos;and&apos;&#10;lines&#9;\u{FFFD}\""
    ));
}

#[test]
fn an_alert_names_its_subject_and_says_what_happened() {
    let alert = messages().remove(5).1.to_xml();
    // "<script>&..." in base64: the detail is never read as XML
    assert!(
        alert.contains("<ParticipantObjectDetail type=\"Alert Description\" value=\"PHNjcmlwdD4m")
    );
    assert!(alert.contains("ParticipantObjectID=\"/users/7f3a\" ParticipantObjectTypeCode=\"2\" ParticipantObjectTypeCodeRole=\"13\""));
    assert!(alert.contains("<ParticipantObjectIDTypeCode csd-code=\"12\" codeSystemName=\"RFC-3881\" originalText=\"URI\"/>"));
    // the reporter, then the person who did it, who is not the requestor of an alert
    assert!(alert.contains("<ActiveParticipant UserID=\"pacs-server@host\" UserIsRequestor=\"false\"/><ActiveParticipant UserID=\"ada@pacs-rs\" UserName=\"ada\" UserIsRequestor=\"false\""));
    let own = messages().remove(6).1.to_xml();
    assert!(
        own.contains("<ActiveParticipant UserID=\"pacs-server@host\" UserIsRequestor=\"true\"/>")
    );
}
