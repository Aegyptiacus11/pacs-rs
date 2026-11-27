//! The audit trail, when `PACS_AUDIT_FILE` names a file: who signed in, who changed an
//! account, a role or a tool, what a role refused, and when the server started and
//! stopped. Apart from the application log, and in the format a hospital's audit
//! repository reads.
//!
//! Each event is one line of the file: the syslog message of IHE's Record Audit Event
//! [ITI-20] (RFC 5424, MSGID `IHE+RFC-3881`) whose content is the DICOM audit message of
//! PS3.15 A.5, built by the `pacs-audit` crate. A line is written before the answer to
//! the request leaves, so the file does not lack an event a client saw succeed, but when
//! the write itself fails: the request has happened by then and is not undone, and the
//! failure is an error in the application log, with the event's kind.
//!
//! Recorded now:
//!
//! | Event | When |
//! |---|---|
//! | Application Activity | the server starts, and stops |
//! | User Authentication, Login | `POST /auth/login`, a sign-in through the OpenID provider (`GET /auth/oidc/callback`) and a tool's `POST /auth/token`, accepted or refused |
//! | User Authentication, Logout | `POST /auth/logout` and `/auth/logout-all` |
//! | Security Alert, User Security Attributes Changed | a user or a tool is made, changed or removed; a password is changed |
//! | Security Alert, Security Roles Changed | a role is made, changed or removed |
//! | Security Alert, Access Control Decision | a request the caller's role or scope does not allow |
//! | DICOM Instances Transferred | instances received (REST upload, STOW-RS), and instances, frames or images sent to a caller |
//! | DICOM Instances Accessed | a study's attributes, lists or measurements read; measurements replaced, a label or a metadata value put or taken off; part of a study deleted |
//! | DICOM Study Deleted | a study deleted whole, alone or with its patient |
//! | Export | a ZIP archive or a media file-set downloaded |
//! | Query | a search: QIDO-RS, the REST API's lists and `/tools/find`, the web interface's lists |
//!
//! The events about data name the study and its patient, and are written for requests
//! that succeeded. Which request is which is in `audit_routes`. Reading one study takes
//! many requests (a CT is hundreds of frames): the same caller reading, being sent or
//! searching the same thing again within [`REPEAT_SECONDS`] leaves no second line, as the
//! profile allows (A.5.3: a message is not required each time the activity occurs).
//! What is stored, deleted or exported is written every time.
//!
//! DIMSE and the remote AEs:
//!
//! | Event | When |
//! |---|---|
//! | DICOM Instances Transferred | an association is over: one line per study stored through it by C-STORE, from the calling AE title to the called one |
//! | Security Alert, Node Authentication | a connection that did not become an association: an AE title not recognized, the limit reached |
//! | Query, of this server | a C-FIND it answered: the calling AE asks, with the Information Model as the SOP Class queried, the Identifier as sent and its transfer syntax |
//! | DICOM Instances Transferred, from this server | a C-MOVE or a C-GET it answered: one line per study of what the destination stored, from the called AE title to the Move Destination, the calling AE as the requestor; for a C-GET the calling AE is the destination |
//! | Security Alert, Access Control Decision | a C-FIND, a C-MOVE or a C-GET from an AE this server does not answer |
//! | Query | a remote AE searched by C-FIND (`GET /modalities/{id}/studies`): this server asks, the remote AE answers, the caller is the requestor |
//! | Begin Transferring DICOM Instances | a remote AE asked by C-MOVE to send studies here (`POST /modalities/{id}/move`), one line per study; their arrival is the first event of this table |
//!
//! A study that arrives at a project's AE title is de-identified before it is stored: its
//! line names the study and the patient as the archive holds them, the pseudonyms.
//!
//! Not yet: sending the lines to a repository over TLS.
//!
//! A person is `login@<source>`; a tool is its ID, with the name ITI-72 (3.72.5.1) gives
//! a caller that came with a JWT: `audience<subject@issuer>`.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use pacs_archive::observed::{Stored, stored_during};
use pacs_audit::events::{self, access_point, kind};
use pacs_audit::syslog::{self, Sender};
use pacs_audit::{AccessPoint, Action, Code, Message, Outcome, Participant, SopClass, Source};
use pacs_index::SeenBy;
use pacs_index::{AuditedStudy, Index, Level};

use crate::Settings;
use crate::audit_routes::{Subject, Target, subject};
use crate::auth::{SignedIn, basic_credentials};

/// CID 403: a decision about access, here a refusal.
const ACCESS_CONTROL_DECISION: Code = Code::dcm("110147", "Access Control Decision");
/// A sign-in body is a login and a password: nothing near this.
const MAX_LOGIN_BODY_BYTES: usize = 64 * 1024;
/// A search or a list of resources to archive, as JSON: read here to be recorded.
const MAX_QUERY_BODY_BYTES: usize = 1024 * 1024;
/// How long the same caller reading or searching the same thing again leaves no new line.
pub const REPEAT_SECONDS: u64 = 15 * 60;
/// Past this many remembered reads, the old ones are dropped before a new one is kept.
const MAX_REMEMBERED: usize = 20_000;

