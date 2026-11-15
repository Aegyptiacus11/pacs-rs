//! The audit trail, when `PACS_AUDIT_FILE` names a file: who signed in, who changed an
//! account, a role or a tool, what a role refused, and when the server started and
//! stopped. Apart from the application log, and in the format a hospital's audit
//! repository reads.
//!
//! Each event is one line of the file: the syslog message of IHE's Record Audit Event
//! [ITI-20] (RFC 5424, MSGID `IHE+RFC-3881`) whose content is the DICOM audit message of
//! PS3.15 A.5, built by the `pacs-audit` crate. A line is written before the answer to
//! the request leaves, so the file never lacks an event a client saw succeed.
//!
//! Recorded now:
//!
//! | Event | When |
//! |---|---|
//! | Application Activity | the server starts, and stops |
//! | User Authentication, Login | `POST /auth/login` and a tool's `POST /auth/token`, accepted or refused |
//! | User Authentication, Logout | `POST /auth/logout` and `/auth/logout-all` |
//! | Security Alert, User Security Attributes Changed | a user or a tool is made, changed or removed; a password is changed |
//! | Security Alert, Security Roles Changed | a role is made, changed or removed |
//! | Security Alert, Access Control Decision | a request the caller's role or scope does not allow |
//!
//! Not yet: what is read, stored, exported and deleted (the events that name a study and
//! its patient), and sending the lines to a repository over TLS.
//!
//! A person is `login@<source>`; a tool is its ID, with the name ITI-72 (3.72.5.1) gives
//! a caller that came with a JWT: `audience<subject@issuer>`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::Context;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use pacs_audit::events::{self, access_point, kind};
use pacs_audit::syslog::{self, Sender};
use pacs_audit::{Code, Message, Outcome, Participant, Source};

use crate::Settings;
use crate::auth::{SignedIn, basic_credentials};

/// CID 403: a decision about access, here a refusal.
const ACCESS_CONTROL_DECISION: Code = Code::dcm("110147", "Access Control Decision");
/// A sign-in body is a login and a password: nothing near this.
const MAX_LOGIN_BODY_BYTES: usize = 64 * 1024;

struct Trail {
    file: Mutex<File>,
    source: Source,
    sender: Sender,
    /// The server as its own participant: `pacs-server@<host>`.
    process: String,
    /// `PACS_AUTH_ISSUER`: the issuer and audience of a tool's token.
    issuer: String,
}

/// The audit trail of a running server; does nothing when there is none.
#[derive(Clone, Default)]
pub struct Audit(Option<Arc<Trail>>);

fn hostname() -> String {
    let named = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_default();
    named.trim().to_owned()
}

/// Open the trail the settings ask for: [`Audit::off`] without `PACS_AUDIT_FILE`.
///
/// The file is opened for appending, and made readable by its owner alone when it is
/// new. A file that cannot be opened stops the start: a server asked to keep a trail
/// does not run without one.
pub fn open_audit(settings: &Settings) -> anyhow::Result<Audit> {
    let Some(path) = &settings.audit_file else {
        return Ok(Audit::off());
    };
    let source = settings.audit_source_id.trim();
    if source.is_empty() || source.chars().any(|c| c.is_whitespace() || c.is_control()) {
        anyhow::bail!("PACS_AUDIT_SOURCE_ID: a name without spaces, e.g. pacs-rs");
    }
    let file = open_for_append(path).with_context(|| {
        format!(
            "PACS_AUDIT_FILE ({}): opening the audit trail",
            path.display()
        )
    })?;
    let host = hostname();
    Ok(Audit(Some(Arc::new(Trail {
        file: Mutex::new(file),
        source: Source::application_server(source),
        process: if host.is_empty() {
            "pacs-server".into()
        } else {
            format!("pacs-server@{host}")
        },
        sender: Sender {
            hostname: host,
            app_name: "pacs-rs".into(),
            process_id: std::process::id().to_string(),
        },
        issuer: settings.auth_issuer.trim().to_owned(),
    }))))
}

fn open_for_append(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // for a file this call creates; an existing one keeps its mode
        options.mode(0o600);
    }
    options.open(path)
}

impl Audit {
    /// No trail: every call returns at once.
    pub fn off() -> Self {
        Audit(None)
    }

    pub fn is_on(&self) -> bool {
        self.0.is_some()
    }

