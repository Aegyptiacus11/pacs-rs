//! Signing in through an OpenID provider (`PACS_OIDC_ISSUER`), against a provider made here:
//! one that answers as the specification says, and that can be made to answer wrong, which a
//! real provider cannot. The real one (Rauthy) is in `oidc_rauthy.rs`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::extract::{Form, State};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use clap::Parser;
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use pacs_server::auth::open_login;
use pacs_server::{Settings, app_with_login, open_archive};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const SECRET: &str = "a-signing-secret-of-at-least-32-bytes";
const CLIENT_ID: &str = "pacs";
const CLIENT_SECRET: &str = "the-client-secret";

/// Makes the ID token the token endpoint answers with, from the nonce of the sign-in.
type Mint = Arc<dyn Fn(&str) -> String + Send + Sync>;

#[derive(Default)]
struct Shared {
    /// code -> (PKCE challenge, nonce)
    pending: HashMap<String, (String, String)>,
    mint: Option<Mint>,
    /// opaque token -> what the introspection endpoint answers for it
    introspected: HashMap<String, Value>,
}

/// A provider on a port of this machine.
struct Provider {
    issuer: String,
    key: Arc<rcgen::KeyPair>,
    shared: Arc<Mutex<Shared>>,
}

impl Provider {
    /// `named`: the issuer its discovery document claims, when not its own address.
    async fn start(named: Option<&str>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address: SocketAddr = listener.local_addr().unwrap();
        let issuer = format!("http://127.0.0.1:{}", address.port());
        let key = Arc::new(rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap());
        let shared = Arc::new(Mutex::new(Shared::default()));
        let discovery = json!({
            "issuer": named.unwrap_or(&issuer),
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{issuer}/token"),
            "jwks_uri": format!("{issuer}/jwks"),
            "introspection_endpoint": format!("{issuer}/introspect"),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["EdDSA"],
        });
        let jwks = json!({"keys": [{
            "kty": "OKP",
            "crv": "Ed25519",
            "kid": "k1",
            "alg": "EdDSA",
            "use": "sig",
            "x": URL_SAFE_NO_PAD.encode(key.public_key_raw()),
        }]});
        let app = Router::new()
            .route(
                "/.well-known/openid-configuration",
                get(move || async move { axum::Json(discovery) }),
            )
            .route("/jwks", get(move || async move { axum::Json(jwks) }))
            .route("/token", post(token))
            .route("/introspect", post(introspect))
            .with_state(shared.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            issuer,
            key,
            shared,
        }
    }

    /// An ID token signed with the published key, with these claims.
    fn signed(&self, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some("k1".into());
        let key = EncodingKey::from_ed_der(&self.key.serialize_der());
        jsonwebtoken::encode(&header, claims, &key).unwrap()
    }

    /// The claims of a good ID token for Ada.
    fn claims(&self, nonce: &str) -> Value {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        json!({
            "iss": self.issuer,
            "sub": "sub-ada",
            "aud": CLIENT_ID,
            "exp": now + 300,
            "iat": now,
            "nonce": nonce,
            "email": "Ada@Example.com",
            "email_verified": true,
            "preferred_username": "ada",
        })
    }

    /// The token endpoint answers with a token made by `change` from good claims.
    fn minting(self: &Arc<Self>, change: impl Fn(&mut Value) + Send + Sync + 'static) {
        let provider = self.clone();
        let mint: Mint = Arc::new(move |nonce| {
            let mut claims = provider.claims(nonce);
            change(&mut claims);
            provider.signed(&claims)
        });
        self.shared.lock().unwrap().mint = Some(mint);
    }
}

