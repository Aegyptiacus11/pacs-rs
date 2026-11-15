//! A [`Message`] as XML, by the schema of PS3.15 A.5.1: elements in the schema's order, no
//! empty element and no empty attribute (A.5.2), on one line.

use base64::Engine;

use crate::{AccessPoint, Code, Message, NameOrQuery, Object, Participant, rfc3339};

/// Text for an attribute or an element: the five characters XML reserves, and line breaks
/// and tabs as references so that a message stays on one line. A character XML 1.0 cannot
/// hold at all (a control character) becomes U+FFFD.
fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
}

/// ` name="value"`; nothing for an empty value, which the profile forbids.
fn attribute(name: &str, value: &str, out: &mut String) {
    if value.is_empty() {
        return;
    }
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    escape(value, out);
    out.push('"');
}

fn coded(element: &str, code: &Code, out: &mut String) {
    out.push('<');
    out.push_str(element);
    attribute("csd-code", code.code, out);
    attribute("codeSystemName", code.system, out);
    attribute("originalText", code.text, out);
    out.push_str("/>");
}

fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn participant(participant: &Participant, out: &mut String) {
    out.push_str("<ActiveParticipant");
    attribute("UserID", &participant.user_id, out);
    if let Some(id) = &participant.alternative_user_id {
        attribute("AlternativeUserID", id, out);
    }
    if let Some(name) = &participant.user_name {
        attribute("UserName", name, out);
    }
    let requestor = if participant.is_requestor {
        "true"
    } else {
        "false"
    };
    attribute("UserIsRequestor", requestor, out);
    match &participant.access_point {
        Some(AccessPoint::Machine(name)) if !name.is_empty() => {
            attribute("NetworkAccessPointID", name, out);
            attribute("NetworkAccessPointTypeCode", "1", out);
        }
        Some(AccessPoint::Ip(address)) if !address.is_empty() => {
            attribute("NetworkAccessPointID", address, out);
            attribute("NetworkAccessPointTypeCode", "2", out);
        }
        _ => {}
    }
    if participant.roles.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for role in &participant.roles {
        coded("RoleIDCode", role, out);
    }
    out.push_str("</ActiveParticipant>");
}

fn object(object: &Object, out: &mut String) {
    out.push_str("<ParticipantObjectIdentification");
    attribute("ParticipantObjectID", &object.id, out);
    for (name, value) in [
        ("ParticipantObjectTypeCode", object.type_code),
        ("ParticipantObjectTypeCodeRole", object.role),
        ("ParticipantObjectDataLifeCycle", object.life_cycle),
    ] {
        if let Some(value) = value {
            attribute(name, value, out);
        }
    }
    out.push('>');
    coded("ParticipantObjectIDTypeCode", &object.id_type, out);
    match &object.name {
        NameOrQuery::Name(name) => {
            out.push_str("<ParticipantObjectName>");
            escape(name, out);
            out.push_str("</ParticipantObjectName>");
        }
        NameOrQuery::Query(query) => {
            out.push_str("<ParticipantObjectQuery>");
            out.push_str(&base64(query));
            out.push_str("</ParticipantObjectQuery>");
        }
    }
    for (kind, value) in &object.details {
        out.push_str("<ParticipantObjectDetail");
        attribute("type", kind, out);
        attribute("value", &base64(value), out);
        out.push_str("/>");
    }
    if !object.sop_classes.is_empty() {
        out.push_str("<ParticipantObjectDescription>");
        for class in &object.sop_classes {
            out.push_str("<SOPClass");
            attribute("UID", &class.uid, out);
            attribute("NumberOfInstances", &class.instances.to_string(), out);
            out.push_str("/>");
        }
        out.push_str("</ParticipantObjectDescription>");
    }
    out.push_str("</ParticipantObjectIdentification>");
}

impl Message {
    /// The message as one line of XML, without a declaration: UTF-8 is XML's default, and
    /// what ITI-20 asks for.
    pub fn to_xml(&self) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str("<AuditMessage><EventIdentification");
        if let Some(action) = self.action {
            attribute("EventActionCode", action.letter(), &mut out);
        }
        attribute("EventDateTime", &rfc3339(self.time), &mut out);
        attribute("EventOutcomeIndicator", self.outcome.number(), &mut out);
        out.push('>');
        coded("EventID", &self.event_id, &mut out);
        for kind in &self.event_types {
            coded("EventTypeCode", kind, &mut out);
        }
        if let Some(description) = self.outcome_description.as_deref()
            && !description.is_empty()
        {
            out.push_str("<EventOutcomeDescription>");
            escape(description, &mut out);
            out.push_str("</EventOutcomeDescription>");
        }
        out.push_str("</EventIdentification>");
        for one in &self.participants {
            participant(one, &mut out);
        }
        out.push_str("<AuditSourceIdentification");
        if let Some(site) = &self.source.enterprise_site_id {
            attribute("AuditEnterpriseSiteID", site, &mut out);
        }
        attribute("AuditSourceID", &self.source.id, &mut out);
        out.push_str("><AuditSourceTypeCode");
        attribute("csd-code", self.source.type_code, &mut out);
        out.push_str("/></AuditSourceIdentification>");
        for one in &self.objects {
            object(one, &mut out);
        }
        out.push_str("</AuditMessage>");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_and_stays_on_one_line() {
        let mut out = String::new();
        escape("a<b>&\"c'\td\ne\rf\u{0}g\u{1F}é", &mut out);
        assert_eq!(
            out,
            "a&lt;b&gt;&amp;&quot;c&apos;&#9;d&#10;e&#13;f\u{FFFD}g\u{FFFD}é"
        );
    }

    #[test]
    fn an_empty_attribute_is_left_out() {
        let mut out = String::new();
        attribute("UserName", "", &mut out);
        attribute("UserID", "ada", &mut out);
        assert_eq!(out, " UserID=\"ada\"");
    }
}
