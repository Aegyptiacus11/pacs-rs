//! The audit trail (`PACS_AUDIT_FILE`): which requests leave a line, what each line says,
//! what it never holds, and the starts that must fail. Every line's message is validated
//! against DICOM's schema when `PACS_TEST_XMLLINT` names `xmllint`.

use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use base64::Engine;
use clap::Parser;
use http_body_util::BodyExt;
use pacs_core::synthetic::SyntheticSeries;
use pacs_dicomweb::multipart;
use pacs_server::audit::open_audit;
use pacs_server::auth::open_login;
use pacs_server::{Settings, app_with_audit, app_with_login, open_archive};
use serde_json::{Value, json};
use tower::ServiceExt;

const SECRET: &str = "a-signing-secret-of-at-least-32-bytes";
const ADMIN_PASSWORD: &str = "admin-password-1";
const READER_PASSWORD: &str = "reader-password-1";

fn settings(dir: &Path, extra: &[&str]) -> Settings {
    let mut args: Vec<String> = [
        "pacs-server".to_owned(),
        "--index".into(),
        dir.join("db/index.sqlite").display().to_string(),
        "--storage".into(),
        format!("file:{}", dir.join("files").display()),
        "--auth-cookie-secure".into(),
        "false".into(),
    ]
    .to_vec();
    args.extend(extra.iter().map(|s| (*s).to_owned()));
    Settings::try_parse_from(args).unwrap()
}

fn trail(dir: &Path) -> PathBuf {
    dir.join("audit.log")
}

async fn start(settings: &Settings) -> Router {
    let archive = open_archive(settings).await.unwrap();
    let env = |name: &str| match name {
        "PACS_AUTH_SECRET" => Some(SECRET.to_owned()),
        "PACS_ADMIN_PASSWORD" => Some(ADMIN_PASSWORD.to_owned()),
        _ => None,
    };
    let login = open_login(settings, archive.index(), env).await.unwrap();
    let audit = open_audit(settings).unwrap();
    app_with_audit(archive, settings, login, audit).unwrap()
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Value,
}