struct Trail {
    file: Mutex<File>,
    source: Source,
    sender: Sender,
    /// The server as its own participant: `pacs-server@<host>`.
    process: String,
    /// `PACS_AUTH_ISSUER`: the issuer and audience of a tool's token.
    issuer: String,
    /// What was read or searched lately, and when: see [`REPEAT_SECONDS`].
    recent: Mutex<HashMap<String, Instant>>,
    /// Told of each line written, for who sends the file on (`audit_forward`).
    written: Arc<tokio::sync::Notify>,
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
        recent: Mutex::new(HashMap::new()),
        written: Arc::new(tokio::sync::Notify::new()),
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
        trail.written.notify_one();
    }

    /// Notified after each line is written; none without a trail.
    pub fn written(&self) -> Option<Arc<tokio::sync::Notify>> {
        self.0.as_ref().map(|trail| trail.written.clone())
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

    /// Index entries the consistency repair removed, their files being gone: a deletion by
    /// the server itself, as `DELETE` by a person would be recorded. `studies` are those the
    /// entries were in, read before; `index` says which of them are gone entirely.
    pub async fn removed_by_repair(
        &self,
        index: &pacs_index::Index,
        studies: &[AuditedStudy],
    ) -> Result<(), pacs_index::Error> {
        let Some(trail) = &self.0 else { return Ok(()) };
        let now = SystemTime::now();
        let who = Participant::new(&trail.process, true);
        let (mut gone, mut cut) = (Vec::new(), Vec::new());
        for study in studies {
            if index
                .audited_study(&study.study_instance_uid)
                .await?
                .is_some()
            {
                cut.push(described(study, false));
            } else {
                gone.push(described(study, true));
            }
        }
        for group in by_patient(gone) {
            self.record(events::study_deleted(
                &trail.source,
                now,
                Outcome::Success,
                vec![who.clone()],
                &group,
            ));
        }
        for group in by_patient(cut) {
            self.record(events::instances_accessed(
                &trail.source,
                now,
                Action::Delete,
                Outcome::Success,
                vec![who.clone()],
                &group,
            ));
        }
        Ok(())
    }

    /// Put the trail around `app`: outside the guard, so that refusals are seen too.
    /// `index` says whose study a request touched, and `mount` is the DICOMweb root.
    /// `dimse` names the remote AEs and this server's own AE title.
    pub fn observe(
        &self,
        app: axum::Router,
        index: Index,
        mount: &str,
        dimse: pacs_dimse::Config,
        projects: Vec<(String, pacs_dimse::AeTitle)>,
    ) -> axum::Router {
        if !self.is_on() {
            return app;
        }
        let observer = Observer {
            audit: self.clone(),
            index,
            mount: mount.into(),
            dimse: Arc::new(dimse),
            projects: projects.into(),
        };
        app.layer(axum::middleware::from_fn_with_state(observer, observe))
    }
}

impl Trail {
    /// Whether `what` was noted less than [`REPEAT_SECONDS`] ago; notes it now if not.
    fn repeated(&self, what: String, now: Instant) -> bool {
        let window = Duration::from_secs(REPEAT_SECONDS);
        let mut recent = match self.recent.lock() {
            Ok(recent) => recent,
            Err(poisoned) => poisoned.into_inner(),
        };
        if recent
            .get(&what)
            .is_some_and(|last| now.duration_since(*last) < window)
        {
            return true;
        }
        if recent.len() >= MAX_REMEMBERED {
            recent.retain(|_, last| now.duration_since(*last) < window);
            // still full of fresh entries: forget them all rather than grow; the cost is
            // a line written twice
            if recent.len() >= MAX_REMEMBERED {
                recent.clear();
            }
        }
        recent.insert(what, now);
        false
    }
}

/// The trail with what it needs to say whose data a request touched.
#[derive(Clone)]
struct Observer {
    audit: Audit,
    index: Index,
    mount: Arc<str>,
    dimse: Arc<pacs_dimse::Config>,
    /// Project names and their AE titles: where a move may send what it brings.
    projects: Arc<[(String, pacs_dimse::AeTitle)]>,
}

/// An application that speaks DICOM, by its AE title (A.5.2.2).
fn application(ae_title: &str, at: Option<AccessPoint>) -> Participant {
    Participant::new(ae_title, false)
        .alternative(format!("AETITLES={ae_title}"))
        .at(at)
}

impl std::fmt::Debug for Audit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.is_on() {
            "Audit(on)"
        } else {
            "Audit(off)"
        })
    }
}

impl Audit {
    /// The alert for an operation asked by an AE this server does not answer: an access
    /// violation. A caller that keeps asking is one line a window.
    fn not_answered(
        &self,
        trail: &Trail,
        peer: &pacs_dimse::Peer,
        asker: Participant,
        operation: &str,
        sop_class: &str,
    ) {
        let again = format!("{operation} refused|{}|{}", peer.address.ip(), peer.calling);
        if trail.repeated(again, Instant::now()) {
            return;
        }
        self.record(events::security_alert(
            &trail.source,
            SystemTime::now(),
            ACCESS_CONTROL_DECISION,
            Outcome::MinorFailure,
            &trail.process,
            Some(asker),
            &format!("urn:oid:{sop_class}"),
            &format!(
                "{operation} refused: {} is not an AE this server answers the queries and retrievals of",
                peer.calling
            ),
        ));
    }
}

