//! The login, when `PACS_AUTH` is on: who may sign in, what each role may do, and the guard
//! that stands before every route.
//!
//! Sessions are the `jwt-sessions` crate's: `POST /auth/login` answers with an access token
//! (a JWT, 15 minutes, sent as `Authorization: Bearer`) and sets the refresh token in an
//! httpOnly cookie that only `/auth/*` receives; `POST /auth/refresh` rotates it. No request
//! is authorised by a cookie alone, so no route needs protection against forged cross-site
//! requests.
//!
//! | Role | May |
//! |---|---|
//! | `reader` | read: studies, images, metadata, archives, searches of this archive |
//! | `editor` | also write: uploads, measurements, hanging protocols, and everything about remote AEs (`/modalities`, `/jobs`), which reaches the hospital's PACS |
//! | `manager` | also delete |
//! | `admin` | also manage users (`/users`) |
//!
//! Open without a token: `/healthz`, the web interface's own files (not `/ui/api/`), the
//! sign-in routes, and `OPTIONS`. Everything else, a route that does not exist included, is
//! refused without one.
//!
//! DIMSE has no login here: `PACS_DIMSE_KNOWN_CALLERS_ONLY` and the network decide who may
//! open an association.

use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Context;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Request, State};
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use jwt_sessions::http::{CookieSettings, auth_router};
use jwt_sessions::{
    AccessTokens, Auth, AuthUser, Claims, RefreshTokens, SqlRefreshStore, StoreError, UserStore,
    check_password, hash_password,
};
use pacs_index::{Index, StoredUser, UserChange};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Settings;

/// What a user may do; each role includes the ones before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Reader,
    Editor,
    Manager,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Reader => "reader",
            Role::Editor => "editor",
            Role::Manager => "manager",
            Role::Admin => "admin",
        }
    }

    pub fn allows(self, right: Right) -> bool {
        match right {
            Right::SignedIn | Right::Read => true,
            Right::Write => self >= Role::Editor,
            Right::Delete => self >= Role::Manager,
            Right::Admin => self == Role::Admin,
        }
    }
}

impl FromStr for Role {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        [Role::Reader, Role::Editor, Role::Manager, Role::Admin]
            .into_iter()
            .find(|role| role.as_str() == text)
            .ok_or_else(|| format!("{text:?} is not a role: reader, editor, manager or admin"))
    }
}

/// What a request needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Right {
    /// Any signed-in user: their own account.
    SignedIn,
    Read,
    Write,
    Delete,
    Admin,
}

/// Whether a request passes without a token.
pub fn is_open(method: &Method, path: &str) -> bool {
    method == Method::OPTIONS
        || matches!(
            path,
            "/healthz" | "/" | "/ui" | "/auth/login" | "/auth/refresh" | "/auth/logout"
        )
        // the web interface's files, which hold nothing of the archive; its API is not open
        || (method == Method::GET && path.starts_with("/ui/") && !path.starts_with("/ui/api/"))
}

/// The right a request to `path` needs, when it is not open. A method this server does not
/// know needs the highest.
pub fn right_for(method: &Method, path: &str) -> Right {
    let under = |prefix: &str| {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    };
    if under("/users") {
        return Right::Admin;
    }
    if under("/auth") {
        return Right::SignedIn;
    }
    // Remote AEs: a search there lists the hospital's patients, identified, and a move brings
    // their studies in. Not for someone who may only read this archive.
    if under("/modalities") || under("/jobs") {
        return Right::Write;
    }
    match *method {
        Method::GET | Method::HEAD => Right::Read,
        // searches and archives of what the user may already read
        Method::POST if matches!(path, "/tools/find" | "/tools/create-archive") => Right::Read,
        Method::POST | Method::PUT | Method::PATCH => Right::Write,
        // the web interface's own documents are written and removed with the same right
        Method::DELETE if under("/ui/api") => Right::Write,
        Method::DELETE => Right::Delete,
        _ => Right::Admin,
    }
}

/// The index's users, as the session library reads them. The role and the login travel in the
/// access token.
#[derive(Debug, Clone)]
pub struct IndexUsers(Index);

fn auth_user(user: StoredUser) -> AuthUser {
    let mut claims = serde_json::Map::new();
    claims.insert("role".into(), user.role.into());
    claims.insert("login".into(), user.login.into());
    AuthUser {
        id: user.id,
        password_hash: user.password_hash,
        active: user.active,
        claims,
    }
}