async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Reply {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let body = match body {
        Some(json) => {
            request = request.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    send(app, request.body(body).unwrap()).await
}

async fn sign_in(app: &Router, login: &str, password: &str) -> Reply {
    let body = json!({"email": login, "password": password});
    call(app, Method::POST, "/auth/login", None, Some(body)).await
}

async fn ask_token(app: &Router, id: &str, secret: &str) -> Reply {
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"));
    let request = Request::post("/auth/token")
        .header("authorization", format!("Basic {basic}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("grant_type=client_credentials"))
        .unwrap();
    send(app, request).await
}

/// The trail's lines since the last call, each split into its syslog header and its XML.
struct Reader {
    path: PathBuf,
    seen: usize,
}

impl Reader {
    fn new(path: PathBuf) -> Self {
        Reader { path, seen: 0 }
    }

    fn new_lines(&mut self) -> Vec<String> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        assert!(
            text.is_empty() || text.ends_with('\n'),
            "a line is cut short"
        );
        let lines: Vec<String> = text.lines().skip(self.seen).map(str::to_owned).collect();
        self.seen += lines.len();
        for line in &lines {
            check_line(line);
        }
        lines
    }

    /// The one line the last request left.
    fn one(&mut self) -> String {
        let mut lines = self.new_lines();
        assert_eq!(lines.len(), 1, "{lines:#?}");
        lines.remove(0)
    }

    fn none(&mut self) {
        assert_eq!(self.new_lines(), Vec::<String>::new());
    }
}

/// A line is the syslog message of ITI-20, and its XML is valid by DICOM's schema.
fn check_line(line: &str) {
    let (header, xml) = line.split_once(" - ").expect("a syslog header");
    let fields: Vec<&str> = header.split(' ').collect();
    assert_eq!(fields.len(), 6, "{header}");
    assert!(fields[0] == "<85>1" || fields[0] == "<84>1", "{header}");
    assert!(
        fields[1].ends_with('Z') && fields[1].len() == 24,
        "{header}"
    );
    assert_eq!(fields[3], "pacs-rs");
    assert_eq!(fields[4], std::process::id().to_string());
    assert_eq!(fields[5], "IHE+RFC-3881");
    assert!(
        xml.starts_with("<AuditMessage>") && xml.ends_with("</AuditMessage>"),
        "{xml}"
    );
    assert!(
        xml.contains("<AuditSourceIdentification AuditSourceID=\"test-pacs\">"),
        "{xml}"
    );

    let Ok(xmllint) = std::env::var("PACS_TEST_XMLLINT") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("message.xml");
    std::fs::write(&file, xml).unwrap();
    let schema =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../pacs-audit/tests/dicom-audit-message.rng");
    let output = std::process::Command::new(&xmllint)
        .arg("--noout")
        .arg("--relaxng")
        .arg(schema)
        .arg(&file)
        .output()
        .unwrap_or_else(|e| panic!("PACS_TEST_XMLLINT names {xmllint:?}, which did not run: {e}"));
    assert!(
        output.status.success(),
        "{}\n{xml}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn has(line: &str, parts: &[&str]) {
    for part in parts {
        assert!(line.contains(part), "no {part:?} in\n{line}");
    }
}

const LOGIN: &str = "originalText=\"Login\"";
const LOGOUT: &str = "originalText=\"Logout\"";
const AUTHENTICATION: &str = "<EventID csd-code=\"110114\"";
const ALERT: &str = "<EventID csd-code=\"110113\"";
const OK: &str = "EventOutcomeIndicator=\"0\"";
const REFUSED: &str = "EventOutcomeIndicator=\"4\"";

fn described(words: &str) -> String {
    let value = base64::engine::general_purpose::STANDARD.encode(words);
    format!("<ParticipantObjectDetail type=\"Alert Description\" value=\"{value}\"/>")
}

#[tokio::test]
async fn sign_ins_account_changes_and_refusals_leave_a_line_each() {
    let dir = tempfile::tempdir().unwrap();
    let path = trail(dir.path());
    let extra = [
        "--auth",
        "true",
        "--audit-file",
        path.to_str().unwrap(),
        "--audit-source-id",
        "test-pacs",
    ];
    let app = start(&settings(dir.path(), &extra)).await;
    let mut lines = Reader::new(path.clone());
    lines.none();

    // Signing in: accepted, then refused for a wrong password and for a body that names nobody.
    let admin = sign_in(&app, "Admin", ADMIN_PASSWORD).await;
    assert_eq!(admin.status, StatusCode::OK);
    let cookie = admin.headers["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let admin = admin.body["access_token"].as_str().unwrap().to_owned();
    let line = lines.one();
    assert!(line.starts_with("<85>1 "));
    has(
        &line,
        &[
            AUTHENTICATION,
            LOGIN,
            OK,
            "EventActionCode=\"E\"",
            "UserID=\"admin@test-pacs\" UserIsRequestor=\"true\"",
        ],
    );
    // no socket address in these tests: the server's own name stands for the access point
    has(
        &line,
        &["NetworkAccessPointID=\"test-pacs\" NetworkAccessPointTypeCode=\"1\""],
    );

    assert_eq!(
        sign_in(&app, "admin", "not-the-password").await.status,
        StatusCode::UNAUTHORIZED
    );
    let line = lines.one();
    assert!(line.starts_with("<84>1 "));
    has(
        &line,
        &[
            AUTHENTICATION,
            LOGIN,
            REFUSED,
            "UserID=\"admin@test-pacs\"",
            "<EventOutcomeDescription>HTTP 401</EventOutcomeDescription>",
        ],
    );

    assert_eq!(
        sign_in(&app, "<nobody>\n", "x").await.status,
        StatusCode::UNAUTHORIZED
    );
    has(
        &lines.one(),
        &[LOGIN, REFUSED, "UserID=\"&lt;nobody&gt;&#10;@test-pacs\""],
    );
    let empty = call(&app, Method::POST, "/auth/login", None, Some(json!({}))).await;
    assert!(empty.status.is_client_error());
    has(
        &lines.one(),
        &[LOGIN, REFUSED, "UserID=\"unknown@test-pacs\""],
    );

    // Reading the accounts leaves nothing, and neither does a request without a token.
    assert_eq!(
        call(&app, Method::GET, "/roles", Some(&admin), None)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, Method::GET, "/users", Some(&admin), None)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, Method::GET, "/studies", None, None).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, Method::GET, "/healthz", None, None).await.status,
        StatusCode::OK
    );
    lines.none();

    // Accounts: a user made, changed and removed; a role; a tool.
    let by_admin = "<ActiveParticipant UserID=\"admin@test-pacs\" UserName=\"admin\" UserIsRequestor=\"false\"";
    let new = json!({"login": "rita", "password": READER_PASSWORD, "role": "reader"});
    let made = call(
        &app,
        Method::POST,
        "/users",
        Some(&admin),
        Some(new.clone()),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED);
    let rita = made.body["id"].as_str().unwrap().to_owned();
    let line = lines.one();
    has(
        &line,
        &[
            ALERT,
            "originalText=\"User Security Attributes Changed\"",
            OK,
            by_admin,
            "ParticipantObjectID=\"/users\"",
            &described("a user was made"),
        ],
    );
    // a change that is refused changes nothing, and is no account change
    assert_eq!(
        call(&app, Method::POST, "/users", Some(&admin), Some(new))
            .await
            .status,
        StatusCode::CONFLICT
    );
    lines.none();

    let role = json!({"name": "Student", "permissions": ["read"]});
    assert_eq!(
        call(&app, Method::POST, "/roles", Some(&admin), Some(role))
            .await
            .status,
        StatusCode::CREATED
    );
    has(
        &lines.one(),
        &[
            ALERT,
            "originalText=\"Security Roles Changed\"",
            by_admin,
            &described("a role was made"),
        ],
    );
    let moved = call(
        &app,
        Method::PUT,
        &format!("/users/{rita}"),
        Some(&admin),
        Some(json!({"role": "student"})),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK);
    has(
        &lines.one(),
        &[
            &format!("ParticipantObjectID=\"/users/{rita}\""),
            &described("a user was changed"),
        ],
    );

    let tool = call(
        &app,
        Method::POST,
        "/clients",
        Some(&admin),
        Some(json!({"name": "Viewer", "role": "reader"})),
    )
    .await;
    assert_eq!(tool.status, StatusCode::CREATED);
    let (id, secret) = (
        tool.body["id"].as_str().unwrap().to_owned(),
        tool.body["secret"].as_str().unwrap().to_owned(),
    );
    has(&lines.one(), &[&described("a tool was made"), by_admin]);

    // A tool's token request is a sign-in too, by its ID.
    let token = ask_token(&app, &id, &secret).await;
    assert_eq!(token.status, StatusCode::OK);
    let token = token.body["access_token"].as_str().unwrap().to_owned();
    has(
        &lines.one(),
        &[
            AUTHENTICATION,
            LOGIN,
            OK,
            &format!("UserID=\"{id}\" UserIsRequestor=\"true\""),
        ],
    );
    assert_eq!(
        ask_token(&app, &id, "a-wrong-secret").await.status,
        StatusCode::UNAUTHORIZED
    );
    has(&lines.one(), &[LOGIN, REFUSED, &format!("UserID=\"{id}\"")]);

    // Refused for what the caller is: a reader who deletes, a tool that uploads.
    let reader = sign_in(&app, "rita", READER_PASSWORD).await.body["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    lines.one();
    assert_eq!(
        call(&app, Method::DELETE, "/studies/abc", Some(&reader), None)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let line = lines.one();
    assert!(line.starts_with("<84>1 "));
    has(
        &line,
        &[
            ALERT,
            "originalText=\"Access Control Decision\"",
            REFUSED,
            "UserID=\"rita@test-pacs\" UserName=\"rita\" UserIsRequestor=\"false\"",
            "ParticipantObjectID=\"/studies/abc\"",
        ],
    );
    assert_eq!(
        call(&app, Method::POST, "/instances", Some(&token), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // the tool, named as ITI-72 names a caller with a JWT: audience<subject@issuer>
    has(
        &lines.one(),
        &[
            "originalText=\"Access Control Decision\"",
            &format!("UserID=\"{id}\" UserName=\"pacs-rs&lt;{id}@pacs-rs&gt;\""),
        ],
    );

    // A password changed by its owner; then signing out, by the cookie and everywhere.
    let change = json!({"current_password": READER_PASSWORD, "new_password": "another-password-2"});
    assert_eq!(
        call(
            &app,
            Method::POST,
            "/auth/password",
            Some(&reader),
            Some(change)
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    has(
        &lines.one(),
        &[
            "UserID=\"rita@test-pacs\"",
            &described("a user changed their own password"),
        ],
    );

    assert_eq!(
        call(&app, Method::POST, "/auth/logout-all", Some(&admin), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    has(
        &lines.one(),
        &[AUTHENTICATION, LOGOUT, OK, "UserID=\"admin@test-pacs\""],
    );
    let by_cookie = Request::post("/auth/logout")
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, by_cookie).await.status, StatusCode::NO_CONTENT);
    // a cookie alone does not say whose session it was
    has(&lines.one(), &[LOGOUT, OK, "UserID=\"unknown@test-pacs\""]);

    // Nothing that opens a door is ever in the trail.
    let everything = std::fs::read_to_string(&path).unwrap();
    for hidden in [
        ADMIN_PASSWORD,
        READER_PASSWORD,
        "another-password-2",
        "not-the-password",
        &secret,
        &admin,
        &reader,
        &token,
        SECRET,
    ] {
        assert!(!everything.contains(hidden), "the trail holds {hidden:?}");
    }
}

#[tokio::test]
async fn the_trail_is_appended_to_and_kept_private() {
    let dir = tempfile::tempdir().unwrap();
    let path = trail(dir.path());
    std::fs::write(&path, "an earlier line\n").unwrap();
    let extra = [
        "--audit-file",
        path.to_str().unwrap(),
        "--audit-source-id",
        "test-pacs",
    ];
    let settings = settings(dir.path(), &extra);
    let audit = open_audit(&settings).unwrap();
    assert!(audit.is_on());
    audit.application(true, &[]);
    audit.application(false, &["PACSRS".into(), "RESEARCH".into()]);

    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "an earlier line");
    check_line(lines[1]);
    check_line(lines[2]);
    has(
        lines[1],
        &[
            "<EventID csd-code=\"110100\"",
            "originalText=\"Application Start\"",
            "originalText=\"Application\"",
            "UserID=\"pacs-server",
        ],
    );
    has(
        lines[2],
        &[
            "originalText=\"Application Stop\"",
            "AlternativeUserID=\"AETITLES=PACSRS;RESEARCH\"",
        ],
    );

    // a new trail is its owner's alone
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let fresh = dir.path().join("fresh.log");
        let extra = ["--audit-file", fresh.to_str().unwrap()];
        open_audit(&self::settings(dir.path(), &extra)).unwrap();
        assert_eq!(
            std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn without_a_file_there_is_no_trail_and_nothing_changes() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings(dir.path(), &[]);
    let audit = open_audit(&settings).unwrap();
    assert!(!audit.is_on());
    audit.application(true, &[]);
    let archive = open_archive(&settings).await.unwrap();
    let app = app_with_audit(archive, &settings, None, audit).unwrap();
    assert_eq!(
        call(&app, Method::GET, "/studies", None, None).await.status,
        StatusCode::OK
    );
    assert!(!trail(dir.path()).exists());
}

#[tokio::test]
async fn starts_that_must_fail() {
    let dir = tempfile::tempdir().unwrap();
    // a trail that cannot be opened: its folder is not there
    let nowhere = dir.path().join("missing/audit.log");
    let unopened = settings(dir.path(), &["--audit-file", nowhere.to_str().unwrap()]);
    let error = format!("{:#}", open_audit(&unopened).expect_err("a refused start"));
    assert!(error.contains("PACS_AUDIT_FILE"), "{error}");
    // ...or is a folder
    let folder = settings(dir.path(), &["--audit-file", dir.path().to_str().unwrap()]);
    assert!(open_audit(&folder).is_err());
    // a source name with a space
    let path = trail(dir.path());
    let spaced = settings(
        dir.path(),
        &[
            "--audit-file",
            path.to_str().unwrap(),
            "--audit-source-id",
            "our pacs",
        ],
    );
    let error = format!("{:#}", open_audit(&spaced).expect_err("a refused start"));
    assert!(error.contains("PACS_AUDIT_SOURCE_ID"), "{error}");

    // The trail asked for and the one given must agree: no silent server without one.
    let asked = settings(dir.path(), &["--audit-file", path.to_str().unwrap()]);
    let archive = open_archive(&asked).await.unwrap();
    assert!(app_with_login(archive.clone(), &asked, None).is_err());
    assert!(
        app_with_audit(
            archive.clone(),
            &asked,
            None,
            pacs_server::audit::Audit::off()
        )
        .is_err()
    );
    let unasked = settings(dir.path(), &[]);
    let given = open_audit(&asked).unwrap();
    assert!(app_with_audit(archive, &unasked, None, given).is_err());
}

const TRANSFERRED: &str = "<EventID csd-code=\"110104\"";
const ACCESSED: &str = "<EventID csd-code=\"110103\"";
const STUDY_DELETED: &str = "<EventID csd-code=\"110105\"";
const EXPORT: &str = "<EventID csd-code=\"110106\"";
const QUERY: &str = "<EventID csd-code=\"110112\"";
const CT: &str = "1.2.840.10008.5.1.4.1.1.2";
const SOURCE_ROLE: &str = "<RoleIDCode csd-code=\"110153\"";
const DESTINATION_ROLE: &str = "<RoleIDCode csd-code=\"110152\"";

fn study_object(uid: &str) -> String {
    format!(
        "ParticipantObjectID=\"{uid}\" ParticipantObjectTypeCode=\"2\" ParticipantObjectTypeCodeRole=\"3\">"
    )
}

fn patient_object(id: &str, name: &str) -> String {
    format!(
        "<ParticipantObjectIdentification ParticipantObjectID=\"{id}\" ParticipantObjectTypeCode=\"1\" ParticipantObjectTypeCodeRole=\"1\"><ParticipantObjectIDTypeCode csd-code=\"2\" codeSystemName=\"RFC-3881\" originalText=\"Patient Number\"/><ParticipantObjectName>{name}</ParticipantObjectName>"
    )
}

fn counted(instances: u32) -> String {
    format!("<SOPClass UID=\"{CT}\" NumberOfInstances=\"{instances}\"/>")
}

fn base64(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(text)
}

async fn upload(app: &Router, token: Option<&str>, file: Vec<u8>) -> Reply {
    let mut request = Request::post("/instances");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    send(app, request.body(Body::from(file)).unwrap()).await
}

#[tokio::test]
async fn what_is_stored_read_searched_exported_and_deleted_leaves_a_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = trail(dir.path());
    let extra = [
        "--auth",
        "true",
        "--audit-file",
        path.to_str().unwrap(),
        "--audit-source-id",
        "test-pacs",
    ];
    let app = start(&settings(dir.path(), &extra)).await;
    let mut lines = Reader::new(path.clone());
    let bearer = sign_in(&app, "admin", ADMIN_PASSWORD).await.body["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let admin = Some(bearer.as_str());
    lines.one();
    let by_admin = "UserID=\"admin@test-pacs\" UserName=\"admin\" UserIsRequestor=\"true\"";
    let server = "UserID=\"pacs-server";

    // Stored: one line per study a request brings, from the caller to the server.
    let series = SyntheticSeries::new(51, 3).with_patient("DOE^JANE", None);
    let (uid, patient) = (
        series.study_uid(),
        patient_object(&series.patient_id(), "DOE^JANE"),
    );
    let first = upload(&app, admin, series.slice_bytes(0)).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    let (study, instance) = (
        first.body["ParentStudy"].as_str().unwrap().to_owned(),
        first.body["ID"].as_str().unwrap().to_owned(),
    );
    let line = lines.one();
    has(
        &line,
        &[
            TRANSFERRED,
            "EventActionCode=\"C\"",
            OK,
            &study_object(&uid),
            &counted(1),
            &patient,
        ],
    );
    // the caller sent, the server received
    let (sender, receiver) = (line.find(by_admin).unwrap(), line.find(server).unwrap());
    assert!(sender < receiver, "{line}");
    assert!(
        line[sender..receiver].contains(SOURCE_ROLE) && line[receiver..].contains(DESTINATION_ROLE),
        "{line}"
    );
    // the same file again: the server held it already, which is a Read
    assert_eq!(
        upload(&app, admin, series.slice_bytes(0)).await.body["Status"],
        "AlreadyStored"
    );
    has(
        &lines.one(),
        &[TRANSFERRED, "EventActionCode=\"R\"", &counted(1)],
    );
    // not DICOM: nothing was stored, nothing is said
    assert_eq!(
        upload(&app, admin, b"not DICOM".to_vec()).await.status,
        StatusCode::BAD_REQUEST
    );
    lines.none();

    // STOW-RS: two studies in one request, one already partly held, are two lines.
    let other = SyntheticSeries::new(52, 2).with_patient("ROE^RICHARD", None);
    let boundary = multipart::new_boundary();
    let files = [
        series.slice_bytes(0),
        series.slice_bytes(1),
        series.slice_bytes(2),
        other.slice_bytes(0),
        other.slice_bytes(1),
    ];
    let body = multipart::body(
        &boundary,
        files
            .into_iter()
            .map(|f| ("application/dicom".to_owned(), bytes::Bytes::from(f))),
    );
    let stow = Request::post("/dicom-web/studies")
        .header("host", "pacs.test")
        .header("authorization", format!("Bearer {bearer}"))
        .header(
            "content-type",
            format!(r#"multipart/related; type="application/dicom"; boundary={boundary}"#),
        )
        .body(Body::from(body))
        .unwrap();
    assert_eq!(send(&app, stow).await.status, StatusCode::OK);
    let stored = lines.new_lines();
    assert_eq!(stored.len(), 2, "{stored:#?}");
    // two new instances and one held: a Create, with all three counted
    has(
        &stored[0],
        &[
            TRANSFERRED,
            "EventActionCode=\"C\"",
            &study_object(&uid),
            &counted(3),
            &patient,
        ],
    );
    has(
        &stored[1],
        &[
            TRANSFERRED,
            "EventActionCode=\"C\"",
            &study_object(&other.study_uid()),
            &counted(2),
            &patient_object(&other.patient_id(), "ROE^RICHARD"),
        ],
    );

    // Searched: what was asked is in the line; the same search again is not a second line.
    assert_eq!(
        call(&app, Method::GET, "/studies", admin, None)
            .await
            .status,
        StatusCode::OK
    );
    let line = lines.one();
    has(
        &line,
        &[
            QUERY,
            "EventActionCode=\"E\"",
            by_admin,
            "ParticipantObjectID=\"/studies\"",
            "originalText=\"URI\"",
            // with no parameters, what was asked is the path itself
            &format!(
                "<ParticipantObjectQuery>{}</ParticipantObjectQuery>",
                base64("/studies")
            ),
        ],
    );
    assert!(!line.contains("Patient Number"), "{line}");
    assert_eq!(
        call(&app, Method::GET, "/studies", admin, None)
            .await
            .status,
        StatusCode::OK
    );
    lines.none();
    let qido = "/dicom-web/studies?PatientID=SYN51";
    assert_eq!(
        call(&app, Method::GET, qido, admin, None).await.status,
        StatusCode::OK
    );
    has(
        &lines.one(),
        &[
            QUERY,
            "ParticipantObjectID=\"/dicom-web/studies\"",
            &format!(
                "<ParticipantObjectQuery>{}</ParticipantObjectQuery>",
                base64(qido)
            ),
        ],
    );
    let find = json!({"Level": "Study", "Query": {"PatientID": series.patient_id()}});
    assert_eq!(
        call(&app, Method::POST, "/tools/find", admin, Some(find.clone()))
            .await
            .status,
        StatusCode::OK
    );
    has(
        &lines.one(),
        &[
            QUERY,
            "ParticipantObjectID=\"/tools/find\"",
            &base64(&find.to_string()),
        ],
    );
    assert_eq!(
        call(&app, Method::GET, "/ui/api/studies?q=doe", admin, None)
            .await
            .status,
        StatusCode::OK
    );
    has(
        &lines.one(),
        &[QUERY, "ParticipantObjectID=\"/ui/api/studies\""],
    );

    // Read: a study's attributes are Accessed; its instances and frames, Transferred from
    // the server to the caller. Once per study and kind, however many requests it takes.
    let read = async |path: String| {
        let reply = call(&app, Method::GET, &path, admin, None).await;
        assert!(reply.status.is_success(), "{path}: {}", reply.status);
    };
    read(format!("/studies/{study}")).await;
    let line = lines.one();
    has(
        &line,
        &[
            ACCESSED,
            "EventActionCode=\"R\"",
            by_admin,
            &study_object(&uid),
            &patient,
        ],
    );
    // a read does not count instances
    assert!(!line.contains("SOPClass"), "{line}");
    read(format!("/studies/{study}")).await;
    read(format!("/studies/{study}/series")).await;
    read(format!("/studies/{study}/statistics")).await;
    lines.none();
    // the same study by DICOMweb is the same study, but asked another way: one more line
    read(format!("/dicom-web/studies/{uid}/metadata")).await;
    has(
        &lines.one(),
        &[ACCESSED, "EventActionCode=\"R\"", &study_object(&uid)],
    );
    read(format!("/dicom-web/studies/{uid}/series")).await;
    lines.none();

    let series_uid = series.series_uid();
    for slice in 0..3 {
        let sop = series.instance_uid(slice);
        read(format!(
            "/dicom-web/studies/{uid}/series/{series_uid}/instances/{sop}/frames/1"
        ))
        .await;
    }
    read(format!("/dicom-web/studies/{uid}/series/{series_uid}")).await;
    let line = lines.one();
    has(
        &line,
        &[
            TRANSFERRED,
            "EventActionCode=\"R\"",
            &study_object(&uid),
            &patient,
        ],
    );
    // the server sent, the caller received
    let (sender, receiver) = (line.find(server).unwrap(), line.find(by_admin).unwrap());
    assert!(sender < receiver, "{line}");
    assert!(
        line[sender..receiver].contains(SOURCE_ROLE) && line[receiver..].contains(DESTINATION_ROLE),
        "{line}"
    );
    read(format!("/instances/{instance}/file")).await;
    has(
        &lines.one(),
        &[TRANSFERRED, "EventActionCode=\"R\"", &study_object(&uid)],
    );
    // the other patient's study is its own line
    read(format!("/dicom-web/studies/{}/metadata", other.study_uid())).await;
    has(
        &lines.one(),
        &[ACCESSED, &study_object(&other.study_uid()), "ROE^RICHARD"],
    );
    // what is not there was not read
    assert_eq!(
        call(&app, Method::GET, "/studies/nothing", admin, None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            Method::GET,
            "/dicom-web/studies/1.2.3/metadata",
            admin,
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    lines.none();

    // Measurements of a study: read, then replaced.
    read(format!("/ui/api/annotations/{uid}")).await;
    lines.none(); // the study was viewed by its UID a moment ago
    let put = call(
        &app,
        Method::PUT,
        &format!("/ui/api/annotations/{uid}"),
        admin,
        Some(json!([])),
    )
    .await;
    assert!(put.status.is_success(), "{}", put.status);
    has(
        &lines.one(),
        &[
            ACCESSED,
            "EventActionCode=\"U\"",
            &study_object(&uid),
            &patient,
        ],
    );

    // Exported: every download is a line, with the whole study counted.
    for _ in 0..2 {
        read(format!("/studies/{study}/archive")).await;
        let line = lines.one();
        has(
            &line,
            &[
                EXPORT,
                "EventActionCode=\"R\"",
                by_admin,
                &study_object(&uid),
                &counted(3),
                &patient,
            ],
        );
        has(
            &line,
            &[
                &format!("UserID=\"/studies/{study}/archive\" UserIsRequestor=\"false\""),
                "originalText=\"Destination Media\"",
            ],
        );
    }
    // an archive made to order, of both patients: a line each
    let order = json!({"Resources": [study, call(&app, Method::GET, "/studies", admin, None).await.body[1]]});
    let made = call(
        &app,
        Method::POST,
        "/tools/create-archive",
        admin,
        Some(order),
    )
    .await;
    assert_eq!(made.status, StatusCode::OK, "{}", made.body);
    let exported = lines.new_lines();
    assert_eq!(exported.len(), 2, "{exported:#?}");
    assert!(exported.iter().all(|line| line.contains(EXPORT)));
    assert!(
        exported.iter().any(|line| line.contains("DOE^JANE"))
            && exported.iter().any(|line| line.contains("ROE^RICHARD"))
    );

    // Deleted: an instance of three is part of a study; the study itself is the study.
    let gone = call(
        &app,
        Method::DELETE,
        &format!("/instances/{instance}"),
        admin,
        None,
    )
    .await;
    assert_eq!(gone.status, StatusCode::OK);
    let line = lines.one();
    has(
        &line,
        &[
            ACCESSED,
            "EventActionCode=\"D\"",
            by_admin,
            &study_object(&uid),
            &patient,
        ],
    );
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/studies/{study}"),
            admin,
            None
        )
        .await
        .status,
        StatusCode::OK
    );
    let line = lines.one();
    // the two instances that were left, counted before they went
    has(
        &line,
        &[
            STUDY_DELETED,
            "EventActionCode=\"D\"",
            by_admin,
            &study_object(&uid),
            &counted(2),
            &patient,
        ],
    );
    // deleting what is gone deletes nothing
    assert_eq!(
        call(
            &app,
            Method::DELETE,
            &format!("/studies/{study}"),
            admin,
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    lines.none();
}

#[tokio::test]
async fn without_a_login_the_trail_names_the_address_and_a_tool_is_named_by_its_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = trail(dir.path());
    let extra = [
        "--audit-file",
        path.to_str().unwrap(),
        "--audit-source-id",
        "test-pacs",
    ];
    let settings = settings(dir.path(), &extra);
    let archive = open_archive(&settings).await.unwrap();
    let app = app_with_audit(archive, &settings, None, open_audit(&settings).unwrap()).unwrap();
    let mut lines = Reader::new(path);

    let series = SyntheticSeries::new(53, 1);
    assert_eq!(
        upload(&app, None, series.slice_bytes(0)).await.status,
        StatusCode::OK
    );
    // nobody signed in: the caller is unknown, at the address it came from
    has(
        &lines.one(),
        &[
            TRANSFERRED,
            "EventActionCode=\"C\"",
            "UserID=\"unknown@test-pacs\" UserIsRequestor=\"true\" NetworkAccessPointID=\"test-pacs\"",
        ],
    );
    assert_eq!(
        call(&app, Method::GET, "/patients", None, None)
            .await
            .status,
        StatusCode::OK
    );
    has(&lines.one(), &[QUERY, "UserID=\"unknown@test-pacs\""]);
}