/// The introspection endpoint (RFC 7662): this server's client credentials, then what the
/// test said of the token, or inactive.
async fn introspect(
    State(shared): State<Arc<Mutex<Shared>>>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> axum::response::Response {
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"))
    );
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(basic.as_str()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let token = form.get("token").cloned().unwrap_or_default();
    let answer = shared.lock().unwrap().introspected.get(&token).cloned();
    axum::Json(answer.unwrap_or(json!({"active": false}))).into_response()
}

/// The token endpoint: the client's credentials, the code, and the PKCE verifier checked as
/// RFC 6749 4.1.3 and RFC 7636 4.6 say.
async fn token(
    State(shared): State<Arc<Mutex<Shared>>>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> axum::response::Response {
    let refused = |error: &str| {
        (StatusCode::BAD_REQUEST, axum::Json(json!({"error": error}))).into_response()
    };
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT_ID}:{CLIENT_SECRET}"))
    );
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(basic.as_str()) {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({"error": "invalid_client"})),
        )
            .into_response();
    }
    if form.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return refused("unsupported_grant_type");
    }
    let mut shared = shared.lock().unwrap();
    let Some((challenge, nonce)) = form
        .get("code")
        .and_then(|code| shared.pending.remove(code))
    else {
        return refused("invalid_grant");
    };
    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
    if URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != challenge {
        return refused("invalid_grant");
    }
    let mint = shared.mint.clone().expect("the test says what to answer");
    axum::Json(json!({
        "access_token": "opaque",
        "token_type": "Bearer",
        "expires_in": 300,
        "id_token": mint(&nonce),
    }))
    .into_response()
}

fn settings(dir: &Path, issuer: &str, extra: &[&str]) -> Settings {
    let index = dir.join("db/index.sqlite").display().to_string();
    let storage = format!("file:{}", dir.join("files").display());
    let mut args: Vec<String> = [
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
        CLIENT_ID,
        "--oidc-redirect-url",
        "http://localhost:8042/auth/oidc/callback",
        "--oidc-admins",
        "boss@example.com",
    ]
    .map(String::from)
    .to_vec();
    args.extend(extra.iter().map(|s| s.to_string()));
    Settings::try_parse_from(args).unwrap()
}

fn env(name: &str) -> Option<String> {
    match name {
        "PACS_AUTH_SECRET" => Some(SECRET.into()),
        "PACS_OIDC_CLIENT_SECRET" => Some(CLIENT_SECRET.into()),
        _ => None,
    }
}