impl UserStore for IndexUsers {
    async fn by_login(&self, login: &str) -> Result<Option<AuthUser>, StoreError> {
        let user = self
            .0
            .user_by_login(&login.to_lowercase())
            .await
            .map_err(StoreError::new)?;
        Ok(user.map(auth_user))
    }

    async fn by_id(&self, user_id: &str) -> Result<Option<AuthUser>, StoreError> {
        Ok(self
            .0
            .user(user_id)
            .await
            .map_err(StoreError::new)?
            .map(auth_user))
    }
}

type Sessions = Auth<IndexUsers, SqlRefreshStore>;

/// The login of a running server.
#[derive(Clone)]
pub struct Login {
    sessions: Arc<Sessions>,
    index: Index,
    cookie: CookieSettings,
}

/// Passwords shorter than this are refused when a user is made or changed.
pub const MIN_PASSWORD_BYTES: usize = 12;
/// argon2 hashes whatever it is given; this bounds the work one request can ask for.
const MAX_PASSWORD_BYTES: usize = 1024;

fn check_new_password(password: &str) -> Result<(), String> {
    if password.len() < MIN_PASSWORD_BYTES || password.len() > MAX_PASSWORD_BYTES {
        return Err(format!(
            "a password is {MIN_PASSWORD_BYTES} to {MAX_PASSWORD_BYTES} bytes"
        ));
    }
    Ok(())
}

/// A login as stored: lower case, 1 to 320 bytes, no spaces or control characters.
fn normal_login(login: &str) -> Result<String, String> {
    let login = login.trim().to_lowercase();
    let usable = !login.is_empty()
        && login.len() <= 320
        && !login.chars().any(|c| c.is_whitespace() || c.is_control());
    if usable {
        Ok(login)
    } else {
        Err("a login is 1 to 320 bytes, without spaces".into())
    }
}

fn new_id() -> anyhow::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no random bytes: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// argon2 takes tens of milliseconds of CPU: off the async threads.
async fn hashed(password: String) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .context("hashing a password")
}