    /// Append one message. A line that cannot be written is an error in the application
    /// log, with the event's kind: the request it reports has already happened.
    fn record(&self, message: Message) {
        let Some(trail) = &self.0 else { return };
        let mut line = syslog::line(&message, &trail.sender, SystemTime::now());
        line.push('\n');
        // one write per line: appends of whole lines do not interleave
        let written = match trail.file.lock() {
            Ok(mut file) => file.write_all(line.as_bytes()),
            Err(poisoned) => poisoned.into_inner().write_all(line.as_bytes()),
        };
        if let Err(error) = written {
            tracing::error!(event = message.event_id.text, %error, "an audit message was not written");
        }
    }

    /// The server started (`true`) or is stopping; `ae_titles` are its DICOM AE titles
    /// when DIMSE is on.
    pub fn application(&self, started: bool, ae_titles: &[String]) {
        let Some(trail) = &self.0 else { return };
        self.record(events::application_activity(
            &trail.source,
            SystemTime::now(),
            started,
            &trail.process,
            ae_titles,
        ));
        if !started
            && let Ok(file) = trail.file.lock()
            && let Err(error) = file.sync_all()
        {
            tracing::error!(%error, "the audit trail was not flushed to disk");
        }
    }

    /// Put the trail around `app`: outside the guard, so that refusals are seen too.
    pub fn observe(&self, app: axum::Router) -> axum::Router {
        if !self.is_on() {
            return app;
        }
        app.layer(axum::middleware::from_fn_with_state(self.clone(), observe))
    }
}

/// Who a request came from, as an audit participant.
fn caller(trail: &Trail, user: &SignedIn, ip: Option<IpAddr>) -> Participant {
    let participant = if user.client {
        // ITI TF-2: 3.72.5.1, a caller with a JWT: alias<user@issuer>, the alias being
        // the token's audience
        Participant::new(&user.id, true).named(format!("{0}<{1}@{0}>", trail.issuer, user.id))
    } else {
        Participant::new(format!("{}@{}", user.login, trail.source.id), true).named(&user.login)
    };
    participant.at(access_point(ip))
}

/// Someone who has only claimed a name: a login typed, or a tool's ID sent.
fn claimed(trail: &Trail, name: &str, person: bool, ip: Option<IpAddr>) -> Participant {
    let name = if name.is_empty() { "unknown" } else { name };
    let id = if person {
        format!("{name}@{}", trail.source.id)
    } else {
        name.to_owned()
    };
    // the table makes the access point mandatory: without a socket address (a test, a
    // proxy that hides it), the server's own name
    let at = access_point(ip)
        .unwrap_or_else(|| pacs_audit::AccessPoint::Machine(trail.source.id.clone()));
    Participant::new(id, true).at(Some(at))
}

fn outcome_of(status: StatusCode) -> Outcome {
    if status.is_success() {
        Outcome::Success
    } else if status.is_server_error() {
        Outcome::SeriousFailure
    } else {
        Outcome::MinorFailure
    }
}

/// The login a sign-in body names, when it is JSON with an `email`.
fn login_of(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("email")?.as_str().map(str::to_lowercase))
        .unwrap_or_default()
}

/// What changed, for the routes that change accounts: the alert's kind and its words.
fn account_change(method: &Method, path: &str) -> Option<(Code, String)> {
    if !matches!(*method, Method::POST | Method::PUT | Method::DELETE) {
        return None;
    }
    let done = match *method {
        Method::POST => "made",
        Method::PUT => "changed",
        _ => "removed",
    };
    let under = |prefix: &str| {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    };
    if under("/users") {
        Some((
            kind::USER_SECURITY_ATTRIBUTES_CHANGED,
            format!("a user was {done}"),
        ))
    } else if under("/clients") {
        Some((
            kind::USER_SECURITY_ATTRIBUTES_CHANGED,
            format!("a tool was {done}"),
        ))
    } else if under("/roles") {
        Some((kind::SECURITY_ROLES_CHANGED, format!("a role was {done}")))
    } else if path == "/auth/password" && *method == Method::POST {
        Some((
            kind::USER_SECURITY_ATTRIBUTES_CHANGED,
            "a user changed their own password".into(),
        ))
    } else {
        None
    }
}