async fn start(settings: &Settings) -> anyhow::Result<Router> {
    let archive = open_archive(settings).await?;
    let login = open_login(settings, archive.index(), env).await?;
    app_with_login(archive, settings, login)
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl Reply {
    fn location(&self) -> &str {
        self.headers["location"].to_str().unwrap()
    }

    /// The value of a cookie this answer sets.
    fn cookie(&self, name: &str) -> Option<String> {
        self.headers
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap())
            .find(|v| v.starts_with(&format!("{name}=")))
            .map(|v| v.split(';').next().unwrap()[name.len() + 1..].to_owned())
    }

    fn set_cookie(&self, name: &str) -> String {
        self.headers
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .find(|v| v.starts_with(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no cookie {name}"))
    }
}

async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<Value>,
) -> Reply {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Reply {
        status,
        headers,
        body,
    }
}

fn query_of(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

/// The browser's way through: start, the provider's sign-in (the code it would give), back.
/// `tamper` may change the callback's query before it is sent.
async fn sign_in(
    app: &Router,
    provider: &Provider,
    tamper: impl FnOnce(&mut Vec<(String, String)>, &mut Option<String>),
) -> Reply {
    let started = send(
        app,
        Method::GET,
        "/auth/oidc/start?next=studies/1.2.3",
        &[],
        None,
    )
    .await;
    assert_eq!(started.status, StatusCode::SEE_OTHER);
    let sent = query_of(started.location());
    assert!(
        started
            .location()
            .starts_with(&format!("{}/authorize?", provider.issuer))
    );
    assert_eq!(sent["response_type"], "code");
    assert_eq!(sent["client_id"], CLIENT_ID);
    assert_eq!(sent["code_challenge_method"], "S256");
    assert_eq!(
        sent["redirect_uri"],
        "http://localhost:8042/auth/oidc/callback"
    );
    assert_eq!(sent["scope"], "openid email profile");
    let state_cookie = started.set_cookie("pacs_oidc");
    assert!(
        state_cookie.contains("Path=/auth/oidc; HttpOnly; SameSite=Lax"),
        "{state_cookie}"
    );
    provider.shared.lock().unwrap().pending.insert(
        "code-1".into(),
        (sent["code_challenge"].clone(), sent["nonce"].clone()),
    );
    let mut query = vec![
        ("code".to_owned(), "code-1".to_owned()),
        ("state".to_owned(), sent["state"].clone()),
    ];
    let mut cookie = started.cookie("pacs_oidc");
    tamper(&mut query, &mut cookie);
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(query)
        .finish();
    let headers: Vec<(&str, String)> = cookie
        .map(|c| ("cookie", format!("pacs_oidc={c}")))
        .into_iter()
        .collect();
    send(
        app,
        Method::GET,
        &format!("/auth/oidc/callback?{query}"),
        &headers,
        None,
    )
    .await
}

/// The access token a sign-in's refresh cookie gives, and who it says.
async fn session_of(app: &Router, back: &Reply) -> (String, Value) {
    let refresh = back.cookie("refresh_token").expect("a refresh cookie");
    let renewed = send(
        app,
        Method::POST,
        "/auth/refresh",
        &[("cookie", format!("refresh_token={refresh}"))],
        None,
    )
    .await;
    assert_eq!(renewed.status, StatusCode::OK, "{}", renewed.body);
    let token = renewed.body["access_token"].as_str().unwrap().to_owned();
    let me = send(
        app,
        Method::GET,
        "/auth/me",
        &[("authorization", format!("Bearer {token}"))],
        None,
    )
    .await;
    assert_eq!(me.status, StatusCode::OK);
    (token, me.body)
}

fn refused_for(back: &Reply, reason: &str) {
    assert_eq!(back.status, StatusCode::SEE_OTHER);
    assert_eq!(
        back.location(),
        format!("../../ui/?signin_error={reason}"),
        "{:?}",
        back.headers
    );
    assert_eq!(back.cookie("refresh_token"), None);
    // the sign-in under way is over either way
    assert!(back.set_cookie("pacs_oidc").contains("Max-Age=0"));
}

#[tokio::test]
async fn the_first_sign_in_makes_an_account_and_the_next_finds_it() {
    let provider = Arc::new(Provider::start(None).await);
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings(dir.path(), &provider.issuer, &[]))
        .await
        .unwrap();

    let methods = send(&app, Method::GET, "/auth/methods", &[], None).await;
    assert_eq!(methods.body, json!({"password": false, "provider": true}));

    provider.minting(|_| {});
    let back = sign_in(&app, &provider, |_, _| {}).await;
    assert_eq!(back.status, StatusCode::SEE_OTHER, "{:?}", back.headers);
    assert_eq!(back.location(), "../../ui/studies/1.2.3");
    assert!(
        back.set_cookie("refresh_token")
            .contains("Path=/auth; HttpOnly")
    );
    let (_, me) = session_of(&app, &back).await;
    assert_eq!(me["login"], "ada@example.com");
    assert_eq!(me["role"], "reader");

    // the same subject again: the same account, whatever its email is now
    provider.minting(|claims| claims["email"] = json!("ada.lovelace@example.com"));
    let again = sign_in(&app, &provider, |_, _| {}).await;
    let (_, me_again) = session_of(&app, &again).await;
    assert_eq!(me_again["id"], me["id"]);
    assert_eq!(me_again["login"], "ada@example.com");
}

#[tokio::test]
async fn a_listed_verified_address_is_an_administrator_and_an_unverified_one_is_not() {
    let provider = Arc::new(Provider::start(None).await);
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings(dir.path(), &provider.issuer, &[]))
        .await
        .unwrap();

    // unverified: not vouched for, so neither its login nor an administrator
    provider.minting(|claims| {
        claims["sub"] = json!("sub-pretender");
        claims["email"] = json!("boss@example.com");
        claims["email_verified"] = json!(false);
        claims["preferred_username"] = json!("pretender");
    });
    let (_, me) = session_of(&app, &sign_in(&app, &provider, |_, _| {}).await).await;
    assert_eq!(
        (me["login"].as_str(), me["role"].as_str()),
        (Some("pretender"), Some("reader"))
    );
    // "true" as a string is not a verification either (Core 5.1: a boolean)
    provider.minting(|claims| {
        claims["sub"] = json!("sub-pretender-2");
        claims["email"] = json!("boss@example.com");
        claims["email_verified"] = json!("true");
        claims["preferred_username"] = json!("pretender2");
    });
    let (_, me) = session_of(&app, &sign_in(&app, &provider, |_, _| {}).await).await;
    assert_eq!(me["role"], "reader");

    provider.minting(|claims| {
        claims["sub"] = json!("sub-boss");
        claims["email"] = json!("Boss@Example.com");
    });
    let (token, me) = session_of(&app, &sign_in(&app, &provider, |_, _| {}).await).await;
    assert_eq!(
        (me["login"].as_str(), me["role"].as_str()),
        (Some("boss@example.com"), Some("admin"))
    );
    let users = send(
        &app,
        Method::GET,
        "/users",
        &[("authorization", format!("Bearer {token}"))],
        None,
    )
    .await;
    assert_eq!(users.status, StatusCode::OK);
    assert_eq!(users.body.as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn no_password_signs_anyone_in_when_a_provider_is_set() {
    let provider = Arc::new(Provider::start(None).await);
    let dir = tempfile::tempdir().unwrap();
    // no first administrator is made from PACS_ADMIN_PASSWORD, even when it is given
    let app = start(&settings(dir.path(), &provider.issuer, &[]))
        .await
        .unwrap();
    let login = send(
        &app,
        Method::POST,
        "/auth/login",
        &[],
        Some(json!({"email": "admin", "password": "admin-password-1"})),
    )
    .await;
    assert_eq!(login.status, StatusCode::FORBIDDEN);

    provider.minting(|claims| claims["email"] = json!("boss@example.com"));
    let (token, _) = session_of(&app, &sign_in(&app, &provider, |_, _| {}).await).await;
    let bearer = [("authorization", format!("Bearer {token}"))];
    let made = send(
        &app,
        Method::POST,
        "/users",
        &bearer,
        Some(json!({"login": "bob", "password": "a-long-password", "role": "reader"})),
    )
    .await;
    assert_eq!(made.status, StatusCode::FORBIDDEN);
    let changed = send(
        &app,
        Method::POST,
        "/auth/password",
        &bearer,
        Some(json!({"current_password": "x", "new_password": "a-long-password"})),
    )
    .await;
    assert_eq!(changed.status, StatusCode::FORBIDDEN);
    // a tool's own secret neither
    let tool = send(
        &app,
        Method::POST,
        "/auth/token",
        &[("content-type", "application/x-www-form-urlencoded".into())],
        None,
    )
    .await;
    assert_eq!(tool.status, StatusCode::BAD_REQUEST);
    assert_eq!(tool.body["error"], "unauthorized_client");
}

#[tokio::test]
async fn a_callback_without_its_sign_in_is_refused() {
    let provider = Arc::new(Provider::start(None).await);
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings(dir.path(), &provider.issuer, &[]))
        .await
        .unwrap();
    provider.minting(|_| {});

    // no state cookie: a callback this browser did not start (CSRF)
    refused_for(
        &sign_in(&app, &provider, |_, cookie| *cookie = None).await,
        "expired",
    );
    // another state than the cookie's
    let back = sign_in(&app, &provider, |query, _| query[1].1 = "forged".into()).await;
    refused_for(&back, "expired");
    // a cookie that is not ours
    refused_for(
        &sign_in(&app, &provider, |_, cookie| *cookie = Some("x.y.z".into())).await,
        "expired",
    );
    // the provider said no
    let back = sign_in(&app, &provider, |query, _| {
        query[0] = ("error".into(), "access_denied".into());
    })
    .await;
    refused_for(&back, "refused");
    // a code the provider does not know
    let back = sign_in(&app, &provider, |query, _| query[0].1 = "code-2".into()).await;
    refused_for(&back, "refused");
}

#[tokio::test]
async fn an_id_token_that_fails_a_check_is_refused() {
    let provider = Arc::new(Provider::start(None).await);
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings(dir.path(), &provider.issuer, &[]))
        .await
        .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    type Change = Box<dyn Fn(&mut Value) + Send + Sync>;
    let cases: Vec<(&str, Change)> = vec![
        (
            "another nonce",
            Box::new(|c| c["nonce"] = json!("replayed")),
        ),
        (
            "no nonce",
            Box::new(|c| {
                c.as_object_mut().unwrap().remove("nonce");
            }),
        ),
        (
            "another issuer",
            Box::new(|c| c["iss"] = json!("http://127.0.0.1:1")),
        ),
        (
            "another audience",
            Box::new(|c| c["aud"] = json!("other-client")),
        ),
        ("expired", Box::new(move |c| c["exp"] = json!(now - 3600))),
        (
            "not yet valid",
            Box::new(move |c| c["nbf"] = json!(now + 3600)),
        ),
        (
            "no iat",
            Box::new(|c| {
                c.as_object_mut().unwrap().remove("iat");
            }),
        ),
        (
            "no subject",
            Box::new(|c| {
                c.as_object_mut().unwrap().remove("sub");
            }),
        ),
        (
            "issued to another client",
            Box::new(|c| c["azp"] = json!("other-client")),
        ),
        (
            "several audiences and no azp",
            Box::new(|c| c["aud"] = json!([CLIENT_ID, "other-client"])),
        ),
    ];
    for (case, change) in cases {
        provider.minting(change);
        let back = sign_in(&app, &provider, |_, _| {}).await;
        assert_eq!(back.location(), "../../ui/?signin_error=token", "{case}");
    }
    // several audiences with this client as azp pass
    provider.minting(|c| {
        c["aud"] = json!([CLIENT_ID, "other-client"]);
        c["azp"] = json!(CLIENT_ID);
    });
    let back = sign_in(&app, &provider, |_, _| {}).await;
    assert_eq!(back.location(), "../../ui/studies/1.2.3");

    // signatures: an HMAC keyed with the client's secret, an unknown key, none at all
    let forged = |header: Header, key: EncodingKey| {
        let minter = provider.clone();
        let mint: Mint = Arc::new(move |nonce| {
            jsonwebtoken::encode(&header, &minter.claims(nonce), &key).unwrap()
        });
        provider.shared.lock().unwrap().mint = Some(mint);
    };
    forged(
        Header::new(Algorithm::HS256),
        EncodingKey::from_secret(CLIENT_SECRET.as_bytes()).unwrap(),
    );
    refused_for(&sign_in(&app, &provider, |_, _| {}).await, "token");
    let stranger = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some("k1".into());
    forged(header, EncodingKey::from_ed_der(&stranger.serialize_der()));
    refused_for(&sign_in(&app, &provider, |_, _| {}).await, "token");
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some("k2".into());
    forged(header, EncodingKey::from_ed_der(&stranger.serialize_der()));
    refused_for(&sign_in(&app, &provider, |_, _| {}).await, "token");
    let unsigned = {
        let provider = provider.clone();
        Arc::new(move |nonce: &str| {
            let head = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
            let body = URL_SAFE_NO_PAD.encode(provider.claims(nonce).to_string());
            format!("{head}.{body}.")
        }) as Mint
    };
    provider.shared.lock().unwrap().mint = Some(unsigned);
    refused_for(&sign_in(&app, &provider, |_, _| {}).await, "token");
}

