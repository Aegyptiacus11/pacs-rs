//! The login (`PACS_AUTH`): what is open, what each role may do on the real routes, the users
//! API, and the starts that must fail. On SQLite always, and on Postgres when
//! `PACS_TEST_POSTGRES_URL` names a server.

use std::collections::HashMap;
use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use base64::Engine;
use clap::Parser;
use http_body_util::BodyExt;
use pacs_core::real::RealSeries;
use pacs_server::api::accounts::{
    DownloadToken, Me, Refused, Role, Session, Token, TokenRefused, Tool, User,
};
use pacs_server::auth::open_login;
use pacs_server::{Settings, app, app_with_login, open_archive};
use serde_json::{Value, json};
use tower::ServiceExt;

const SECRET: &str = "a-signing-secret-of-at-least-32-bytes";
const ADMIN_PASSWORD: &str = "admin-password-1";

/// An answer of the login's routes, as the type the API's description names for it; the
/// types refuse a field they do not name.
fn described<T: serde::de::DeserializeOwned>(answer: &Value) -> T {
    serde_json::from_value(answer.clone())
        .unwrap_or_else(|e| panic!("not what the description says: {e}\n{answer}"))
}

fn settings_with(index: &str, dir: &Path, extra: &[&str]) -> Settings {
    let storage = format!("file:{}", dir.join("files").display());
    let mut args: Vec<String> = [
        "pacs-server",
        "--index",
        index,
        "--storage",
        &storage,
        "--auth",
        "true",
    ]
    .map(String::from)
    .to_vec();
    // plain HTTP in these tests
    args.extend(["--auth-cookie-secure", "false"].map(String::from));
    args.extend(extra.iter().map(|s| s.to_string()));
    Settings::try_parse_from(args).unwrap()
}

fn settings_in(dir: &Path, extra: &[&str]) -> Settings {
    settings_with(
        &dir.join("db/index.sqlite").display().to_string(),
        dir,
        extra,
    )
}

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |name| map.get(name).cloned()
}

fn usual_env() -> impl Fn(&str) -> Option<String> {
    env(&[
        ("PACS_AUTH_SECRET", SECRET),
        ("PACS_ADMIN_PASSWORD", ADMIN_PASSWORD),
    ])
}

async fn start(settings: &Settings) -> Router {
    let archive = open_archive(settings).await.unwrap();
    let login = open_login(settings, archive.index(), usual_env())
        .await
        .unwrap();
    app_with_login(archive, settings, login).unwrap()
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Value,
}

impl Reply {
    fn cookie(&self) -> String {
        let header = self.headers["set-cookie"].to_str().unwrap();
        header
            .split(';')
            .next()
            .unwrap()
            .strip_prefix("refresh_token=")
            .unwrap()
            .to_owned()
    }

    fn token(&self) -> String {
        self.body["access_token"]
            .as_str()
            .expect("an access token")
            .to_owned()
    }
}