/// Around every request: after the answer is made, write the event it amounts to, if any.
async fn observe(State(audit): State<Audit>, request: Request, next: Next) -> Response {
    let Some(trail) = audit.0.clone() else {
        return next.run(request).await;
    };
    let (method, path) = (request.method().clone(), request.uri().path().to_owned());
    let ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| address.ip());
    let posted = |wanted: &str| method == Method::POST && path == wanted;

    // Who is asking to sign in is in the request, which the route consumes.
    let mut claimed_name = None;
    let request = if posted("/auth/login") {
        let (parts, body) = request.into_parts();
        let Ok(bytes) = axum::body::to_bytes(body, MAX_LOGIN_BODY_BYTES).await else {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        };
        claimed_name = Some(login_of(&bytes));
        Request::from_parts(parts, Body::from(bytes))
    } else {
        if posted("/auth/token") {
            let id = basic_credentials(request.headers().get(AUTHORIZATION)).map(|(id, _)| id);
            claimed_name = Some(id.unwrap_or_default());
        }
        request
    };

    let response = next.run(request).await;
    let status = response.status();
    let user = response.extensions().get::<SignedIn>();
    let now = SystemTime::now();
    let said = || (!status.is_success()).then(|| format!("HTTP {}", status.as_u16()));

    if let Some(name) = claimed_name {
        audit.record(events::user_authentication(
            &trail.source,
            now,
            true,
            claimed(&trail, &name, path == "/auth/login", ip),
            &trail.process,
            outcome_of(status),
            said(),
        ));
    } else if posted("/auth/logout") || posted("/auth/logout-all") {
        // a session ended by its cookie alone does not say whose it was
        let who = match user {
            Some(user) => caller(&trail, user, ip),
            None => claimed(&trail, "", true, ip),
        };
        let who = who.at(Some(access_point(ip).unwrap_or_else(|| {
            pacs_audit::AccessPoint::Machine(trail.source.id.clone())
        })));
        audit.record(events::user_authentication(
            &trail.source,
            now,
            false,
            who,
            &trail.process,
            outcome_of(status),
            said(),
        ));
    } else if status == StatusCode::FORBIDDEN
        || (status == StatusCode::UNAUTHORIZED && user.is_some())
    {
        // refused by the guard for what the caller is, not for a missing or bad token
        if let Some(user) = user {
            audit.record(events::security_alert(
                &trail.source,
                now,
                ACCESS_CONTROL_DECISION,
                Outcome::MinorFailure,
                &trail.process,
                Some(caller(&trail, user, ip)),
                &path,
                &format!("{method} {path} refused: the caller's role or scope does not allow it"),
            ));
        }
    } else if status.is_success()
        && let Some((alert, what)) = account_change(&method, &path)
    {
        audit.record(events::security_alert(
            &trail.source,
            now,
            alert,
            Outcome::Success,
            &trail.process,
            user.map(|user| caller(&trail, user, ip)),
            &path,
            &what,
        ));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_requests_change_accounts() {
        let change =
            |method: Method, path: &str| account_change(&method, path).map(|(c, w)| (c.code, w));
        assert_eq!(
            change(Method::POST, "/users"),
            Some(("110137", "a user was made".into()))
        );
        assert_eq!(
            change(Method::PUT, "/users/u1"),
            Some(("110137", "a user was changed".into()))
        );
        assert_eq!(
            change(Method::DELETE, "/clients/c1"),
            Some(("110137", "a tool was removed".into()))
        );
        assert_eq!(
            change(Method::PUT, "/roles/reader"),
            Some(("110136", "a role was changed".into()))
        );
        assert_eq!(
            change(Method::POST, "/auth/password"),
            Some(("110137", "a user changed their own password".into()))
        );
        // reading changes nothing, and neither does a route that only looks like one
        for (method, path) in [
            (Method::GET, "/users"),
            (Method::GET, "/roles"),
            (Method::POST, "/usersx"),
            (Method::POST, "/studies"),
            (Method::DELETE, "/studies/s1"),
            (Method::PUT, "/auth/password"),
        ] {
            assert_eq!(change(method.clone(), path), None, "{method} {path}");
        }
    }

    #[test]
    fn the_login_of_a_sign_in_body_and_outcomes() {
        assert_eq!(login_of(br#"{"email":"Ada","password":"x"}"#), "ada");
        assert_eq!(login_of(br#"{"password":"x"}"#), "");
        assert_eq!(login_of(b"not json"), "");
        assert_eq!(login_of(br#"{"email":7}"#), "");
        assert_eq!(outcome_of(StatusCode::OK), Outcome::Success);
        assert_eq!(outcome_of(StatusCode::NO_CONTENT), Outcome::Success);
        assert_eq!(outcome_of(StatusCode::UNAUTHORIZED), Outcome::MinorFailure);
        assert_eq!(
            outcome_of(StatusCode::INTERNAL_SERVER_ERROR),
            Outcome::SeriousFailure
        );
    }
}