#[tokio::test]
async fn a_deactivated_account_or_a_taken_login_gets_no_session() {
    let provider = Arc::new(Provider::start(None).await);
    let dir = tempfile::tempdir().unwrap();
    let app = start(&settings(dir.path(), &provider.issuer, &[]))
        .await
        .unwrap();
    provider.minting(|claims| {
        claims["sub"] = json!("sub-boss");
        claims["email"] = json!("boss@example.com");
    });
    let (token, _) = session_of(&app, &sign_in(&app, &provider, |_, _| {}).await).await;
    let bearer = [("authorization", format!("Bearer {token}"))];

    provider.minting(|_| {});
    let (_, ada) = session_of(&app, &sign_in(&app, &provider, |_, _| {}).await).await;
    let off = send(
        &app,
        Method::PUT,
        &format!("/users/{}", ada["id"].as_str().unwrap()),
        &bearer,
        Some(json!({"active": false})),
    )
    .await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.body);
    refused_for(&sign_in(&app, &provider, |_, _| {}).await, "account");

    // another subject whose unverified user name is a login already taken
    provider.minting(|claims| {
        claims["sub"] = json!("sub-other");
        claims["email_verified"] = json!(false);
        claims["preferred_username"] = json!("boss@example.com");
    });
    refused_for(&sign_in(&app, &provider, |_, _| {}).await, "account");
}

