//! The login (`PACS_AUTH`): what is open, what each role may do on the real routes, the users
//! API, and the starts that must fail. On SQLite always, and on Postgres when
//! `PACS_TEST_POSTGRES_URL` names a server.

use std::collections::HashMap;
use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt;
use pacs_core::synthetic::SyntheticSeries;
use pacs_server::auth::open_login;
use pacs_server::{Settings, app, app_with_login, open_archive};
use serde_json::{Value, json};
use tower::ServiceExt;

const SECRET: &str = "a-signing-secret-of-at-least-32-bytes";
const ADMIN_PASSWORD: &str = "admin-password-1";

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
    let series = SyntheticSeries::new(91, 2);

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

#[tokio::test]
async fn the_whole_login_on_postgres() {
    let Ok(server) = std::env::var("PACS_TEST_POSTGRES_URL") else {
        eprintln!("PACS_TEST_POSTGRES_URL not set; skipping");
        return;
    };
    let server = server.trim_end_matches('/');
    let name = format!("pacs_auth_test_{}", std::process::id());
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
    let dir = tempfile::tempdir().unwrap();
    the_whole_login(settings_with(&format!("{server}/{name}"), dir.path(), &[])).await;
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
        let bytes = SyntheticSeries::new(seed, 1).slice_bytes(0);
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
        json!(["read", "write", "remote", "delete", "users"])
    );
    let me = get(&app, "/auth/me", Some(&admin)).await;
    assert_eq!(
        me.body["permissions"],
        json!(["read", "write", "remote", "delete", "users"])
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
    let series = SyntheticSeries::new(97, 1);
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