/// What DIMSE associations did: see the module's second table.
impl pacs_dimse::Observer for Audit {
    fn queried(&self, peer: &pacs_dimse::Peer, query: &pacs_dimse::Queried<'_>) {
        let Some(trail) = &self.0 else { return };
        let now = SystemTime::now();
        let node = peer.address.ip().to_string();
        let mut asker = application(&peer.calling, Some(AccessPoint::Ip(node.clone())));
        if query.status == pacs_dimse::Status::NOT_AUTHORIZED {
            // an access violation is an alert, not a query (A.5.3.10, note)
            self.not_answered(trail, peer, asker, "C-FIND", query.sop_class);
            return;
        }
        use pacs_dimse::{Status, StatusClass};
        let outcome = match query.status.class() {
            StatusClass::Success | StatusClass::Cancel => Outcome::Success,
            // the server's own failure, not a query it could not accept
            _ if matches!(
                query.status,
                Status::UNABLE_TO_PROCESS_INDEX | Status::FIND_OUT_OF_RESOURCES
            ) =>
            {
                Outcome::SeriousFailure
            }
            _ => Outcome::MinorFailure,
        };
        // the same search by the same caller again is not a second line
        let mut again = format!(
            "query|{node}|{}|{}|{}|",
            peer.calling, peer.called, query.sop_class
        );
        again.extend(query.identifier.iter().map(|byte| format!("{byte:02x}")));
        if outcome == Outcome::Success && trail.repeated(again, Instant::now()) {
            return;
        }
        asker.is_requestor = true;
        let answerer = Participant::new(&trail.process, false)
            .alternative(format!("AETITLES={}", peer.called));
        self.record(events::dicom_query(
            &trail.source,
            now,
            outcome,
            asker,
            answerer,
            query.sop_class,
            query.transfer_syntax,
            query.identifier,
        ));
    }

    fn retrieved(&self, peer: &pacs_dimse::Peer, retrieval: &pacs_dimse::Retrieved<'_>) {
        let Some(trail) = &self.0 else { return };
        let now = SystemTime::now();
        let node = peer.address.ip().to_string();
        let mut asker = application(&peer.calling, Some(AccessPoint::Ip(node)));
        if retrieval.status == pacs_dimse::Status::NOT_AUTHORIZED {
            self.not_answered(trail, peer, asker, retrieval.operation, retrieval.sop_class);
            return;
        }
        asker.is_requestor = true;
        // a C-GET sends to who asks, on its own association; a C-MOVE to a third AE
        let to_the_asker = retrieval.destination_host.is_none();
        // what the destination stored: this server sent it, for the AE that asked
        for study in retrieval.studies {
            let sender = Participant::new(&trail.process, false)
                .alternative(format!("AETITLES={}", peer.called));
            let receiver = if to_the_asker {
                asker.clone()
            } else {
                application(
                    retrieval.destination,
                    retrieval
                        .destination_host
                        .map(|host| AccessPoint::Machine(host.to_owned())),
                )
            };
            let mut message = events::instances_transferred(
                &trail.source,
                now,
                Action::Read,
                Outcome::Success,
                sender,
                receiver,
                &events::Study {
                    study_instance_uid: study.study_instance_uid.clone(),
                    patient_id: study.patient_id.clone(),
                    patient_name: study.patient_name.clone(),
                    sop_classes: study
                        .sop_classes
                        .iter()
                        .map(|(uid, instances)| SopClass {
                            uid: uid.clone(),
                            instances: u64::from(*instances),
                        })
                        .collect(),
                },
            );
            if !to_the_asker {
                // "other participants that are known, especially third parties that are
                // the requestor" (A.5.3.7): the AE that asked for the move
                message.participants.push(asker.clone());
            }
            self.record(message);
        }
    }

    fn received(&self, peer: &pacs_dimse::Peer, stored: Vec<Stored>) {
        let Some(trail) = &self.0 else { return };
        let now = SystemTime::now();
        let at = Some(AccessPoint::Ip(peer.address.ip().to_string()));
        for (study, action) in stored_studies(stored) {
            // a C-STORE is pushed: the sender asked for it
            let mut sender = application(&peer.calling, at.clone());
            sender.is_requestor = true;
            let receiver = Participant::new(&trail.process, false)
                .alternative(format!("AETITLES={}", peer.called));
            self.record(events::instances_transferred(
                &trail.source,
                now,
                action,
                Outcome::Success,
                sender,
                receiver,
                &study,
            ));
        }
    }