#[tokio::test]
async fn settings_that_cannot_work_stop_the_start() {
    let provider = Provider::start(None).await;
    let elsewhere = Provider::start(Some("https://idp.example")).await;
    let dir = tempfile::tempdir().unwrap();
    let fails = async |settings: Settings, says: &str| {
        let error = format!("{:#}", start(&settings).await.expect_err("a failed start"));
        assert!(error.contains(says), "{error}");
    };
    // the provider names itself otherwise (Discovery 4.3)
    fails(
        settings(dir.path(), &elsewhere.issuer, &[]),
        "the provider names itself",
    )
    .await;
    // not https, and not this machine
    fails(
        settings(dir.path(), "http://idp.example", &[]),
        "must be https",
    )
    .await;
    // nobody answers
    fails(
        settings(dir.path(), "http://127.0.0.1:1", &[]),
        "discovery document",
    )
    .await;
    fails(
        settings(dir.path(), &provider.issuer, &["--oidc-role", "nobody"]),
        "PACS_OIDC_ROLE",
    )
    .await;
    fails(
        settings(dir.path(), &provider.issuer, &["--oidc-scopes", "email"]),
        "openid",
    )
    .await;
    fails(
        settings(
            dir.path(),
            &provider.issuer,
            &["--oidc-admins", "not-an-address"],
        ),
        "PACS_OIDC_ADMINS",
    )
    .await;
    let mut without_client = settings(dir.path(), &provider.issuer, &[]);
    without_client.oidc_client_id = None;
    fails(without_client, "PACS_OIDC_CLIENT_ID").await;
    let mut no_redirect = settings(dir.path(), &provider.issuer, &[]);
    no_redirect.oidc_redirect_url = None;
    fails(no_redirect, "PACS_OIDC_REDIRECT_URL").await;
    let archive = open_archive(&settings(dir.path(), &provider.issuer, &[]))
        .await
        .unwrap();
    let no_secret = open_login(
        &settings(dir.path(), &provider.issuer, &[]),
        archive.index(),
        |name| (name == "PACS_AUTH_SECRET").then(|| SECRET.to_owned()),
    )
    .await;
    assert!(format!("{:#}", no_secret.err().unwrap()).contains("PACS_OIDC_CLIENT_SECRET"));
}