/// One request: `token` goes in `Authorization`, a JSON `body` or raw `bytes` in the body.
async fn send(
    app: &Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
    bytes: Option<Vec<u8>>,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "pacs.test");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let body = match (body, bytes) {
        (Some(json), _) => {
            request = request.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        (None, Some(bytes)) => Body::from(bytes),
        (None, None) => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Reply {
        status,
        headers,
        body,
    }
}

async fn get(app: &Router, path: &str, token: Option<&str>) -> Reply {
    send(app, Method::GET, path, token, None, None).await
}

async fn post(app: &Router, path: &str, token: Option<&str>, body: Value) -> Reply {
    send(app, Method::POST, path, token, Some(body), None).await
}

async fn sign_in(app: &Router, login: &str, password: &str) -> Reply {
    post(
        app,
        "/auth/login",
        None,
        json!({"email": login, "password": password}),
    )
    .await
}

async fn token_of(app: &Router, login: &str, password: &str) -> String {
    let reply = sign_in(app, login, password).await;
    assert_eq!(reply.status, StatusCode::OK, "{login}: {}", reply.body);
    described::<Session>(&reply.body);
    reply.token()
}

async fn refresh(app: &Router, cookie: &str) -> Reply {
    let request = Request::post("/auth/refresh")
        .header("cookie", format!("refresh_token={cookie}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

/// Make a user of each role; their tokens, by role.
async fn one_of_each(app: &Router, admin: &str) -> HashMap<&'static str, String> {
    let mut tokens = HashMap::from([("admin", admin.to_owned())]);
    for role in ["reader", "editor", "manager"] {
        let password = format!("{role}-password-1");
        let made = post(
            app,
            "/users",
            Some(admin),
            json!({"login": role, "password": password, "role": role}),
        )
        .await;
        assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
        described::<User>(&made.body);
        tokens.insert(role, token_of(app, role, &password).await);
    }
    tokens
}

async fn the_whole_login(settings: Settings) {
    let app = start(&settings).await;

    // Open without a token: the health check and the web interface's files.
    assert_eq!(get(&app, "/healthz", None).await.status, StatusCode::OK);
    assert_eq!(
        get(&app, "/", None).await.status,
        StatusCode::TEMPORARY_REDIRECT
    );
    assert_eq!(get(&app, "/ui/", None).await.status, StatusCode::OK);
    let config = get(&app, "/ui/config.json", None).await;
    assert_eq!(config.body["auth"], true);
    // the router does not resolve `..`: a missing file of the interface, not the studies
    assert_eq!(
        get(&app, "/ui/../studies", None).await.status,
        StatusCode::NOT_FOUND
    );

    // Everything else is refused without one, whatever the route, even one that is not there.
    let closed = [
        (Method::GET, "/studies"),
        (Method::GET, "/patients"),
        (Method::GET, "/dicom-web/studies"),
        (Method::GET, "/ui/api/studies"),
        (Method::GET, "/ui/api/protocols"),
        (Method::GET, "/modalities"),
        (Method::GET, "/jobs"),
        (Method::GET, "/users"),
        (Method::GET, "/auth/me"),
        (Method::GET, "/no/such/route"),
        (Method::POST, "/instances"),
        (Method::POST, "/dicom-web/studies"),
        (Method::POST, "/tools/find"),
        (Method::POST, "/auth/logout-all"),
        (Method::POST, "/auth/password"),
        (Method::DELETE, "/studies/abc"),
        (Method::PUT, "/ui/api/protocols/p1"),
    ];
    for (method, path) in closed {
        for token in [None, Some("nonsense"), Some("")] {
            let reply = send(&app, method.clone(), path, token, None, None).await;
            assert_eq!(
                reply.status,
                StatusCode::UNAUTHORIZED,
                "{method} {path} with {token:?}"
            );
            assert_eq!(
                reply.headers["www-authenticate"], "Bearer",
                "{method} {path}"
            );
            assert!(reply.body["detail"].is_string(), "{method} {path}");
        }
    }
    // a token signed with another secret is no token
    let foreign = jwt_sessions::AccessTokens::new("another-secret-of-at-least-32-bytes!!")
        .unwrap()
        .issue(
            "u1",
            json!({"role": "admin", "login": "admin"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        get(&app, "/studies", Some(&foreign)).await.status,
        StatusCode::UNAUTHORIZED
    );

    // The first administrator signs in; a wrong password and an unknown login look the same.
    let wrong = sign_in(&app, "admin", "not-the-password").await;
    let unknown = sign_in(&app, "nobody", "not-the-password").await;
    assert_eq!(
        (wrong.status, unknown.status),
        (StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED)
    );
    assert_eq!(wrong.body, unknown.body);
    let admin_session = sign_in(&app, "Admin", ADMIN_PASSWORD).await; // logins ignore case
    assert_eq!(admin_session.status, StatusCode::OK);
    let cookie = admin_session.headers["set-cookie"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        cookie.contains("Path=/auth") && cookie.contains("HttpOnly"),
        "{cookie}"
    );
    let admin = admin_session.token();
    let me = get(&app, "/auth/me", Some(&admin)).await;
    assert_eq!(
        (me.body["login"].as_str(), me.body["role"].as_str()),
        (Some("admin"), Some("admin"))
    );

    // The refresh cookie alone authorises nothing but a refresh.
    let by_cookie = Request::get("/studies")
        .header(
            "cookie",
            format!("refresh_token={}", admin_session.cookie()),
        )
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(by_cookie).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let refreshed = refresh(&app, &admin_session.cookie()).await;
    assert_eq!(refreshed.status, StatusCode::OK);
    assert_eq!(
        get(&app, "/studies", Some(&refreshed.token())).await.status,
        StatusCode::OK
    );

    let tokens = one_of_each(&app, &admin).await;
    let series = RealSeries::new(91, 2);

    // Reading: everyone signed in. Writing: editor and above.
    for (role, may_write) in [
        ("reader", false),
        ("editor", true),
        ("manager", true),
        ("admin", true),
    ] {
        let token = Some(tokens[role].as_str());
        let upload = send(
            &app,
            Method::POST,
            "/instances",
            token,
            None,
            Some(series.slice_bytes(0)),
        )
        .await;
        let expected = if may_write {
            StatusCode::OK
        } else {
            StatusCode::FORBIDDEN
        };
        assert_eq!(upload.status, expected, "{role} uploads");
        if !may_write {
            assert_eq!(
                upload.body,
                json!({"detail": "Your role does not allow this"})
            );
            assert!(!upload.headers.contains_key("www-authenticate"));
        }
        let protocol = send(
            &app,
            Method::PUT,
            "/ui/api/protocols/p1",
            token,
            Some(json!({"id": "p1"})),
            None,
        )
        .await;
        assert_eq!(
            protocol.status.is_success(),
            may_write,
            "{role} saves a protocol: {}",
            protocol.status
        );
        let modalities = get(&app, "/modalities", token).await;
        assert_eq!(
            modalities.status.is_success(),
            may_write,
            "{role} lists the remote AEs"
        );
        if !may_write {
            assert_eq!(modalities.status, StatusCode::FORBIDDEN);
        }

        for path in [
            "/studies",
            "/patients",
            "/dicom-web/studies",
            "/ui/api/studies",
            "/ui/api/protocols",
        ] {
            assert_eq!(
                get(&app, path, token).await.status,
                StatusCode::OK,
                "{role} reads {path}"
            );
        }
        let found = post(
            &app,
            "/tools/find",
            token,
            json!({"Level": "Study", "Query": {}}),
        )
        .await;
        assert_eq!(
            found.status,
            StatusCode::OK,
            "{role} searches: {}",
            found.body
        );
    }
    let study = get(&app, "/studies", Some(&admin)).await.body[0]
        .as_str()
        .unwrap()
        .to_owned();

    // Deleting: manager and above. Users: the administrator only.
    for role in ["reader", "editor"] {
        let token = Some(tokens[role].as_str());
        let delete = send(
            &app,
            Method::DELETE,
            &format!("/studies/{study}"),
            token,
            None,
            None,
        )
        .await;
        assert_eq!(delete.status, StatusCode::FORBIDDEN, "{role} deletes");
    }
    assert_eq!(
        get(&app, &format!("/studies/{study}"), Some(&admin))
            .await
            .status,
        StatusCode::OK
    );
    let delete = send(
        &app,
        Method::DELETE,
        &format!("/studies/{study}"),
        Some(&tokens["manager"]),
        None,
        None,
    )
    .await;
    assert_eq!(delete.status, StatusCode::OK, "{}", delete.body);
    assert_eq!(
        get(&app, &format!("/studies/{study}"), Some(&admin))
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    for role in ["reader", "editor", "manager"] {
        let token = Some(tokens[role].as_str());
        assert_eq!(
            get(&app, "/users", token).await.status,
            StatusCode::FORBIDDEN,
            "{role} lists users"
        );
        let make = post(
            &app,
            "/users",
            token,
            json!({"login": "x", "password": "long-enough-password", "role": "admin"}),
        )
        .await;
        assert_eq!(make.status, StatusCode::FORBIDDEN, "{role} makes a user");
        // their own account is theirs
        assert_eq!(get(&app, "/auth/me", token).await.body["role"], role);
    }
    let users = get(&app, "/users", Some(&admin)).await;
    let logins: Vec<&str> = users
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["login"].as_str().unwrap())
        .collect();
    assert_eq!(logins, ["admin", "editor", "manager", "reader"]);
    assert!(
        !users.body.to_string().contains("argon2"),
        "a password hash is listed"
    );
}

#[tokio::test]
async fn the_whole_login_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    the_whole_login(settings_in(dir.path(), &[])).await;
}

/// Settings on a new, empty database of the Postgres server `PACS_TEST_POSTGRES_URL` names;
/// `None` when it names none.
async fn postgres_settings(name: &str, dir: &Path) -> Option<Settings> {
    let Ok(server) = std::env::var("PACS_TEST_POSTGRES_URL") else {
        eprintln!("PACS_TEST_POSTGRES_URL not set; skipping");
        return None;
    };
    let server = server.trim_end_matches('/');
    let name = format!("{name}_{}", std::process::id());
    sqlx::any::install_default_drivers();
    let mut admin =
        <sqlx::AnyConnection as sqlx::Connection>::connect(&format!("{server}/postgres"))
            .await
            .unwrap();
    for statement in [
        format!("DROP DATABASE IF EXISTS {name}"),
        format!("CREATE DATABASE {name}"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&mut admin)
            .await
            .unwrap();
    }
    Some(settings_with(&format!("{server}/{name}"), dir, &[]))
}

#[tokio::test]
async fn the_whole_login_on_postgres() {
    let dir = tempfile::tempdir().unwrap();
    if let Some(settings) = postgres_settings("pacs_auth_test", dir.path()).await {
        the_whole_login(settings).await;
    }
}

async fn user_id(app: &Router, admin: &str, login: &str) -> String {
    let users = get(app, "/users", Some(admin)).await;
    let user = users
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["login"] == login)
        .unwrap()
        .clone();
    user["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn users_are_made_changed_and_removed_by_the_administrator() {
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings_in(dir.path(), &[])).await;
    let admin = token_of(&app, "admin", ADMIN_PASSWORD).await;
    let make = |body: Value| post(&app, "/users", Some(&admin), body);

    // What is not a user is refused, each for its own reason.
    let unprocessable = [
        json!({"login": "bob", "password": "short", "role": "reader"}),
        json!({"login": "bob", "password": "x".repeat(2000), "role": "reader"}),
        json!({"login": "bob", "password": "long-enough-password", "role": "owner"}),
        json!({"login": "", "password": "long-enough-password", "role": "reader"}),
        json!({"login": "bob smith", "password": "long-enough-password", "role": "reader"}),
        json!({"login": "bob", "password": "long-enough-password"}),
    ];
    for body in unprocessable {
        let reply = make(body.clone()).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }
    let bob = make(json!({"login": "Bob", "password": "bob-password-1", "role": "reader"})).await;
    assert_eq!(bob.status, StatusCode::CREATED);
    assert_eq!(
        (bob.body["login"].as_str(), bob.body["active"].as_bool()),
        (Some("bob"), Some(true))
    );
    assert!(bob.body.get("password_hash").is_none());
    let again = make(json!({"login": "BOB", "password": "bob-password-2", "role": "editor"})).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    let bob_id = bob.body["id"].as_str().unwrap().to_owned();
    let bob_path = format!("/users/{bob_id}");
    let update = |body: Value| send(&app, Method::PUT, &bob_path, Some(&admin), Some(body), None);

    // A new role reaches the access token at the next refresh; the session goes on.
    let session = sign_in(&app, "bob", "bob-password-1").await;
    let promoted = update(json!({"role": "editor"})).await;
    assert_eq!(
        (promoted.status, promoted.body["role"].as_str()),
        (StatusCode::OK, Some("editor"))
    );
    let renewed = refresh(&app, &session.cookie()).await;
    assert_eq!(
        get(&app, "/auth/me", Some(&renewed.token())).await.body["role"],
        "editor"
    );
    assert_eq!(
        update(json!({"role": "owner"})).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        update(json!({"password": "short"})).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // A new password ends the sessions, and the old password with them.
    assert_eq!(
        update(json!({"password": "bob-password-3"})).await.status,
        StatusCode::OK
    );
    assert_eq!(
        refresh(&app, &renewed.cookie()).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        sign_in(&app, "bob", "bob-password-1").await.status,
        StatusCode::UNAUTHORIZED
    );
    let session = sign_in(&app, "bob", "bob-password-3").await;
    assert_eq!(session.status, StatusCode::OK);

    // Deactivated: no refresh, no sign-in. Activated again: back in.
    assert_eq!(update(json!({"active": false})).await.body["active"], false);
    assert_eq!(
        refresh(&app, &session.cookie()).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        sign_in(&app, "bob", "bob-password-3").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(update(json!({"active": true})).await.status, StatusCode::OK);
    let session = sign_in(&app, "bob", "bob-password-3").await;
    assert_eq!(session.status, StatusCode::OK);

    // Removed: the session ends, the login is free again.
    let remove =
        async |path: String| send(&app, Method::DELETE, &path, Some(&admin), None, None).await;
    assert_eq!(
        remove(bob_path.clone()).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        refresh(&app, &session.cookie()).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(remove(bob_path.clone()).await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        update(json!({"role": "reader"})).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        make(json!({"login": "bob", "password": "bob-password-4", "role": "reader"}))
            .await
            .status,
        StatusCode::CREATED
    );

    // The last administrator cannot be removed, deactivated or demoted, by anyone.
    let admin_path = format!("/users/{}", user_id(&app, &admin, "admin").await);
    assert_eq!(
        remove(admin_path.clone()).await.status,
        StatusCode::CONFLICT
    );
    for change in [json!({"active": false}), json!({"role": "manager"})] {
        let reply = send(
            &app,
            Method::PUT,
            &admin_path,
            Some(&admin),
            Some(change),
            None,
        )
        .await;
        assert_eq!(reply.status, StatusCode::CONFLICT);
    }
    assert_eq!(
        get(&app, "/users", Some(&admin)).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_user_changes_their_own_password() {
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings_in(dir.path(), &[])).await;
    let admin = token_of(&app, "admin", ADMIN_PASSWORD).await;
    post(
        &app,
        "/users",
        Some(&admin),
        json!({"login": "eve", "password": "eve-password-1", "role": "reader"}),
    )
    .await;
    let session = sign_in(&app, "eve", "eve-password-1").await;
    let eve = session.token();
    let change = |current: &str, new: &str| {
        post(
            &app,
            "/auth/password",
            Some(&eve),
            json!({"current_password": current, "new_password": new}),
        )
    };

    assert_eq!(
        change("not-my-password", "eve-password-2").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        change("eve-password-1", "short").await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        sign_in(&app, "eve", "eve-password-1").await.status,
        StatusCode::OK,
        "nothing changed yet"
    );

    assert_eq!(
        change("eve-password-1", "eve-password-2").await.status,
        StatusCode::NO_CONTENT
    );
    // every session ends, the one that changed it included
    assert_eq!(
        refresh(&app, &session.cookie()).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        sign_in(&app, "eve", "eve-password-1").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        sign_in(&app, "eve", "eve-password-2").await.status,
        StatusCode::OK
    );
    // and it changed nobody else's
    assert_eq!(
        sign_in(&app, "admin", ADMIN_PASSWORD).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn sessions_and_users_survive_a_restart_and_the_first_password_is_read_once() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(dir.path(), &[]);
    let cookie = {
        let app = start(&settings).await;
        sign_in(&app, "admin", ADMIN_PASSWORD).await.cookie()
    };
    // Started again with another first password, and with none: the administrator is as made.
    for restart_env in [
        env(&[
            ("PACS_AUTH_SECRET", SECRET),
            ("PACS_ADMIN_PASSWORD", "another-password-9"),
        ]),
        env(&[("PACS_AUTH_SECRET", SECRET)]),
    ] {
        let archive = open_archive(&settings).await.unwrap();
        let login = open_login(&settings, archive.index(), restart_env)
            .await
            .unwrap();
        let app = app_with_login(archive, &settings, login).unwrap();
        assert_eq!(
            sign_in(&app, "admin", "another-password-9").await.status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            sign_in(&app, "admin", ADMIN_PASSWORD).await.status,
            StatusCode::OK
        );
    }
    let app = start(&settings).await;
    assert_eq!(
        refresh(&app, &cookie).await.status,
        StatusCode::OK,
        "the session outlives restarts"
    );

    // Another signing secret: the old access tokens are refused, the refresh cookies still work.
    let token = token_of(&app, "admin", ADMIN_PASSWORD).await;
    let archive = open_archive(&settings).await.unwrap();
    let other = env(&[("PACS_AUTH_SECRET", "a-different-secret-of-32-bytes-or-more")]);
    let login = open_login(&settings, archive.index(), other).await.unwrap();
    let app = app_with_login(archive, &settings, login).unwrap();
    assert_eq!(
        get(&app, "/studies", Some(&token)).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn starts_that_must_fail() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(dir.path(), &[]);
    let archive = open_archive(&settings).await.unwrap();
    let failure = async |pairs: &[(&str, &str)], settings: &Settings| {
        let error = open_login(settings, archive.index(), env(pairs))
            .await
            .err()
            .expect("a refused start");
        format!("{error:#}")
    };

    let no_secret = failure(&[("PACS_ADMIN_PASSWORD", ADMIN_PASSWORD)], &settings).await;
    assert!(no_secret.contains("PACS_AUTH_SECRET"), "{no_secret}");
    let short = failure(
        &[
            ("PACS_AUTH_SECRET", "short"),
            ("PACS_ADMIN_PASSWORD", ADMIN_PASSWORD),
        ],
        &settings,
    )
    .await;
    assert!(short.contains("32 bytes"), "{short}");
    let no_password = failure(&[("PACS_AUTH_SECRET", SECRET)], &settings).await;
    assert!(no_password.contains("PACS_ADMIN_PASSWORD"), "{no_password}");
    let weak = failure(
        &[
            ("PACS_AUTH_SECRET", SECRET),
            ("PACS_ADMIN_PASSWORD", "password"),
        ],
        &settings,
    )
    .await;
    assert!(weak.contains("12 to 1024"), "{weak}");
    let missing_file = failure(
        &[("PACS_AUTH_SECRET_FILE", "/nonexistent/secret")],
        &settings,
    )
    .await;
    assert!(
        missing_file.contains("PACS_AUTH_SECRET_FILE"),
        "{missing_file}"
    );
    let bad_login = settings_in(dir.path(), &["--admin-login", "the admin"]);
    let spaced = failure(
        &[
            ("PACS_AUTH_SECRET", SECRET),
            ("PACS_ADMIN_PASSWORD", ADMIN_PASSWORD),
        ],
        &bad_login,
    )
    .await;
    assert!(spaced.contains("PACS_ADMIN_LOGIN"), "{spaced}");
    let bad_issuer = settings_in(dir.path(), &["--auth-issuer", "our pacs"]);
    let spaced = failure(
        &[
            ("PACS_AUTH_SECRET", SECRET),
            ("PACS_ADMIN_PASSWORD", ADMIN_PASSWORD),
        ],
        &bad_issuer,
    )
    .await;
    assert!(spaced.contains("PACS_AUTH_ISSUER"), "{spaced}");
    // none of them left a user behind
    assert_eq!(archive.index().user_count().await.unwrap(), 0);

    let bad_cookie = settings_in(
        dir.path(),
        &["--auth-cookie-path", "/auth; Domain=evil.example"],
    );
    let login = open_login(&bad_cookie, archive.index(), usual_env())
        .await
        .unwrap();
    assert!(app_with_login(archive.clone(), &bad_cookie, login).is_err());

    // The login asked for and the one given must agree: no silent server without a guard.
    assert!(app(archive.clone(), &settings).is_err());
    assert!(app_with_login(archive.clone(), &settings, None).is_err());
    let off = Settings::try_parse_from([
        "pacs-server",
        "--index",
        &dir.path().join("db/index.sqlite").display().to_string(),
        "--storage",
        &format!("file:{}", dir.path().join("files").display()),
    ])
    .unwrap();
    assert!(
        open_login(&off, archive.index(), usual_env())
            .await
            .unwrap()
            .is_none()
    );
    let login = open_login(&settings, archive.index(), usual_env())
        .await
        .unwrap();
    assert!(app_with_login(archive, &off, login).is_err());
}

#[tokio::test]
async fn without_auth_nothing_asks_for_a_token_and_there_are_no_auth_routes() {
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings::try_parse_from([
        "pacs-server",
        "--index",
        &dir.path().join("db/index.sqlite").display().to_string(),
        "--storage",
        &format!("file:{}", dir.path().join("files").display()),
    ])
    .unwrap();
    assert!(!settings.auth);
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    assert_eq!(get(&app, "/studies", None).await.status, StatusCode::OK);
    assert_eq!(get(&app, "/ui/config.json", None).await.body["auth"], false);
    assert_eq!(
        get(&app, "/users", None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        sign_in(&app, "admin", ADMIN_PASSWORD).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn cors_preflight_passes_the_guard() {
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings_in(
        dir.path(),
        &["--cors-origins", "https://viewer.example"],
    ))
    .await;
    let preflight = Request::builder()
        .method(Method::OPTIONS)
        .uri("/dicom-web/studies")
        .header("origin", "https://viewer.example")
        .header("access-control-request-method", "GET")
        .header("access-control-request-headers", "authorization")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(preflight).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "https://viewer.example"
    );
    // the request itself still needs the token, and the refusal carries the CORS header
    let refused = Request::get("/dicom-web/studies")
        .header("origin", "https://viewer.example")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(refused).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "https://viewer.example"
    );
}

async fn download_token(app: &Router, token: &str, path: &str) -> Reply {
    post(
        app,
        "/auth/download-token",
        Some(token),
        json!({"path": path}),
    )
    .await
}

#[tokio::test]
async fn a_download_token_is_good_for_one_path_and_only_in_the_url() {
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings_in(dir.path(), &[])).await;
    let admin = token_of(&app, "admin", ADMIN_PASSWORD).await;
    let tokens = one_of_each(&app, &admin).await;
    let reader = tokens["reader"].as_str();
    for seed in [95, 96] {
        let bytes = RealSeries::new(seed, 1).slice_bytes(0);
        let upload = send(
            &app,
            Method::POST,
            "/instances",
            Some(&admin),
            None,
            Some(bytes),
        )
        .await;
        assert_eq!(upload.status, StatusCode::OK);
    }
    let studies = get(&app, "/studies", Some(reader)).await;
    let ids: Vec<&str> = studies
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    let archive = format!("/studies/{}/archive", ids[0]);
    let other = format!("/studies/{}/archive", ids[1]);

    assert_eq!(
        download_token(&app, "nonsense", &archive).await.status,
        StatusCode::UNAUTHORIZED
    );
    let made = download_token(&app, reader, &archive).await;
    assert_eq!(made.status, StatusCode::OK, "{}", made.body);
    assert_eq!(made.body["expires_in"], 60);
    let token = made.body["token"].as_str().unwrap();

    // In the URL, with no header, the browser gets the archive.
    let raw = |method: Method, uri: String| {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        app.clone().oneshot(request)
    };
    let zip = raw(Method::GET, format!("{archive}?token={token}"))
        .await
        .unwrap();
    assert_eq!(zip.status(), StatusCode::OK);
    assert!(
        zip.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("zip")
    );
    let bytes = zip.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..2], b"PK");

    // Not for another path, another method, the header, or once changed.
    assert_eq!(
        raw(Method::GET, format!("{other}?token={token}"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        raw(Method::GET, format!("/studies?token={token}"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        raw(Method::GET, format!("{archive}?x=1&token={token}"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        raw(Method::DELETE, format!("/studies/{}?token={token}", ids[0]))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        raw(Method::POST, format!("{archive}?token={token}"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        raw(Method::GET, format!("{archive}?token={token}x"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&app, &archive, Some(token)).await.status,
        StatusCode::UNAUTHORIZED,
        "as a Bearer"
    );
    assert_eq!(
        get(&app, "/studies", Some(token)).await.status,
        StatusCode::UNAUTHORIZED,
        "as a Bearer"
    );
    // An access token is never accepted from a URL.
    assert_eq!(
        raw(Method::GET, format!("{archive}?token={reader}"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        raw(Method::GET, format!("/studies?token={admin}"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    // A path with a query is bound with its query.
    let both = format!("/tools/create-archive?resources={},{}", ids[0], ids[1]);
    let made = download_token(&app, reader, &both).await;
    let token = made.body["token"].as_str().unwrap();
    assert_eq!(
        raw(Method::GET, format!("{both}&token={token}"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let one = format!("/tools/create-archive?resources={}", ids[0]);
    assert_eq!(
        raw(Method::GET, format!("{one}&token={token}"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    // No token for accounts, for what is open, for what the role may not read, or for what is no path.
    for path in ["/users", "/auth/me", "/auth/logout-all", "/healthz", "/ui/"] {
        assert_eq!(
            download_token(&app, &admin, path).await.status,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    assert_eq!(
        download_token(&app, reader, "/modalities").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        download_token(&app, &tokens["editor"], "/modalities")
            .await
            .status,
        StatusCode::OK
    );
    for path in [
        "studies",
        "",
        "https://evil.example/x",
        "/studies/a b",
        "/studies?token=x",
        "/studies#x",
    ] {
        let reply = download_token(&app, &admin, path).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{path:?}");
    }
    // A deactivated user's token still runs out by itself within the minute; a role is checked
    // again when it is used.
    let made = download_token(&app, &tokens["editor"], "/modalities").await;
    let token = made.body["token"].as_str().unwrap();
    assert_eq!(
        raw(Method::GET, format!("/modalities?token={token}"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn roles_are_made_changed_and_removed_and_decide_what_a_user_may_do() {
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings_in(dir.path(), &[])).await;
    let admin = token_of(&app, "admin", ADMIN_PASSWORD).await;
    let put = async |path: String, body: Value| {
        send(&app, Method::PUT, &path, Some(&admin), Some(body), None).await
    };
    let ids = |reply: &Reply| -> Vec<String> {
        reply
            .body
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_owned())
            .collect()
    };

    // A server starts with four roles; only admin is locked.
    let roles = get(&app, "/roles", Some(&admin)).await;
    assert_eq!(ids(&roles), ["reader", "editor", "manager", "admin"]);
    assert_eq!(
        roles.body[0],
        json!({"id": "reader", "name": "Reader", "permissions": ["read"], "locked": false})
    );
    assert_eq!(roles.body[3]["locked"], true);
    assert_eq!(
        roles.body[3]["permissions"],
        json!(["read", "write", "remote", "delete", "modify", "users"])
    );
    let me = get(&app, "/auth/me", Some(&admin)).await;
    assert_eq!(
        me.body["permissions"],
        json!(["read", "write", "remote", "delete", "modify", "users"])
    );

    // A new role: its id comes from its name, its permissions are stored in one order.
    let made = post(
        &app,
        "/roles",
        Some(&admin),
        json!({"name": "Night shift", "permissions": ["delete", "read"]}),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(
        made.body,
        json!({"id": "night-shift", "name": "Night shift", "permissions": ["read", "delete"], "locked": false})
    );
    let again = post(&app, "/roles", Some(&admin), json!({"name": "night shift"})).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    for body in [
        json!({"name": "", "permissions": ["read"]}),
        json!({"name": "---"}),
        json!({"name": "Student", "permissions": ["read", "everything"]}),
        json!({"permissions": ["read"]}),
    ] {
        let reply = post(&app, "/roles", Some(&admin), body.clone()).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }

    // A user with it reads and deletes, and cannot upload: permissions do not include one another.
    let nina = post(
        &app,
        "/users",
        Some(&admin),
        json!({"login": "nina", "password": "nina-password-1", "role": "night-shift"}),
    )
    .await;
    assert_eq!(nina.status, StatusCode::CREATED, "{}", nina.body);
    let unknown = post(
        &app,
        "/users",
        Some(&admin),
        json!({"login": "x", "password": "long-enough-password", "role": "owner"}),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);
    let session = sign_in(&app, "nina", "nina-password-1").await;
    let token = session.token();
    assert_eq!(
        get(&app, "/auth/me", Some(&token)).await.body["permissions"],
        json!(["read", "delete"])
    );
    let series = RealSeries::new(97, 1);
    let upload = async |token: String| {
        send(
            &app,
            Method::POST,
            "/instances",
            Some(&token),
            None,
            Some(series.slice_bytes(0)),
        )
        .await
        .status
    };
    assert_eq!(
        get(&app, "/studies", Some(&token)).await.status,
        StatusCode::OK
    );
    assert_eq!(upload(token.clone()).await, StatusCode::FORBIDDEN);
    assert_eq!(
        get(&app, "/modalities", Some(&token)).await.status,
        StatusCode::FORBIDDEN
    );
    // every signed-in user sees the roles; only who manages users changes them
    assert_eq!(
        get(&app, "/roles", Some(&token)).await.status,
        StatusCode::OK
    );
    assert_eq!(
        post(
            &app,
            "/roles",
            Some(&token),
            json!({"name": "Mine", "permissions": ["users"]})
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let self_promote = send(
        &app,
        Method::PUT,
        "/roles/night-shift",
        Some(&token),
        Some(json!({"permissions": ["users"]})),
        None,
    )
    .await;
    assert_eq!(self_promote.status, StatusCode::FORBIDDEN);

    // The role gains a permission: the user has it at their next refresh, not before.
    let widened = put(
        "/roles/night-shift".into(),
        json!({"permissions": ["read", "write", "delete"]}),
    )
    .await;
    assert_eq!(
        widened.body["permissions"],
        json!(["read", "write", "delete"])
    );
    assert_eq!(
        upload(token.clone()).await,
        StatusCode::FORBIDDEN,
        "the old token still says what it said"
    );
    let renewed = refresh(&app, &session.cookie()).await;
    assert_eq!(upload(renewed.token()).await, StatusCode::OK);
    // It loses everything: the user stays signed in and may do nothing but see their account.
    put("/roles/night-shift".into(), json!({"permissions": []})).await;
    let renewed = refresh(&app, &renewed.cookie()).await;
    let token = renewed.token();
    assert_eq!(
        get(&app, "/studies", Some(&token)).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(&app, "/auth/me", Some(&token)).await.body["permissions"],
        json!([])
    );

    // Renaming keeps the id, so the users keep their role.
    let renamed = put("/roles/night-shift".into(), json!({"name": "On call"})).await;
    assert_eq!(
        (renamed.body["id"].as_str(), renamed.body["name"].as_str()),
        (Some("night-shift"), Some("On call"))
    );
    assert_eq!(
        put("/roles/night-shift".into(), json!({"permissions": ["fly"]}))
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        put("/roles/nobody".into(), json!({"name": "x"}))
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // A role someone has cannot be removed; a free one can, a built-in one too.
    let remove = async |id: &str| {
        send(
            &app,
            Method::DELETE,
            &format!("/roles/{id}"),
            Some(&admin),
            None,
            None,
        )
        .await
        .status
    };
    assert_eq!(remove("night-shift").await, StatusCode::CONFLICT);
    let nina_id = nina.body["id"].as_str().unwrap();
    send(
        &app,
        Method::PUT,
        &format!("/users/{nina_id}"),
        Some(&admin),
        Some(json!({"role": "reader"})),
        None,
    )
    .await;
    assert_eq!(remove("night-shift").await, StatusCode::NO_CONTENT);
    assert_eq!(remove("night-shift").await, StatusCode::NOT_FOUND);
    assert_eq!(remove("manager").await, StatusCode::NO_CONTENT);
    assert_eq!(
        ids(&get(&app, "/roles", Some(&admin)).await),
        ["reader", "editor", "admin"]
    );

    // The admin role is as it is, for good: there is always a role that can manage the rest.
    assert_eq!(remove("admin").await, StatusCode::CONFLICT);
    assert_eq!(
        put("/roles/admin".into(), json!({"permissions": ["read"]}))
            .await
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        put("/roles/admin".into(), json!({"name": "Root"}))
            .await
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        get(&app, "/users", Some(&admin)).await.status,
        StatusCode::OK
    );
}

/// `POST /auth/token` as a client sends it: a form, its id and secret in `Authorization: Basic`.
async fn ask_token(app: &Router, credentials: Option<(&str, &str)>, form: &str) -> Reply {
    let mut request = Request::post("/auth/token")
        .header("host", "pacs.test")
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some((id, secret)) = credentials {
        let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"));
        request = request.header("authorization", format!("Basic {encoded}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(form.to_owned())).unwrap())
        .await
        .unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

/// A new client of `role`: its id and secret.
async fn make_client(app: &Router, admin: &str, name: &str, role: &str) -> (String, String) {
    let made = post(
        app,
        "/clients",
        Some(admin),
        json!({"name": name, "role": role}),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert!(described::<Tool>(&made.body).secret.is_some());
    let field = |name: &str| made.body[name].as_str().unwrap().to_owned();
    (field("id"), field("secret"))
}

async fn client_token(app: &Router, id: &str, secret: &str, form: &str) -> String {
    let reply = ask_token(app, Some((id, secret)), form).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    described::<Token>(&reply.body);
    reply.token()
}

/// The claims of a JWT, unverified.
fn claims_of(token: &str) -> Value {
    let payload = token.split('.').nth(1).expect("a JWT");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

const GRANT: &str = "grant_type=client_credentials";

async fn a_client_from_secret_to_revocation(settings: Settings) {
    let app = start(&settings).await;
    let admin = token_of(&app, "admin", ADMIN_PASSWORD).await;

    // Made by an administrator: the secret is in this answer, which no cache keeps.
    let made = post(
        &app,
        "/clients",
        Some(&admin),
        json!({"name": " Nightly pipeline ", "role": "editor"}),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(made.headers["cache-control"], "no-store");
    let (id, secret) = (
        made.body["id"].as_str().unwrap().to_owned(),
        made.body["secret"].as_str().unwrap().to_owned(),
    );
    assert_eq!(secret.len(), 64);
    assert_eq!(made.body["name"], "Nightly pipeline");
    assert_eq!(made.body["created_by"], "admin");
    assert_eq!(made.body["last_token_ms"], Value::Null);

    // ...and nowhere after: the list has neither the secret nor its hash.
    let listed = get(&app, "/clients", Some(&admin)).await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.body.as_array().unwrap().len(), 1);
    let text = listed.body.to_string();
    assert!(
        !text.contains(&secret) && !text.contains("secret"),
        "{text}"
    );

    // The secret gets an access token, as the client credentials grant answers.
    let reply = ask_token(&app, Some((&id, &secret)), GRANT).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.headers["cache-control"], "no-store");
    assert_eq!(reply.headers["pragma"], "no-cache");
    assert_eq!(reply.body["token_type"], "Bearer");
    assert_eq!(reply.body["scope"], "read write remote");
    assert_eq!(reply.body["expires_in"], 300);
    let token = reply.token();
    let claims = claims_of(&token);
    assert_eq!(claims["iss"], "pacs-rs");
    assert_eq!(claims["aud"], "pacs-rs");
    assert_eq!(claims["sub"], id.as_str());
    assert_eq!(claims["client_id"], id.as_str());
    assert_eq!(claims["scope"], "read write remote");
    assert!(claims["jti"].as_str().is_some_and(|jti| !jti.is_empty()));
    assert_eq!(
        claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap(),
        300
    );
    let listed = get(&app, "/clients", Some(&admin)).await;
    assert!(listed.body[0]["last_token_ms"].as_i64().is_some());

    // The token does what an editor may: read and upload, not delete.
    assert_eq!(
        get(&app, "/studies", Some(&token)).await.status,
        StatusCode::OK
    );
    assert_eq!(
        get(&app, "/dicom-web/studies", Some(&token)).await.status,
        StatusCode::OK
    );
    let file = RealSeries::new(98, 1).slice_bytes(0);
    let upload = send(
        &app,
        Method::POST,
        "/instances",
        Some(&token),
        None,
        Some(file),
    )
    .await;
    assert_eq!(upload.status, StatusCode::OK, "{}", upload.body);
    let study = get(&app, "/studies", Some(&token)).await.body[0]
        .as_str()
        .unwrap()
        .to_owned();
    let delete = async |token: &str| {
        send(
            &app,
            Method::DELETE,
            &format!("/studies/{study}"),
            Some(token),
            None,
            None,
        )
        .await
    };
    // Refused with 401 and the reason in the challenge, as ITI-72 asks.
    let refused = delete(&token).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED, "{}", refused.body);
    assert_eq!(
        refused.headers["www-authenticate"],
        "Bearer error=\"insufficient_scope\""
    );

    // Nothing of the accounts, whatever the role: not the users, the clients, or /auth.
    let (root_id, root_secret) = make_client(&app, &admin, "Everything", "admin").await;
    let root = ask_token(&app, Some((&root_id, &root_secret)), GRANT).await;
    assert_eq!(root.body["scope"], "read write remote delete");
    let root = root.token();
    for (method, path) in [
        (Method::GET, "/users"),
        (Method::GET, "/clients"),
        (Method::POST, "/clients"),
        (Method::GET, "/auth/me"),
        (Method::GET, "/roles"),
        (Method::POST, "/auth/download-token"),
        (Method::POST, "/auth/password"),
    ] {
        let reply = send(
            &app,
            method.clone(),
            path,
            Some(&root),
            Some(json!({})),
            None,
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method} {path}");
        assert_eq!(
            reply.headers["www-authenticate"], "Bearer error=\"insufficient_scope\"",
            "{method} {path}"
        );
    }

    // In the header only: not in a URL, and the secret itself is not a token.
    let in_url = get(&app, &format!("/studies?token={token}"), None).await;
    assert_eq!(in_url.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        get(&app, "/studies", Some(&secret)).await.status,
        StatusCode::UNAUTHORIZED
    );

    // A narrower scope on request: this token only reads.
    let narrow = ask_token(&app, Some((&id, &secret)), &format!("{GRANT}&scope=read")).await;
    assert_eq!(narrow.body["scope"], "read");
    let narrow = narrow.token();
    assert_eq!(
        get(&app, "/studies", Some(&narrow)).await.status,
        StatusCode::OK
    );
    let file = RealSeries::new(98, 1).slice_bytes(0);
    let upload = send(
        &app,
        Method::POST,
        "/instances",
        Some(&narrow),
        None,
        Some(file),
    )
    .await;
    assert_eq!(upload.status, StatusCode::UNAUTHORIZED);

    // A new role holds at the next request, for the tokens the client already has.
    let put = async |path: String, body: Value| {
        send(&app, Method::PUT, &path, Some(&admin), Some(body), None).await
    };
    let moved = put(
        format!("/clients/{id}"),
        json!({"role": "reader", "name": "Reader now"}),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
    assert_eq!(moved.body["role"], "reader");
    assert_eq!(moved.body["name"], "Reader now");
    assert!(moved.body.get("secret").is_none());
    assert_eq!(
        get(&app, "/studies", Some(&token)).await.status,
        StatusCode::OK
    );
    let file = RealSeries::new(98, 1).slice_bytes(0);
    let upload = send(
        &app,
        Method::POST,
        "/instances",
        Some(&token),
        None,
        Some(file),
    )
    .await;
    assert_eq!(upload.status, StatusCode::UNAUTHORIZED);
    // ...and so does a permission taken from its role.
    let emptied = put("/roles/reader".into(), json!({"permissions": []})).await;
    assert_eq!(emptied.status, StatusCode::OK, "{}", emptied.body);
    assert_eq!(
        get(&app, "/studies", Some(&token)).await.status,
        StatusCode::UNAUTHORIZED
    );
    let nothing = ask_token(&app, Some((&id, &secret)), GRANT).await;
    assert_eq!(nothing.status, StatusCode::BAD_REQUEST);
    assert_eq!(nothing.body["error"], "invalid_scope");
    put("/roles/reader".into(), json!({"permissions": ["read"]})).await;
    assert_eq!(
        get(&app, "/studies", Some(&token)).await.status,
        StatusCode::OK
    );

    // A role a client has cannot be removed.
    let in_use = send(
        &app,
        Method::DELETE,
        "/roles/reader",
        Some(&admin),
        None,
        None,
    )
    .await;
    assert_eq!(in_use.status, StatusCode::CONFLICT, "{}", in_use.body);

    // Removed: its tokens stop at once, and its secret gets no more.
    assert_eq!(delete(&root).await.status, StatusCode::OK);
    let remove = async |id: &str| {
        send(
            &app,
            Method::DELETE,
            &format!("/clients/{id}"),
            Some(&admin),
            None,
            None,
        )
        .await
    };
    assert_eq!(remove(&id).await.status, StatusCode::NO_CONTENT);
    assert_eq!(remove(&id).await.status, StatusCode::NOT_FOUND);
    let after = get(&app, "/studies", Some(&token)).await;
    assert_eq!(after.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        after.headers["www-authenticate"],
        "Bearer error=\"invalid_token\""
    );
    let again = ask_token(&app, Some((&id, &secret)), GRANT).await;
    assert_eq!(again.status, StatusCode::UNAUTHORIZED);
    assert_eq!(again.body["error"], "invalid_client");
    // the other client is untouched
    assert_eq!(
        get(&app, "/studies", Some(&root)).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_client_from_secret_to_revocation_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    a_client_from_secret_to_revocation(settings_in(dir.path(), &[])).await;
}

#[tokio::test]
async fn a_client_from_secret_to_revocation_on_postgres() {
    let dir = tempfile::tempdir().unwrap();
    if let Some(settings) = postgres_settings("pacs_clients_test", dir.path()).await {
        a_client_from_secret_to_revocation(settings).await;
    }
}

#[tokio::test]
async fn token_requests_that_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings_in(
        dir.path(),
        &["--auth-issuer", "https://pacs.test"],
    ))
    .await;
    let admin = token_of(&app, "admin", ADMIN_PASSWORD).await;
    let (id, secret) = make_client(&app, &admin, "Viewer", "reader").await;
    let (root_id, root_secret) = make_client(&app, &admin, "Everything", "admin").await;
    let own = Some((id.as_str(), secret.as_str()));

    // Who is asking: 401 invalid_client with the Basic challenge, the same for every cause.
    let other_secret = "0".repeat(64);
    for credentials in [
        None,
        Some((id.as_str(), other_secret.as_str())),
        Some((id.as_str(), "")),
        Some(("no-such-client", secret.as_str())),
        Some((root_id.as_str(), secret.as_str())),
    ] {
        let reply = ask_token(&app, credentials, GRANT).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{credentials:?}");
        assert_eq!(reply.body["error"], "invalid_client", "{credentials:?}");
        assert_eq!(reply.headers["www-authenticate"], "Basic");
        assert_eq!(reply.headers["cache-control"], "no-store");
        assert!(reply.body.get("access_token").is_none());
    }
    // the secret in the form, or a Bearer header, is not the Basic header
    let in_form = format!("{GRANT}&client_id={id}&client_secret={secret}");
    assert_eq!(
        ask_token(&app, None, &in_form).await.body["error"],
        "invalid_client"
    );
    let bearer = Request::post("/auth/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("authorization", format!("Bearer {secret}"))
        .body(Body::from(GRANT))
        .unwrap();
    let reply = app.clone().oneshot(bearer).await.unwrap();
    assert_eq!(reply.status(), StatusCode::UNAUTHORIZED);

    // What is asked: 400 with OAuth's error names.
    let resource = "resource=https%3A%2F%2Fpacs.test";
    for (form, error) in [
        ("", "invalid_request"),
        ("scope=read", "invalid_request"),
        ("grant_type=password", "unsupported_grant_type"),
        ("grant_type=authorization_code", "unsupported_grant_type"),
        (
            "grant_type=client_credentials&grant_type=client_credentials",
            "invalid_request",
        ),
        (
            "grant_type=client_credentials&scope=read&scope=read",
            "invalid_request",
        ),
        ("grant_type=client_credentials&scope=write", "invalid_scope"),
        (
            "grant_type=client_credentials&scope=read+delete",
            "invalid_scope",
        ),
        (
            "grant_type=client_credentials&scope=everything",
            "invalid_scope",
        ),
        ("grant_type=client_credentials&scope=users", "invalid_scope"),
        (
            "grant_type=client_credentials&resource=pacs-rs",
            "invalid_target",
        ),
        (
            "grant_type=client_credentials&requested_token_type=urn:ietf:params:oauth:token-type:saml2",
            "invalid_request",
        ),
    ] {
        let reply = ask_token(&app, own, form).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{form}");
        assert_eq!(reply.body["error"], error, "{form}");
        assert!(reply.body["error_description"].as_str().is_some(), "{form}");
        assert_eq!(reply.headers["cache-control"], "no-store", "{form}");
    }
    // never `users`, even for a client whose role has it
    let users = ask_token(
        &app,
        Some((&root_id, &root_secret)),
        &format!("{GRANT}&scope=users"),
    )
    .await;
    assert_eq!(users.body["error"], "invalid_scope");
    // a JSON body is not a form
    let json = post(
        &app,
        "/auth/token",
        None,
        json!({"grant_type": "client_credentials"}),
    )
    .await;
    assert_eq!(json.status, StatusCode::BAD_REQUEST);
    assert_eq!(json.body["error"], "invalid_request");

    // The optional parameters, when they say what this server does, change nothing.
    let jwt = "requested_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Ajwt";
    let form = format!("{GRANT}&{resource}&{jwt}&scope=read");
    let token = client_token(&app, &id, &secret, &form).await;
    let claims = claims_of(&token);
    assert_eq!(claims["iss"], "https://pacs.test");
    assert_eq!(claims["aud"], "https://pacs.test");
    assert_eq!(
        get(&app, "/studies", Some(&token)).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn clients_are_for_who_manages_users_and_tokens_do_not_cross() {
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings_in(dir.path(), &[])).await;
    let admin = token_of(&app, "admin", ADMIN_PASSWORD).await;
    let tokens = one_of_each(&app, &admin).await;
    let (id, secret) = make_client(&app, &admin, "Viewer", "reader").await;

    // Only a role with `users` sees or changes the clients.
    for role in ["reader", "editor", "manager"] {
        let token = Some(tokens[role].as_str());
        let new = json!({"name": "Mine", "role": "admin"});
        for reply in [
            get(&app, "/clients", token).await,
            post(&app, "/clients", token, new).await,
            send(
                &app,
                Method::DELETE,
                &format!("/clients/{id}"),
                token,
                None,
                None,
            )
            .await,
            send(
                &app,
                Method::PUT,
                &format!("/clients/{id}"),
                token,
                Some(json!({"role": "admin"})),
                None,
            )
            .await,
        ] {
            assert_eq!(
                reply.status,
                StatusCode::FORBIDDEN,
                "{role}: {}",
                reply.body
            );
        }
    }
    assert_eq!(
        get(&app, "/clients", None).await.status,
        StatusCode::UNAUTHORIZED
    );

    // What a client is made of is checked.
    for (new, what) in [
        (json!({"name": "", "role": "reader"}), "no name"),
        (
            json!({"name": "x".repeat(61), "role": "reader"}),
            "a long name",
        ),
        (
            json!({"name": "Viewer", "role": "nobody"}),
            "an unknown role",
        ),
        (json!({"name": "Viewer"}), "no role"),
    ] {
        let reply = post(&app, "/clients", Some(&admin), new).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{what}");
    }
    let unknown_role = send(
        &app,
        Method::PUT,
        &format!("/clients/{id}"),
        Some(&admin),
        Some(json!({"role": "nobody"})),
        None,
    )
    .await;
    assert_eq!(unknown_role.status, StatusCode::UNPROCESSABLE_ENTITY);
    let nobody = send(
        &app,
        Method::PUT,
        "/clients/nobody",
        Some(&admin),
        Some(json!({"name": "x"})),
        None,
    )
    .await;
    assert_eq!(nobody.status, StatusCode::NOT_FOUND);

    // A user's access token is not a client's, and a client's does not refresh or sign in.
    let client = client_token(&app, &id, &secret, GRANT).await;
    let me = get(&app, "/auth/me", Some(&admin)).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(
        sign_in(&app, &id, &secret).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        refresh(&app, &client).await.status,
        StatusCode::UNAUTHORIZED
    );
    // a download token is a user's business
    let download = post(
        &app,
        "/auth/download-token",
        Some(&client),
        json!({"path": "/studies"}),
    )
    .await;
    assert_eq!(download.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_client_token_of_another_server_or_secret_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings_in(dir.path(), &[])).await;
    let admin = token_of(&app, "admin", ADMIN_PASSWORD).await;
    let (id, secret) = make_client(&app, &admin, "Viewer", "reader").await;
    let token = client_token(&app, &id, &secret, GRANT).await;
    assert_eq!(
        get(&app, "/studies", Some(&token)).await.status,
        StatusCode::OK
    );

    // The same index behind a server with another name: the audience is not this one.
    let renamed = start(&settings_in(dir.path(), &["--auth-issuer", "other-pacs"])).await;
    assert_eq!(
        get(&renamed, "/studies", Some(&token)).await.status,
        StatusCode::UNAUTHORIZED
    );
    // A token changed after signing.
    let mut parts: Vec<String> = token.split('.').map(str::to_owned).collect();
    let mut claims = claims_of(&token);
    claims["scope"] = "read write remote delete".into();
    parts[1] = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
    let forged = parts.join(".");
    assert_eq!(
        get(&app, "/studies", Some(&forged)).await.status,
        StatusCode::UNAUTHORIZED
    );
}

/// Every route the description gives the login is one the server routes, with the answer
/// the description names: the sign-in routes of the sessions library included.
#[tokio::test]
async fn the_described_login_routes_are_the_served_ones() {
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings_in(dir.path(), &[])).await;
    let admin = token_of(&app, "admin", ADMIN_PASSWORD).await;
    let api = serde_json::to_value(pacs_server::api::description()).unwrap();
    let mut seen = 0;
    for (path, item) in api["paths"].as_object().unwrap() {
        let of_the_login = ["/auth", "/users", "/roles", "/clients", "/projects"]
            .iter()
            .any(|prefix| path.starts_with(prefix));
        if !of_the_login {
            continue;
        }
        for method in item.as_object().unwrap().keys() {
            seen += 1;
            let method: Method = method.to_uppercase().parse().unwrap();
            let uri = path.replace("{id}", "nothing");
            let reply = send(&app, method.clone(), &uri, Some(&admin), None, None).await;
            // refused by its handler, with a reason, never by the router
            assert_ne!(
                reply.status,
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} {uri}"
            );
            if reply.status == StatusCode::NOT_FOUND {
                described::<Refused>(&reply.body);
            }
        }
    }
    assert_eq!(seen, 21, "the login's requests in the description");

    // the answers, each as its described type
    let me: Me = described(&get(&app, "/auth/me", Some(&admin)).await.body);
    assert_eq!((me.login.as_str(), me.projects), ("admin", None));
    let users: Vec<User> = described(&get(&app, "/users", Some(&admin)).await.body);
    assert_eq!(users.len(), 1);
    let roles: Vec<Role> = described(&get(&app, "/roles", Some(&admin)).await.body);
    assert!(roles.iter().any(|role| role.id == "admin" && role.locked));
    let made = post(
        &app,
        "/roles",
        Some(&admin),
        json!({"name": "Night shift", "permissions": ["read"]}),
    )
    .await;
    assert_eq!(described::<Role>(&made.body).id, "night-shift");
    let (id, secret) = make_client(&app, &admin, "Pipeline", "reader").await;
    let tools: Vec<Tool> = described(&get(&app, "/clients", Some(&admin)).await.body);
    assert!(tools.len() == 1 && tools[0].secret.is_none());
    client_token(&app, &id, &secret, "grant_type=client_credentials").await;
    let refused = ask_token(
        &app,
        Some((&id, "not-the-secret")),
        "grant_type=client_credentials",
    )
    .await;
    assert_eq!(
        described::<TokenRefused>(&refused.body).error,
        "invalid_client"
    );
    let link = download_token(&app, &admin, "/studies").await;
    assert!(described::<DownloadToken>(&link.body).expires_in > 0);
    let projects: Vec<String> = described(&get(&app, "/projects", Some(&admin)).await.body);
    assert!(projects.is_empty());
    // refusals
    let wrong = sign_in(&app, "admin", "not-the-password").await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    described::<Refused>(&wrong.body);
    described::<Refused>(&get(&app, "/auth/me", None).await.body);
}