    fn refused(&self, address: SocketAddr, reason: &str) {
        let Some(trail) = &self.0 else { return };
        let node = address.ip().to_string();
        // a scanner, or a peer that keeps retrying, is one line a window
        if trail.repeated(format!("refused|{node}|{reason}"), Instant::now()) {
            return;
        }
        self.record(events::node_authentication_failure(
            &trail.source,
            SystemTime::now(),
            &trail.process,
            &node,
            &format!("a DIMSE connection did not become an association: {reason}"),
        ));
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

/// The study as an event describes it; with its instance counts when the event is about
/// all of it.
fn described(study: &AuditedStudy, whole: bool) -> events::Study {
    events::Study {
        study_instance_uid: study.study_instance_uid.clone(),
        patient_id: study.patient_id.clone(),
        patient_name: study.patient_name.clone(),
        sop_classes: if whole {
            study
                .sop_classes
                .iter()
                .map(|(uid, instances)| SopClass {
                    uid: uid.clone(),
                    instances: u64::try_from(*instances).unwrap_or(0),
                })
                .collect()
        } else {
            Vec::new()
        },
    }
}

/// `studies` in groups of one patient each, in the order met: a message is about one.
fn by_patient(studies: Vec<events::Study>) -> Vec<Vec<events::Study>> {
    let mut groups: Vec<Vec<events::Study>> = Vec::new();
    for study in studies {
        let same = |group: &Vec<events::Study>| {
            group[0].patient_id == study.patient_id && group[0].patient_name == study.patient_name
        };
        match groups.iter_mut().find(|group| same(group)) {
            Some(group) => group.push(study),
            None => groups.push(vec![study]),
        }
    }
    groups
}

/// What was stored, a study at a time: its patient, how many instances of each SOP class,
/// and whether the archive gained them (Create), replaced its copies (Update) or held
/// them already (Read).
fn stored_studies(stored: Vec<Stored>) -> Vec<(events::Study, Action)> {
    let mut studies: Vec<(events::Study, Action)> = Vec::new();
    for one in stored {
        let action = match one.outcome {
            pacs_archive::Outcome::Added => Action::Create,
            pacs_archive::Outcome::Replaced => Action::Update,
            pacs_archive::Outcome::AlreadyStored => Action::Read,
        };
        let position = studies
            .iter()
            .position(|(study, _)| study.study_instance_uid == one.study_instance_uid);
        let index = position.unwrap_or_else(|| {
            studies.push((
                events::Study {
                    study_instance_uid: one.study_instance_uid.clone(),
                    patient_id: one.patient_id.clone(),
                    patient_name: one.patient_name.clone(),
                    sop_classes: Vec::new(),
                },
                Action::Read,
            ));
            studies.len() - 1
        });
        let (study, strongest) = &mut studies[index];
        // one new instance makes the transfer a creation; else one replaced, an update
        if action == Action::Create || (action == Action::Update && *strongest == Action::Read) {
            *strongest = action;
        }
        match study
            .sop_classes
            .iter_mut()
            .find(|c| c.uid == one.sop_class_uid)
        {
            Some(class) => class.instances += 1,
            None => study.sop_classes.push(SopClass {
                uid: one.sop_class_uid,
                instances: 1,
            }),
        }
    }
    studies
}

/// The public IDs an archive made to order names: `?resources=a,b`, or `{"Resources": []}`.
fn listed_resources(query: Option<&str>, body: &[u8]) -> Vec<String> {
    let from_query = query
        .into_iter()
        .flat_map(|query| query.split('&'))
        .find_map(|pair| {
            let ids = pair.strip_prefix("resources=")?;
            Some(ids.split(',').map(str::to_owned).collect::<Vec<_>>())
        });
    let from_body = || {
        let value: serde_json::Value = serde_json::from_slice(body).ok()?;
        let ids = value.get("Resources")?.as_array()?;
        Some(
            ids.iter()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect(),
        )
    };
    let mut ids = from_query.or_else(from_body).unwrap_or_default();
    ids.retain(|id| !id.is_empty());
    ids
}

/// The studies `target` lies in, as the index has them now.
async fn studies_of(
    index: &Index,
    target: &Target,
    listed: &[String],
) -> Result<Vec<AuditedStudy>, pacs_index::Error> {
    match target {
        Target::Study(uid) => Ok(index.audited_study(uid).await?.into_iter().collect()),
        Target::Resource(level, id) => index.audited_by_public_id(*level, id).await,
        Target::Listed => {
            let mut studies: Vec<AuditedStudy> = Vec::new();
            for id in listed {
                // a public ID does not say its level
                for level in [Level::Study, Level::Series, Level::Instance, Level::Patient] {
                    let found = index.audited_by_public_id(level, id).await?;
                    if found.is_empty() {
                        continue;
                    }
                    for study in found {
                        if !studies
                            .iter()
                            .any(|s| s.study_instance_uid == study.study_instance_uid)
                        {
                            studies.push(study);
                        }
                    }
                    break;
                }
            }
            Ok(studies)
        }
    }
}

/// The studies a move asks for, and the AE title they are to arrive at: this server's own,
/// or the project's that the body names (`POST /modalities/{id}/move`).
fn moved(
    body: &[u8],
    dimse: &pacs_dimse::Config,
    projects: &[(String, pacs_dimse::AeTitle)],
) -> (Vec<String>, String) {
    let body: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    let studies = match body.get("StudyInstanceUID") {
        Some(serde_json::Value::String(uid)) => vec![uid.clone()],
        Some(serde_json::Value::Array(uids)) => uids
            .iter()
            .filter_map(|uid| uid.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    };
    let project = body.get("Project").and_then(|name| name.as_str());
    let destination = project
        .and_then(|name| projects.iter().find(|(n, _)| n == name))
        .map(|(_, ae_title)| ae_title.as_str())
        .unwrap_or(dimse.ae_title.as_str());
    (studies, destination.to_owned())
}

/// What the request did to the archive's data, written once it has succeeded.
struct Asked {
    subject: Subject,
    path: String,
    query: Option<String>,
    /// The body of a search or of an archive order.
    body: Vec<u8>,
    /// For a deletion: the studies the resource lay in, read before it went.
    doomed: Vec<AuditedStudy>,
}

/// What is said of a request refused for being outside its account's projects.
const OUTSIDE: &str = "outside the account's projects";

impl Observer {
    /// A request answered 404 to an account limited to some projects. When what it named
    /// is in the archive, outside those projects, the request was an attempt on a study the
    /// account may not see: written as the event it would have been, with a failure
    /// outcome and the studies it named. When nothing of that name exists, there is
    /// nothing to say, as for any other 404.
    async fn outside_events(
        &self,
        trail: &Trail,
        asked: Asked,
        account: &str,
        who: Participant,
        ip: Option<IpAddr>,
        now: SystemTime,
    ) -> Result<(), pacs_index::Error> {
        let (action, named) = match &asked.subject {
            Subject::Viewed(target) | Subject::Sent(target) => {
                (Action::Read, studies_of(&self.index, target, &[]).await?)
            }
            Subject::Annotated(target) => {
                (Action::Update, studies_of(&self.index, target, &[]).await?)
            }
            Subject::Exported(target) => {
                let listed = listed_resources(asked.query.as_deref(), &asked.body);
                (
                    Action::Read,
                    studies_of(&self.index, target, &listed).await?,
                )
            }
            Subject::Deleted(..) => (Action::Delete, asked.doomed.clone()),
            // the account saw the patient whole, or the request was refused
            Subject::Modified(..) => (Action::Update, asked.doomed.clone()),
            // nothing here names a study of this archive
            Subject::Search
            | Subject::Stores
            | Subject::RemoteSearch(_)
            | Subject::RemoteMove(_)
            | Subject::RemoteStore(_) => {
                return Ok(());
            }
        };
        // of an archive order that mixes both, only what the account does not see
        let theirs = self.index.limited_to(account);
        let mut hidden = Vec::new();
        for study in named {
            if theirs
                .audited_study(&study.study_instance_uid)
                .await?
                .is_none()
            {
                hidden.push(described(&study, false));
            }
        }
        if hidden.is_empty() {
            return Ok(());
        }
        // asked again within the window: one line, as for a study read again
        let what = format!("{}|{ip:?}|outside|{:?}", who.user_id, asked.subject);
        if trail.repeated(what, Instant::now()) {
            return Ok(());
        }
        for group in by_patient(hidden) {
            let mut event = events::instances_accessed(
                &trail.source,
                now,
                action,
                Outcome::MinorFailure,
                vec![who.clone()],
                &group,
            );
            event.outcome_description = Some(OUTSIDE.to_owned());
            self.audit.record(event);
        }
        Ok(())
    }

    async fn data_events(
        &self,
        trail: &Trail,
        asked: Asked,
        who: Participant,
        ip: Option<IpAddr>,
        now: SystemTime,
    ) -> Result<(), pacs_index::Error> {
        let server = || Participant::new(&trail.process, false);
        let ok = Outcome::Success;
        let caller_key = format!("{}|{:?}", who.user_id, ip);
        let fresh = |what: &str| !trail.repeated(format!("{caller_key}|{what}"), Instant::now());
        match &asked.subject {
            // written by the caller, with what was stored
            Subject::Stores => {}
            // written by the caller: a remote AE that fails was asked all the same
            Subject::RemoteSearch(_) | Subject::RemoteMove(_) | Subject::RemoteStore(_) => {}
            Subject::Search => {
                // "the query in the format of the protocol used": the body of a search
                // that has one, else the request's target, which is never empty
                let query = match (&asked.query, asked.body.is_empty()) {
                    (_, false) => asked.body.clone(),
                    (Some(query), true) => format!("{}?{query}", asked.path).into_bytes(),
                    (None, true) => asked.path.clone().into_bytes(),
                };
                let what = format!("search|{}|{}", asked.path, String::from_utf8_lossy(&query));
                if fresh(&what) {
                    self.audit.record(events::query(
                        &trail.source,
                        now,
                        ok,
                        who,
                        server(),
                        Vec::new(),
                        &asked.path,
                        &query,
                    ));
                }
            }
            Subject::Viewed(target) | Subject::Sent(target) => {
                let sent = matches!(asked.subject, Subject::Sent(_));
                let what = format!("{}|{target:?}", if sent { "sent" } else { "viewed" });
                if !fresh(&what) {
                    return Ok(());
                }
                let studies = studies_of(&self.index, target, &[]).await?;
                let studies = studies
                    .iter()
                    .map(|study| described(study, false))
                    .collect();
                for group in by_patient(studies) {
                    if sent {
                        for study in &group {
                            self.audit.record(events::instances_transferred(
                                &trail.source,
                                now,
                                Action::Read,
                                ok,
                                server(),
                                who.clone(),
                                study,
                            ));
                        }
                    } else {
                        self.audit.record(events::instances_accessed(
                            &trail.source,
                            now,
                            Action::Read,
                            ok,
                            vec![who.clone()],
                            &group,
                        ));
                    }
                }
            }
            Subject::Annotated(target) => {
                let studies = studies_of(&self.index, target, &[]).await?;
                let studies = studies
                    .iter()
                    .map(|study| described(study, false))
                    .collect();
                for group in by_patient(studies) {
                    self.audit.record(events::instances_accessed(
                        &trail.source,
                        now,
                        Action::Update,
                        ok,
                        vec![who.clone()],
                        &group,
                    ));
                }
            }
            Subject::Exported(target) => {
                let listed = listed_resources(asked.query.as_deref(), &asked.body);
                let studies = studies_of(&self.index, target, &listed).await?;
                // the counts are the study's: right for a whole study, left out otherwise
                let whole = matches!(target, Target::Resource(Level::Study | Level::Patient, _));
                let studies = studies
                    .iter()
                    .map(|study| described(study, whole))
                    .collect();
                let at = access_point(ip)
                    .unwrap_or_else(|| AccessPoint::Machine(trail.source.id.clone()));
                for group in by_patient(studies) {
                    self.audit.record(events::export(
                        &trail.source,
                        now,
                        ok,
                        who.clone(),
                        server(),
                        Participant::new(&asked.path, false).at(Some(at.clone())),
                        &group,
                    ));
                }
            }
            Subject::Deleted(..) => {
                // a study that is still there lost a part; one that is gone, all of itself
                let (mut gone, mut cut) = (Vec::new(), Vec::new());
                for study in &asked.doomed {
                    if self
                        .index
                        .audited_study(&study.study_instance_uid)
                        .await?
                        .is_some()
                    {
                        cut.push(described(study, false));
                    } else {
                        gone.push(described(study, true));
                    }
                }
                for group in by_patient(gone) {
                    self.audit.record(events::study_deleted(
                        &trail.source,
                        now,
                        ok,
                        vec![who.clone()],
                        &group,
                    ));
                }
                for group in by_patient(cut) {
                    self.audit.record(events::instances_accessed(
                        &trail.source,
                        now,
                        Action::Delete,
                        ok,
                        vec![who.clone()],
                        &group,
                    ));
                }
            }
            Subject::Modified(..) => {
                // the studies as they were, under the patient before, and as they are now,
                // under the patient after: both identities are in the trail
                let before: Vec<_> = asked.doomed.iter().map(|s| described(s, false)).collect();
                let mut after = Vec::new();
                for study in &asked.doomed {
                    if let Some(now) = self.index.audited_study(&study.study_instance_uid).await? {
                        after.push(described(&now, false));
                    }
                }
                for group in by_patient(before).into_iter().chain(by_patient(after)) {
                    self.audit.record(events::instances_accessed(
                        &trail.source,
                        now,
                        Action::Update,
                        ok,
                        vec![who.clone()],
                        &group,
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Around every request: after the answer is made, write the event it amounts to, if any.
async fn observe(State(observer): State<Observer>, request: Request, next: Next) -> Response {
    let audit = &observer.audit;
    let Some(trail) = audit.0.clone() else {
        return next.run(request).await;
    };
    let (method, path) = (request.method().clone(), request.uri().path().to_owned());
    let query = request.uri().query().map(str::to_owned);
    let subject = subject(&method, &path, &observer.mount);
    let ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| address.ip());
    let posted = |wanted: &str| method == Method::POST && path == wanted;

    // Who is asking to sign in is in the request, which the route consumes.
    let mut claimed_name = None;
    let mut asked_body = Vec::new();
    let request = if posted("/auth/login") {
        let (parts, body) = request.into_parts();
        let Ok(bytes) = axum::body::to_bytes(body, MAX_LOGIN_BODY_BYTES).await else {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        };
        claimed_name = Some(login_of(&bytes));
        Request::from_parts(parts, Body::from(bytes))
    } else if posted("/tools/find")
        || posted("/tools/create-archive")
        || matches!(
            subject,
            Some(Subject::RemoteMove(_) | Subject::RemoteStore(_))
        )
    {
        // what is searched, and what is taken away, is the event
        let (parts, body) = request.into_parts();
        let Ok(bytes) = axum::body::to_bytes(body, MAX_QUERY_BODY_BYTES).await else {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        };
        asked_body = bytes.to_vec();
        Request::from_parts(parts, Body::from(bytes))
    } else {
        if posted("/auth/token") {
            let id = basic_credentials(request.headers().get(AUTHORIZATION)).map(|(id, _)| id);
            claimed_name = Some(id.unwrap_or_default());
        }
        request
    };

    // A deletion takes what it deletes with it, a modification changes whose it is: whose it
    // was is read first.
    let doomed = match &subject {
        Some(Subject::Deleted(_, id) | Subject::Modified(id)) => {
            let level = match &subject {
                Some(Subject::Deleted(level, _)) => *level,
                _ => Level::Patient,
            };
            match observer.index.audited_by_public_id(level, id).await {
                Ok(studies) => studies,
                Err(error) => {
                    tracing::error!(%error, "the audit trail could not read what is being deleted");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
        }
        _ => Vec::new(),
    };
    let (response, stored) = if subject == Some(Subject::Stores) {
        stored_during(next.run(request)).await
    } else {
        (next.run(request).await, Vec::new())
    };
    let status = response.status();
    let user = response.extensions().get::<SignedIn>();
    let now = SystemTime::now();

    // The archive's data: who it was, then what they did.
    let who = match user {
        Some(user) => caller(&trail, user, ip),
        // no login: all that is known of the caller is where they are
        None => claimed(&trail, "", true, ip),
    };
    // what was stored is stored, whatever the answer says of the rest of the request
    for (study, action) in stored_studies(stored) {
        audit.record(events::instances_transferred(
            &trail.source,
            now,
            action,
            Outcome::Success,
            who.clone(),
            Participant::new(&trail.process, false),
            &study,
        ));
    }
    // A remote AE: asked as soon as the route got as far as calling it, which a failure
    // of that AE (502) shows as much as a success does.
    let called = status.is_success()
        || matches!(
            status,
            StatusCode::BAD_GATEWAY | StatusCode::GATEWAY_TIMEOUT
        );
    let remote =
        match &subject {
            Some(
                Subject::RemoteSearch(id) | Subject::RemoteMove(id) | Subject::RemoteStore(id),
            ) if called => observer.dimse.remote(id),
            _ => None,
        };
    if let Some(remote) = remote {
        let there = application(
            remote.ae_title.as_str(),
            Some(AccessPoint::Machine(remote.host.clone())),
        );
        let outcome = outcome_of(status);
        if matches!(subject, Some(Subject::RemoteSearch(_))) {
            let target = match &query {
                Some(query) => format!("{path}?{query}"),
                None => path.clone(),
            };
            let here = Participant::new(&trail.process, false)
                .alternative(format!("AETITLES={}", observer.dimse.ae_title.as_str()));
            audit.record(events::query(
                &trail.source,
                now,
                outcome,
                here,
                there,
                vec![who.clone()],
                &path,
                target.as_bytes(),
            ));
        } else if matches!(subject, Some(Subject::RemoteStore(_))) {
            // this server sends, for whoever asked: studies of its own, whose patient it knows
            let (studies, _) = moved(&asked_body, &observer.dimse, &observer.projects);
            let here = Participant::new(&trail.process, false)
                .alternative(format!("AETITLES={}", observer.dimse.ae_title.as_str()));
            let mut known = Vec::new();
            for uid in &studies {
                match observer.index.audited_study(uid).await {
                    Ok(Some(study)) => known.push(described(&study, true)),
                    Ok(None) => {}
                    Err(error) => {
                        tracing::error!(%error, "the audit trail could not read a study sent");
                    }
                }
            }
            for group in by_patient(known) {
                audit.record(events::begin_transferring(
                    &trail.source,
                    now,
                    outcome,
                    here.clone(),
                    there.clone(),
                    vec![who.clone()],
                    &group,
                ));
            }
        } else {
            let (studies, destination) = moved(&asked_body, &observer.dimse, &observer.projects);
            let here = Participant::new(&trail.process, false)
                .alternative(format!("AETITLES={destination}"));
            // whose they are is the remote AE's to know: one line a study, since a line
            // is about one patient
            for study_instance_uid in studies {
                audit.record(events::begin_transferring(
                    &trail.source,
                    now,
                    outcome,
                    there.clone(),
                    here.clone(),
                    vec![who.clone()],
                    &[events::Study {
                        study_instance_uid,
                        patient_id: None,
                        patient_name: None,
                        sop_classes: Vec::new(),
                    }],
                ));
            }
        }
    }
    // an account limited to some projects, told that what it asked for is not there
    let outside = match response.extensions().get::<SeenBy>() {
        Some(SeenBy(account)) if status == StatusCode::NOT_FOUND => Some(account.clone()),
        _ => None,
    };
    if let Some(subject) = subject
        && (status.is_success() || outside.is_some())
    {
        let asked = Asked {
            subject,
            path: path.clone(),
            query,
            body: asked_body,
            doomed,
        };
        let written = match &outside {
            Some(account) => {
                observer
                    .outside_events(&trail, asked, account, who, ip, now)
                    .await
            }
            None => observer.data_events(&trail, asked, who, ip, now).await,
        };
        if let Err(error) = written {
            // the request is done; what cannot be said of it is said in the other log
            tracing::error!(%error, %method, %path, "an audit message was not written");
        }
    }
    let said = || (!status.is_success()).then(|| format!("HTTP {}", status.as_u16()));

    if let Some(name) = claimed_name {
        let person = path == "/auth/login";
        let mut who = claimed(&trail, &name, person, ip);
        // once accepted, the claim is who they are: named as on every later line
        if status.is_success() && !name.is_empty() {
            who = who.named(if person {
                name.clone()
            } else {
                format!("{0}<{name}@{0}>", trail.issuer)
            });
        }
        audit.record(events::user_authentication(
            &trail.source,
            now,
            true,
            who,
            &trail.process,
            outcome_of(status),
            said(),
        ));
    } else if let Some(signed) = response.extensions().get::<crate::auth::ProviderSignIn>() {
        // a sign-in through the OpenID provider: a redirect either way, so its outcome is
        // what the callback found, not the status
        let mut who = claimed(&trail, &signed.login, true, ip);
        if signed.accepted {
            who = who.named(signed.login.clone());
        }
        let outcome = if signed.accepted {
            Outcome::Success
        } else {
            Outcome::MinorFailure
        };
        audit.record(events::user_authentication(
            &trail.source,
            now,
            true,
            who,
            &trail.process,
            outcome,
            signed.detail.clone(),
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

    fn trail() -> Trail {
        let dir = tempfile::tempdir().unwrap();
        Trail {
            file: Mutex::new(open_for_append(&dir.path().join("audit.log")).unwrap()),
            source: Source::application_server("pacs-rs"),
            sender: Sender {
                hostname: "host".into(),
                app_name: "pacs-rs".into(),
                process_id: "1".into(),
            },
            process: "pacs-server@host".into(),
            issuer: "pacs-rs".into(),
            recent: Mutex::new(HashMap::new()),
            written: Arc::new(tokio::sync::Notify::new()),
        }
    }

    #[test]
    fn the_same_read_again_is_a_repeat_until_the_window_has_passed() {
        let trail = trail();
        let start = Instant::now();
        let after = |seconds: u64| start + Duration::from_secs(seconds);
        assert!(!trail.repeated("ada|viewed|1.2.3".into(), start));
        assert!(trail.repeated("ada|viewed|1.2.3".into(), after(1)));
        assert!(trail.repeated("ada|viewed|1.2.3".into(), after(REPEAT_SECONDS - 1)));
        // another caller, another study, another kind: each its own
        assert!(!trail.repeated("bob|viewed|1.2.3".into(), after(1)));
        assert!(!trail.repeated("ada|viewed|4.5.6".into(), after(1)));
        assert!(!trail.repeated("ada|sent|1.2.3".into(), after(1)));
        // the window runs from the line that was written, not from the last repeat
        assert!(!trail.repeated("ada|viewed|1.2.3".into(), after(REPEAT_SECONDS)));
        assert!(trail.repeated("ada|viewed|1.2.3".into(), after(REPEAT_SECONDS + 1)));

        // what is remembered is bounded: the old goes when there is no more room
        let late = after(3 * REPEAT_SECONDS);
        for n in 0..MAX_REMEMBERED {
            trail.repeated(format!("filler {n}"), late);
        }
        assert!(trail.recent.lock().unwrap().len() <= MAX_REMEMBERED);
        assert!(
            !trail
                .recent
                .lock()
                .unwrap()
                .contains_key("ada|viewed|4.5.6")
        );
        // ...and when all of it is fresh, all of it goes rather than grow
        trail.repeated("one more".into(), late);
        assert!(trail.recent.lock().unwrap().len() <= MAX_REMEMBERED);
    }

    #[test]
    fn the_resources_of_an_archive_made_to_order() {
        let ids = |list: &[&str]| list.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            listed_resources(Some("resources=a,b"), b""),
            ids(&["a", "b"])
        );
        assert_eq!(
            listed_resources(Some("x=1&resources=a,,b&y=2"), b""),
            ids(&["a", "b"])
        );
        assert_eq!(
            listed_resources(
                None,
                br#"{"Resources": ["a", 7, "b"], "Synchronous": true}"#
            ),
            ids(&["a", "b"])
        );
        // the address wins over a body, as the route for a link has no body
        assert_eq!(
            listed_resources(Some("resources=a"), br#"{"Resources": ["b"]}"#),
            ids(&["a"])
        );
        for nothing in [&b"not json"[..], b"{}", br#"{"Resources": "a"}"#, b""] {
            assert_eq!(listed_resources(None, nothing), ids(&[]));
        }
        assert_eq!(listed_resources(Some("other=1"), b""), ids(&[]));
    }

    #[test]
    fn what_was_stored_is_counted_by_study_and_its_patients_are_kept_apart() {
        let stored = |study: &str, class: &str, patient: &str, outcome| Stored {
            study_instance_uid: study.into(),
            sop_class_uid: class.into(),
            patient_id: Some(patient.into()),
            patient_name: None,
            outcome,
        };
        use pacs_archive::Outcome::*;
        let studies = stored_studies(vec![
            stored("1.1", "ct", "p1", AlreadyStored),
            stored("1.2", "ct", "p2", AlreadyStored),
            stored("1.1", "ct", "p1", Replaced),
            stored("1.1", "seg", "p1", Added),
            stored("1.1", "ct", "p1", Replaced),
            stored("1.3", "ct", "p1", Replaced),
        ]);
        let summary: Vec<_> = studies
            .iter()
            .map(|(study, action)| {
                let classes: Vec<_> = study
                    .sop_classes
                    .iter()
                    .map(|c| (c.uid.as_str(), c.instances))
                    .collect();
                (study.study_instance_uid.as_str(), *action, classes)
            })
            .collect();
        assert_eq!(
            summary,
            [
                // one new instance makes it a creation, whatever the others were
                ("1.1", Action::Create, vec![("ct", 3), ("seg", 1)]),
                ("1.2", Action::Read, vec![("ct", 1)]),
                ("1.3", Action::Update, vec![("ct", 1)]),
            ]
        );
        assert_eq!(stored_studies(Vec::new()), []);

        // a message is about one patient
        let groups = by_patient(studies.into_iter().map(|(study, _)| study).collect());
        let uids: Vec<Vec<&str>> = groups
            .iter()
            .map(|group| {
                group
                    .iter()
                    .map(|s| s.study_instance_uid.as_str())
                    .collect()
            })
            .collect();
        assert_eq!(uids, [vec!["1.1", "1.3"], vec!["1.2"]]);
    }
}