#[tokio::test]
async fn without_a_provider_the_provider_routes_are_not_there() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index.sqlite").display().to_string();
    let storage = format!("file:{}", dir.path().join("files").display());
    let settings = Settings::try_parse_from([
        "pacs-server",
        "--index",
        &index,
        "--storage",
        &storage,
        "--auth",
        "true",
    ])
    .unwrap();
    let archive = open_archive(&settings).await.unwrap();
    let login = open_login(&settings, archive.index(), |name| match name {
        "PACS_AUTH_SECRET" => Some(SECRET.into()),
        "PACS_ADMIN_PASSWORD" => Some("admin-password-1".into()),
        _ => None,
    })
    .await
    .unwrap();
    let app = app_with_login(archive, &settings, login).unwrap();
    let methods = send(&app, Method::GET, "/auth/methods", &[], None).await;
    assert_eq!(methods.body, json!({"password": true, "provider": false}));
    let started = send(&app, Method::GET, "/auth/oidc/start", &[], None).await;
    assert_eq!(started.status, StatusCode::NOT_FOUND);
    let back = send(
        &app,
        Method::GET,
        "/auth/oidc/callback?code=c&state=s",
        &[],
        None,
    )
    .await;
    assert_eq!(back.status, StatusCode::NOT_FOUND);
    // a tool is made here, with a secret, never named after a provider's client
    let signed = send(
        &app,
        Method::POST,
        "/auth/login",
        &[],
        Some(json!({"email": "admin", "password": "admin-password-1"})),
    )
    .await;
    let admin = signed.body["access_token"].as_str().unwrap().to_owned();
    let named = send(
        &app,
        Method::POST,
        "/clients",
        &[("authorization", format!("Bearer {admin}"))],
        Some(json!({"client_id": "tool-1", "name": "Viewer", "role": "reader"})),
    )
    .await;
    assert_eq!(named.status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// Where the tests' pacs-rs is, as the provider's tokens for it name it: the origin of
/// PACS_OIDC_REDIRECT_URL.
const AUDIENCE: &str = "http://localhost:8042";

impl Provider {
    /// A tool's access token, signed with the published key, from good claims changed.
    fn tool_token(&self, change: impl Fn(&mut Value)) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut claims = json!({
            "iss": self.issuer,
            "sub": "tool-1",
            "aud": AUDIENCE,
            "client_id": "tool-1",
            "exp": now + 300,
            "iat": now,
            "jti": "j1",
        });
        change(&mut claims);
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some("k1".into());
        header.typ = Some("at+jwt".into());
        jsonwebtoken::encode(
            &header,
            &claims,
            &EncodingKey::from_ed_der(&self.key.serialize_der()),
        )
        .unwrap()
    }
}