/// Open the login the settings ask for; `None` when `PACS_AUTH` is off.
///
/// The signing secret comes from `PACS_AUTH_SECRET` (or `..._FILE`). With no user in the
/// index yet, the first administrator is made from `PACS_ADMIN_LOGIN` and
/// `PACS_ADMIN_PASSWORD` (or `..._FILE`); once there is a user, those are not read.
pub async fn open_login(
    settings: &Settings,
    index: &Index,
    env: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Option<Login>> {
    if !settings.auth {
        return Ok(None);
    }
    let secret = crate::secret("PACS_AUTH_SECRET", &env)?.context(
        "PACS_AUTH is on: PACS_AUTH_SECRET (or PACS_AUTH_SECRET_FILE) must hold the signing \
         secret, at least 32 bytes (openssl rand -hex 32)",
    )?;
    let access = AccessTokens::new(&secret).context("PACS_AUTH_SECRET")?;
    let cookie = CookieSettings {
        path: settings.auth_cookie_path.clone(),
        secure: settings.auth_cookie_secure,
        ..CookieSettings::default()
    };
    let store = SqlRefreshStore::new(index.pool().clone(), "auth_")
        .map_err(|e| anyhow::anyhow!("the refresh token store: {e}"))?;
    let refresh = RefreshTokens::new(store);
    let purged = refresh
        .purge(SystemTime::now())
        .await
        .map_err(|e| anyhow::anyhow!("dropping expired refresh tokens: {e}"))?;
    if purged > 0 {
        tracing::info!(purged, "expired refresh tokens dropped");
    }

    if index.user_count().await? == 0 {
        let password = crate::secret("PACS_ADMIN_PASSWORD", &env)?.context(
            "PACS_AUTH is on and there is no user yet: PACS_ADMIN_PASSWORD (or \
             PACS_ADMIN_PASSWORD_FILE) must hold the first administrator's password",
        )?;
        check_new_password(&password).map_err(|e| anyhow::anyhow!("PACS_ADMIN_PASSWORD: {e}"))?;
        let login = normal_login(&settings.admin_login)
            .map_err(|e| anyhow::anyhow!("PACS_ADMIN_LOGIN: {e}"))?;
        index
            .create_user(
                &new_id()?,
                &login,
                &hashed(password).await?,
                Role::Admin.as_str(),
            )
            .await
            .context("making the first administrator")?;
        tracing::info!(%login, "first administrator made; PACS_ADMIN_PASSWORD is no longer read");
    }
    let sessions = Arc::new(Auth::new(IndexUsers(index.clone()), access, refresh));
    Ok(Some(Login {
        sessions,
        index: index.clone(),
        cookie,
    }))
}

/// A request refused: `{"detail": ...}`, with the Bearer challenge on a 401.
#[derive(Debug)]
struct Refusal {
    status: StatusCode,
    detail: String,
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let body = Json(json!({ "detail": self.detail }));
        let mut response = (self.status, body).into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

fn refused(status: StatusCode, detail: &str) -> Refusal {
    Refusal {
        status,
        detail: detail.to_owned(),
    }
}

/// Our failure, not the caller's: logged in full, told as nothing more than a 500.
fn internal(what: &str, error: impl std::fmt::Display) -> Refusal {
    tracing::error!("{what}: {error}");
    refused(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
}

/// The signed-in user of a request, put there by the guard.
#[derive(Debug, Clone)]
pub struct SignedIn {
    pub id: String,
    pub login: String,
    pub role: Role,
}

fn signed_in(claims: &Claims) -> Option<SignedIn> {
    Some(SignedIn {
        id: claims.sub().to_owned(),
        login: claims.get("login")?.as_str()?.to_owned(),
        role: claims.get("role")?.as_str()?.parse().ok()?,
    })
}

/// Before every route: let open requests through, refuse the others without a valid access
/// token (401) or with a role that does not allow them (403).
async fn guard(State(login): State<Login>, mut request: Request, next: Next) -> Response {
    let (method, path) = (request.method().clone(), request.uri().path().to_owned());
    if is_open(&method, &path) {
        return next.run(request).await;
    }
    let token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim());
    let Some(token) = token else {
        return refused(StatusCode::UNAUTHORIZED, "Not authenticated").into_response();
    };
    let Ok(claims) = login.sessions.access.verify(token) else {
        return refused(StatusCode::UNAUTHORIZED, "Invalid or expired access token")
            .into_response();
    };
    // A token of ours without a role we know: made by another version, refuse it.
    let Some(user) = signed_in(&claims) else {
        return refused(StatusCode::UNAUTHORIZED, "Invalid or expired access token")
            .into_response();
    };
    if !user.role.allows(right_for(&method, &path)) {
        tracing::info!(login = %user.login, role = user.role.as_str(), %method, %path, "refused: role");
        return refused(StatusCode::FORBIDDEN, "Your role does not allow this").into_response();
    }
    request.extensions_mut().insert(user);
    next.run(request).await
}

fn shown(user: &StoredUser) -> Value {
    json!({
        "id": user.id,
        "login": user.login,
        "role": user.role,
        "active": user.active,
        "created_ms": user.created_ms,
    })
}

/// A JSON body, or the 4xx that says why it is not one.
fn body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Refusal> {
    body.map(|Json(value)| value)
        .map_err(|e| refused(e.status(), &e.body_text()))
}

fn unprocessable(detail: String) -> Refusal {
    refused(StatusCode::UNPROCESSABLE_ENTITY, &detail)
}

fn from_index(what: &str, error: pacs_index::Error) -> Refusal {
    match error {
        pacs_index::Error::Conflict(message) => refused(StatusCode::CONFLICT, &message),
        other => internal(what, other),
    }
}

async fn me(Extension(user): Extension<SignedIn>) -> Json<Value> {
    Json(json!({ "id": user.id, "login": user.login, "role": user.role.as_str() }))
}

#[derive(Deserialize)]
struct PasswordChange {
    current_password: String,
    new_password: String,
}

/// `POST /auth/password`: a user changes their own password, and every session of theirs
/// ends, this one included.
async fn change_password(
    State(login): State<Login>,
    Extension(user): Extension<SignedIn>,
    change: Result<Json<PasswordChange>, JsonRejection>,
) -> Result<Response, Refusal> {
    let change = body(change)?;
    check_new_password(&change.new_password).map_err(unprocessable)?;
    if change.current_password.len() > MAX_PASSWORD_BYTES {
        return Err(refused(
            StatusCode::FORBIDDEN,
            "The current password is wrong",
        ));
    }
    let stored = login
        .index
        .user(&user.id)
        .await
        .map_err(|e| from_index("reading a user", e))?;
    let hash = stored.map(|u| u.password_hash);
    let current = change.current_password;
    let matches = tokio::task::spawn_blocking(move || check_password(&current, hash.as_deref()))
        .await
        .map_err(|e| internal("checking a password", e))?
        .map_err(|e| internal("checking a password", e))?;
    if !matches {
        return Err(refused(
            StatusCode::FORBIDDEN,
            "The current password is wrong",
        ));
    }
    let new_hash = hashed(change.new_password)
        .await
        .map_err(|e| internal("hashing a password", e))?;
    let change = UserChange {
        password_hash: Some(new_hash),
        ..UserChange::default()
    };
    login
        .index
        .update_user(&user.id, &change, Role::Admin.as_str())
        .await
        .map_err(|e| from_index("changing a password", e))?;
    end_sessions(&login, &user.id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn end_sessions(login: &Login, user_id: &str) -> Result<(), Refusal> {
    login
        .sessions
        .logout_everywhere(user_id)
        .await
        .map(|_| ())
        .map_err(|e| internal("ending a user's sessions", e))
}

async fn list_users(State(login): State<Login>) -> Result<Json<Value>, Refusal> {
    let users = login
        .index
        .users()
        .await
        .map_err(|e| from_index("listing users", e))?;
    Ok(Json(Value::Array(users.iter().map(shown).collect())))
}

#[derive(Deserialize)]
struct NewUser {
    login: String,
    password: String,
    role: String,
}

async fn create_user(
    State(login): State<Login>,
    new: Result<Json<NewUser>, JsonRejection>,
) -> Result<Response, Refusal> {
    let new = body(new)?;
    let name = normal_login(&new.login).map_err(unprocessable)?;
    let role: Role = new.role.parse().map_err(unprocessable)?;
    check_new_password(&new.password).map_err(unprocessable)?;
    let id = new_id().map_err(|e| internal("making a user id", e))?;
    let hash = hashed(new.password)
        .await
        .map_err(|e| internal("hashing a password", e))?;
    let user = login
        .index
        .create_user(&id, &name, &hash, role.as_str())
        .await
        .map_err(|e| from_index("making a user", e))?;
    Ok((StatusCode::CREATED, Json(shown(&user))).into_response())
}

#[derive(Deserialize)]
struct UserUpdate {
    password: Option<String>,
    role: Option<String>,
    active: Option<bool>,
}

/// `PUT /users/{id}`: a new password, role or state. A new password or a deactivation ends
/// the user's sessions; a new role reaches their access token at its next refresh, within
/// 15 minutes.
async fn update_user(
    State(login): State<Login>,
    Path(id): Path<String>,
    update: Result<Json<UserUpdate>, JsonRejection>,
) -> Result<Response, Refusal> {
    let update = body(update)?;
    let role = update
        .role
        .as_deref()
        .map(Role::from_str)
        .transpose()
        .map_err(unprocessable)?;
    let password_hash = match update.password {
        Some(password) => {
            check_new_password(&password).map_err(unprocessable)?;
            Some(
                hashed(password)
                    .await
                    .map_err(|e| internal("hashing a password", e))?,
            )
        }
        None => None,
    };
    let ends_sessions = password_hash.is_some() || update.active == Some(false);
    let change = UserChange {
        password_hash,
        role: role.map(|r| r.as_str().to_owned()),
        active: update.active,
    };
    let user = login
        .index
        .update_user(&id, &change, Role::Admin.as_str())
        .await
        .map_err(|e| from_index("changing a user", e))?
        .ok_or_else(|| refused(StatusCode::NOT_FOUND, "No such user"))?;
    if ends_sessions {
        end_sessions(&login, &id).await?;
    }
    Ok(Json(shown(&user)).into_response())
}

async fn delete_user(
    State(login): State<Login>,
    Path(id): Path<String>,
) -> Result<Response, Refusal> {
    let deleted = login
        .index
        .delete_user(&id, Role::Admin.as_str())
        .await
        .map_err(|e| from_index("removing a user", e))?;
    if !deleted {
        return Err(refused(StatusCode::NOT_FOUND, "No such user"));
    }
    end_sessions(&login, &id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

impl Login {
    /// The sign-in routes (`/auth/login`, `/auth/refresh`, `/auth/logout`,
    /// `/auth/logout-all`), `GET /auth/me`, `POST /auth/password` and `/users`.
    pub fn router(&self) -> anyhow::Result<Router> {
        let sessions: Router = auth_router(self.sessions.clone(), self.cookie.clone())
            .context("the refresh cookie")?;
        let own = Router::new()
            .route("/auth/me", get(me))
            .route("/auth/password", post(change_password))
            .route("/users", get(list_users).post(create_user))
            .route("/users/{id}", put(update_user).delete(delete_user))
            .with_state(self.clone());
        Ok(sessions.merge(own))
    }

    /// Put the guard before every route of `app`.
    pub fn protect(&self, app: Router) -> Router {
        app.layer(axum::middleware::from_fn_with_state(self.clone(), guard))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_role_includes_the_ones_before() {
        use Right::*;
        let allowed = |role: Role| [SignedIn, Read, Write, Delete, Admin].map(|r| role.allows(r));
        assert_eq!(allowed(Role::Reader), [true, true, false, false, false]);
        assert_eq!(allowed(Role::Editor), [true, true, true, false, false]);
        assert_eq!(allowed(Role::Manager), [true, true, true, true, false]);
        assert_eq!(allowed(Role::Admin), [true, true, true, true, true]);
        for role in [Role::Reader, Role::Editor, Role::Manager, Role::Admin] {
            assert_eq!(role.as_str().parse(), Ok(role));
        }
        assert!("Admin".parse::<Role>().is_err());
        assert!("".parse::<Role>().is_err());
    }

    #[test]
    fn what_is_open_without_a_token() {
        let get = Method::GET;
        for path in [
            "/healthz",
            "/",
            "/ui",
            "/ui/",
            "/ui/assets/app.js",
            "/ui/config.json",
        ] {
            assert!(is_open(&get, path), "{path}");
        }
        for path in ["/auth/login", "/auth/refresh", "/auth/logout"] {
            assert!(is_open(&Method::POST, path), "{path}");
        }
        assert!(is_open(&Method::OPTIONS, "/studies"));
        let closed = [
            "/studies",
            "/dicom-web/studies",
            "/ui/api/studies",
            "/ui/api/protocols",
            "/auth/me",
            "/auth/logout-all",
            "/auth/password",
            "/users",
            "/UI/",
            "/healthz/",
            "//healthz",
            "",
        ];
        for path in closed {
            assert!(!is_open(&get, path), "{path}");
        }
        // Open, and harmless: the router does not resolve `..`, so this names a file of the
        // interface that does not exist (tests/auth.rs asks for it).
        assert!(is_open(&get, "/ui/../studies"));
        // only reading the interface's files is open
        assert!(!is_open(&Method::POST, "/ui/anything"));
        assert!(!is_open(&Method::DELETE, "/ui/"));
    }

    #[test]
    fn the_right_each_request_needs() {
        use Right::*;
        let cases = [
            (Method::GET, "/studies", Read),
            (Method::HEAD, "/studies/abc/archive", Read),
            (Method::GET, "/dicom-web/studies/1.2/series", Read),
            (Method::GET, "/ui/api/studies", Read),
            (Method::POST, "/tools/find", Read),
            (Method::POST, "/tools/create-archive", Read),
            (Method::POST, "/instances", Write),
            (Method::POST, "/dicom-web/studies", Write),
            (Method::PUT, "/ui/api/annotations/1.2", Write),
            (Method::PUT, "/ui/api/protocols/p1", Write),
            (Method::DELETE, "/ui/api/protocols/p1", Write),
            (Method::PATCH, "/studies/abc", Write),
            (Method::GET, "/modalities", Write),
            (Method::GET, "/modalities/hospital/studies", Write),
            (Method::POST, "/modalities/hospital/move", Write),
            (Method::GET, "/jobs/1", Write),
            (Method::DELETE, "/studies/abc", Delete),
            (Method::DELETE, "/patients/abc", Delete),
            (Method::GET, "/users", Admin),
            (Method::POST, "/users", Admin),
            (Method::DELETE, "/users/u1", Admin),
            (Method::GET, "/auth/me", SignedIn),
            (Method::POST, "/auth/logout-all", SignedIn),
            (Method::POST, "/auth/password", SignedIn),
            (Method::TRACE, "/studies", Admin),
            (Method::CONNECT, "/studies", Admin),
            // a name that only starts like a protected one is an ordinary path
            (Method::GET, "/usersx", Read),
            (Method::DELETE, "/modalitiesx", Delete),
        ];
        for (method, path, right) in cases {
            assert_eq!(right_for(&method, path), right, "{method} {path}");
        }
    }

    #[test]
    fn logins_and_passwords() {
        assert_eq!(
            normal_login("  Ada@Example.COM "),
            Ok("ada@example.com".into())
        );
        for bad in ["", "   ", "a b", "a\tb", "a\u{0}b", &"a".repeat(321)] {
            assert!(normal_login(bad).is_err(), "{bad:?}");
        }
        assert!(check_new_password("12345678901").is_err());
        assert!(check_new_password("123456789012").is_ok());
        assert!(check_new_password(&"x".repeat(1024)).is_ok());
        assert!(check_new_password(&"x".repeat(1025)).is_err());
    }
}
