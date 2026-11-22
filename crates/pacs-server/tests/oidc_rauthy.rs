//! Against a real OpenID provider: Rauthy, started with the bootstrap in `tests/rauthy/`
//! (a client for pacs-rs, a tool, a user and an API key for these tests). Runs only when
//! `PACS_TEST_RAUTHY_URL` is Rauthy's issuer, e.g. `http://localhost:18088/auth/v1/`; CI
//! starts it so:
//!
//! ```text
//! docker run -d -p 18088:8080 -e LOCAL_TEST=true -e LISTEN_SCHEME=http \
//!   -e PUB_URL=localhost:18088 -e BOOTSTRAP_DIR=/app/bootstrap \
//!   -v $PWD/crates/pacs-server/tests/rauthy/bootstrap:/app/bootstrap:ro \
//!   ghcr.io/sebadob/rauthy:0.37.2
//! ```
//!
//! The browser's sign-in goes through Rauthy's own page, which needs a browser; what these
//! tests cover is what pacs-rs asks of the provider without one: its discovery document and
//! keys, its authorization endpoint taking pacs-rs's request, and a tool's access tokens.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use clap::Parser;
use pacs_server::auth::open_login;
use pacs_server::{Settings, app_with_login, open_archive};
use serde_json::Value;
use tower::ServiceExt;

const CLIENT_SECRET: &str = "pacsTestClientSecret0123456789abcdefghijklmnopqrstuvwxyzABCDEFGH";
const TOOL: &str = "tool-test";
const TOOL_SECRET: &str = "toolTestClientSecret0123456789abcdefghijklmnopqrstuvwxyzABCDEFGH";
const API_KEY: &str =
    "API-Key pacs-tests$pacsTestsApiKey0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMN";
/// This server as its tools' tokens must name it: the origin of the redirect URL.
const AUDIENCE: &str = "http://localhost:8042";

fn issuer() -> Option<String> {
    let issuer = std::env::var("PACS_TEST_RAUTHY_URL")
        .ok()
        .filter(|v| !v.is_empty());
    if issuer.is_none() {
        eprintln!("PACS_TEST_RAUTHY_URL not set; skipping");
    }
    issuer
}

fn settings(dir: &std::path::Path, issuer: &str) -> Settings {
    let index = dir.join("index.sqlite").display().to_string();
    let storage = format!("file:{}", dir.join("files").display());
    Settings::try_parse_from([
        "pacs-server",
        "--index",
        &index,
        "--storage",
        &storage,
        "--auth",
        "true",
        "--auth-cookie-secure",
        "false",
        "--oidc-issuer",
        issuer,
        "--oidc-client-id",
        "pacs",
        "--oidc-redirect-url",
        "http://localhost:8042/auth/oidc/callback",
    ])
    .unwrap()
}

/// pacs-rs on Rauthy, with the tool registered as `role`.
async fn start(dir: &std::path::Path, issuer: &str, role: &str) -> Router {
    let settings = settings(dir, issuer);
    let archive = open_archive(&settings).await.unwrap();
    archive
        .index()
        .create_client(TOOL, "A tool under test", "", role, "tests")
        .await
        .unwrap();
    let login = open_login(&settings, archive.index(), |name| match name {
        "PACS_AUTH_SECRET" => Some("a-signing-secret-of-at-least-32-bytes".into()),
        "PACS_OIDC_CLIENT_SECRET" => Some(CLIENT_SECRET.into()),
        _ => None,
    })
    .await
    .expect("pacs-rs reads Rauthy's discovery document and keys");
    app_with_login(archive, &settings, login).unwrap()
}

async fn discovery(http: &reqwest::Client, issuer: &str) -> Value {
    http.get(format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    ))
    .send()
    .await
    .unwrap()
    .json()
    .await
    .unwrap()
}

/// Rauthy issues a token for a resource only when the client allows it, which its
/// bootstrap files cannot say: set through its API, once.
async fn allow_resource(http: &reqwest::Client, issuer: &str) {
    let url = format!("{}/clients/{TOOL}", issuer.trim_end_matches('/'));
    let mut client: Value = http
        .get(&url)
        .header("authorization", API_KEY)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    client["allowed_resources"] = serde_json::json!([AUDIENCE]);
    http.put(&url)
        .header("authorization", API_KEY)
        .json(&client)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
}

/// The tool's access token, for `resource` when given (RFC 8707). Rauthy takes the client's
/// credentials in the form.
async fn tool_token(http: &reqwest::Client, issuer: &str, resource: Option<&str>) -> String {
    let endpoint = discovery(http, issuer).await["token_endpoint"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut form = vec![
        ("grant_type", "client_credentials"),
        ("client_id", TOOL),
        ("client_secret", TOOL_SECRET),
    ];
    if let Some(resource) = resource {
        form.push(("resource", resource));
    }
    let answer: Value = http
        .post(endpoint)
        .form(&form)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    answer["access_token"]
        .as_str()
        .unwrap_or_else(|| panic!("no token: {answer}"))
        .to_owned()
}

async fn status(app: &Router, method: Method, path: &str, token: &str) -> StatusCode {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(request).await.unwrap().status()
}

#[tokio::test]
async fn a_tools_token_from_rauthy_for_this_server_opens_what_its_role_allows() {
    let Some(issuer) = issuer() else { return };
    let http = reqwest::Client::new();
    allow_resource(&http, &issuer).await;
    let dir = tempfile::tempdir().unwrap();
    let app = start(dir.path(), &issuer, "reader").await;

    let token = tool_token(&http, &issuer, Some(AUDIENCE)).await;
    assert_eq!(
        status(&app, Method::GET, "/studies", &token).await,
        StatusCode::OK
    );
    // a reader's tool deletes nothing
    assert_eq!(
        status(&app, Method::DELETE, "/studies/x", &token).await,
        StatusCode::UNAUTHORIZED
    );
    // without the resource, Rauthy's token is for the tool alone: not for this server
    let own = tool_token(&http, &issuer, None).await;
    assert_eq!(
        status(&app, Method::GET, "/studies", &own).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn rauthy_takes_the_sign_in_pacs_rs_starts() {
    let Some(issuer) = issuer() else { return };
    let dir = tempfile::tempdir().unwrap();
    let app = start(dir.path(), &issuer, "reader").await;
    let started = app
        .clone()
        .oneshot(
            Request::get("/auth/oidc/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::SEE_OTHER);
    let to = started.headers()["location"].to_str().unwrap().to_owned();
    // Rauthy checks the client, the redirect URI and the PKCE method before it shows its
    // sign-in page; any of them wrong is an error answer instead
    let page = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(&to)
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), reqwest::StatusCode::OK, "{to}");
}