/// A server with an administrator signed in through the provider, and the tool `tool-1`
/// registered with `role`.
async fn with_a_tool(provider: &Arc<Provider>, dir: &Path, role: &str) -> (Router, String) {
    let app = start(&settings(dir, &provider.issuer, &[])).await.unwrap();
    provider.minting(|claims| {
        claims["sub"] = json!("sub-boss");
        claims["email"] = json!("boss@example.com");
    });
    let (admin, _) = session_of(&app, &sign_in(&app, provider, |_, _| {}).await).await;
    let made = send(
        &app,
        Method::POST,
        "/clients",
        &[("authorization", format!("Bearer {admin}"))],
        Some(json!({"client_id": "tool-1", "name": "Viewer", "role": role})),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(made.body["id"], "tool-1");
    // the provider keeps its secret: none here
    assert!(made.body.get("secret").is_none(), "{}", made.body);
    (app, admin)
}

async fn status_with(app: &Router, method: Method, path: &str, token: &str) -> StatusCode {
    send(
        app,
        method,
        path,
        &[("authorization", format!("Bearer {token}"))],
        None,
    )
    .await
    .status
}

#[tokio::test]
async fn a_tools_token_from_the_provider_gets_its_roles_rights_and_no_more() {
    let provider = Arc::new(Provider::start(None).await);
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = with_a_tool(&provider, dir.path(), "reader").await;

    let token = provider.tool_token(|_| {});
    assert_eq!(
        status_with(&app, Method::GET, "/studies", &token).await,
        StatusCode::OK
    );
    // a reader deletes nothing, and a tool manages no account: refused for the token's scope
    // (401 with insufficient_scope, as for this server's own tool tokens)
    let delete = send(
        &app,
        Method::DELETE,
        "/studies/x",
        &[("authorization", format!("Bearer {token}"))],
        None,
    )
    .await;
    assert_eq!(delete.status, StatusCode::UNAUTHORIZED);
    assert!(
        delete.headers["www-authenticate"]
            .to_str()
            .unwrap()
            .contains("insufficient_scope")
    );
    assert_eq!(
        status_with(&app, Method::GET, "/users", &token).await,
        StatusCode::UNAUTHORIZED
    );

    // a provider that names the client in azp only (RFC 9068 asks for client_id)
    let azp = provider.tool_token(|c| {
        c.as_object_mut().unwrap().remove("client_id");
        c["azp"] = json!("tool-1");
    });
    assert_eq!(
        status_with(&app, Method::GET, "/studies", &azp).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_tokens_scopes_narrow_the_role_and_never_widen_it() {
    let provider = Arc::new(Provider::start(None).await);
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = with_a_tool(&provider, dir.path(), "manager").await;
    let with_scope = |scope: &'static str| provider.tool_token(move |c| c["scope"] = json!(scope));

    // no pacs: scope: the whole role
    let whole = with_scope("openid");
    assert_eq!(
        status_with(&app, Method::DELETE, "/studies/x", &whole).await,
        StatusCode::NOT_FOUND
    );
    // pacs:read only: reading, not deleting
    let read = with_scope("openid pacs:read");
    assert_eq!(
        status_with(&app, Method::GET, "/studies", &read).await,
        StatusCode::OK
    );
    assert_eq!(
        status_with(&app, Method::DELETE, "/studies/x", &read).await,
        StatusCode::UNAUTHORIZED
    );
    // a scope the role does not give gives nothing: users is not a tool's, ever
    let users = with_scope("pacs:users");
    assert_eq!(
        status_with(&app, Method::GET, "/users", &users).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_with(&app, Method::GET, "/studies", &users).await,
        StatusCode::UNAUTHORIZED
    );

    // a reader's tool asking for delete gets nothing it did not have
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = with_a_tool(&provider, dir.path(), "reader").await;
    let delete = with_scope("pacs:delete");
    assert_eq!(
        status_with(&app, Method::DELETE, "/studies/x", &delete).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_tools_token_that_fails_a_check_is_refused() {
    let provider = Arc::new(Provider::start(None).await);
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = with_a_tool(&provider, dir.path(), "reader").await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    type Change = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, Change)> = vec![
        (
            "for the tool itself, not this server",
            Box::new(|c| c["aud"] = json!("tool-1")),
        ),
        (
            "another issuer",
            Box::new(|c| c["iss"] = json!("http://127.0.0.1:1")),
        ),
        ("expired", Box::new(move |c| c["exp"] = json!(now - 3600))),
        (
            "a tool not registered here",
            Box::new(|c| c["client_id"] = json!("tool-2")),
        ),
        (
            "no client",
            Box::new(|c| {
                c.as_object_mut().unwrap().remove("client_id");
            }),
        ),
        ("an ID token", Box::new(|c| c["nonce"] = json!("n"))),
    ];
    for (case, change) in cases {
        let token = provider.tool_token(change);
        assert_eq!(
            status_with(&app, Method::GET, "/studies", &token).await,
            StatusCode::UNAUTHORIZED,
            "{case}"
        );
    }
    // signed with the client's secret, which this server holds too
    let mut claims = json!({"iss": provider.issuer, "sub": "tool-1", "aud": AUDIENCE, "client_id": "tool-1", "exp": now + 300});
    claims["iat"] = json!(now);
    let hmac = jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(CLIENT_SECRET.as_bytes()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        status_with(&app, Method::GET, "/studies", &hmac).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn an_opaque_token_is_asked_of_the_providers_introspection_endpoint() {
    let provider = Arc::new(Provider::start(None).await);
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = with_a_tool(&provider, dir.path(), "reader").await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let good = json!({"active": true, "client_id": "tool-1", "aud": [AUDIENCE, "other"], "iss": provider.issuer, "exp": now + 300, "scope": "pacs:read"});
    let cases = [
        ("opaque-good", good.clone(), StatusCode::OK),
        (
            "opaque-inactive",
            json!({"active": false, "client_id": "tool-1", "aud": AUDIENCE}),
            StatusCode::UNAUTHORIZED,
        ),
        (
            "opaque-active-string",
            {
                let mut g = good.clone();
                g["active"] = json!("true");
                g
            },
            StatusCode::UNAUTHORIZED,
        ),
        (
            "opaque-no-audience",
            {
                let mut g = good.clone();
                g.as_object_mut().unwrap().remove("aud");
                g
            },
            StatusCode::UNAUTHORIZED,
        ),
        (
            "opaque-other-issuer",
            {
                let mut g = good.clone();
                g["iss"] = json!("http://127.0.0.1:1");
                g
            },
            StatusCode::UNAUTHORIZED,
        ),
        (
            "opaque-expired",
            {
                let mut g = good.clone();
                g["exp"] = json!(now - 3600);
                g
            },
            StatusCode::UNAUTHORIZED,
        ),
    ];
    for (token, answer, _) in &cases {
        provider
            .shared
            .lock()
            .unwrap()
            .introspected
            .insert((*token).into(), answer.clone());
    }
    for (token, _, expected) in cases {
        assert_eq!(
            status_with(&app, Method::GET, "/studies", token).await,
            expected,
            "{token}"
        );
    }
    // a token the provider does not know is inactive
    assert_eq!(
        status_with(&app, Method::GET, "/studies", "never-issued").await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_tool_is_registered_by_its_client_id_at_the_provider_and_only_with_one() {
    let provider = Arc::new(Provider::start(None).await);
    let dir = tempfile::tempdir().unwrap();
    let (app, admin) = with_a_tool(&provider, dir.path(), "reader").await;
    let bearer = [("authorization", format!("Bearer {admin}"))];
    for body in [
        json!({"name": "No id", "role": "reader"}),
        json!({"client_id": "", "name": "Empty", "role": "reader"}),
        json!({"client_id": "a b", "name": "Space", "role": "reader"}),
    ] {
        let made = send(&app, Method::POST, "/clients", &bearer, Some(body.clone())).await;
        assert_eq!(made.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }
    let again = send(
        &app,
        Method::POST,
        "/clients",
        &bearer,
        Some(json!({"client_id": "tool-1", "name": "Twice", "role": "reader"})),
    )
    .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
}
