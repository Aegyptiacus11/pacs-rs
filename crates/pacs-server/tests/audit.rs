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

    // What is read leaves nothing yet, and neither does a request without a token.
    assert_eq!(
        call(&app, Method::GET, "/studies", Some(&admin), None)
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
    let error = format!(
        "{:#}",
        open_audit(&unopened).err().expect("a refused start")
    );
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
    let error = format!("{:#}", open_audit(&spaced).err().expect("a refused start"));
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
