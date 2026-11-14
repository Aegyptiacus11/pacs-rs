//! The login, when `PACS_AUTH` is on: who may sign in, what each role may do, and the guard
//! that stands before every route.
//!
//! Sessions are the `jwt-sessions` crate's: `POST /auth/login` answers with an access token
//! (a JWT, 15 minutes, sent as `Authorization: Bearer`) and sets the refresh token in an
//! httpOnly cookie that only `/auth/*` receives; `POST /auth/refresh` rotates it. No request
//! is authorised by a cookie alone, so no route needs protection against forged cross-site
//! requests.
//!
//! A user has one role, and a role gives permissions:
//!
//! | Permission | Allows |
//! |---|---|
//! | `read` | studies, images, metadata, archives, searches of this archive |
//! | `write` | uploads, measurements, hanging protocols |
//! | `remote` | everything about remote AEs (`/modalities`, `/jobs`), which reaches the hospital's PACS |
//! | `delete` | deleting patients, studies, series and instances |
//! | `users` | managing users and roles (`/users`, `/roles`) |
//!
//! Roles are in the index and can be changed (`/roles`). A server starts with four: `reader`
//! (read), `editor` (read, write, remote), `manager` (also delete) and `admin` (everything).
//! `admin` cannot be changed or removed, so there is always a role that can manage the rest.
//!
//! A browser's own downloads (a link to an archive, a file) cannot carry the header. For
//! those, `POST /auth/download-token` with `{"path": "/studies/<id>/archive"}` gives a token
//! good for one minute and for `GET` on that path alone, sent as `?token=`. It is the only
//! token a URL may carry: an access token in a query is refused, and so is a download token
//! in the header.
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

/// One thing a role may allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Read,
    Write,
    Remote,
    Delete,
    Users,
}

impl Permission {
    pub const ALL: [Permission; 5] = [
        Permission::Read,
        Permission::Write,
        Permission::Remote,
        Permission::Delete,
        Permission::Users,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Permission::Read => "read",
            Permission::Write => "write",
            Permission::Remote => "remote",
            Permission::Delete => "delete",
            Permission::Users => "users",
        }
    }
}

impl FromStr for Permission {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Permission::ALL
            .into_iter()
            .find(|permission| permission.as_str() == text)
            .ok_or_else(|| {
                format!("{text:?} is not a permission: read, write, remote, delete or users")
            })
    }
}

/// The role that always has every permission and cannot be changed or removed.
pub const ADMIN_ROLE: &str = "admin";

/// What a request needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Right {
    /// Any signed-in user: their own account.
    SignedIn,
    Read,
    Write,
    /// Remote AEs: a search there lists the hospital's patients, identified.
    Remote,
    Delete,
    /// Users and roles.
    Admin,
}

impl Right {
    /// The permission this right asks of a role; none for a user's own account.
    pub fn permission(self) -> Option<Permission> {
        match self {
            Right::SignedIn => None,
            Right::Read => Some(Permission::Read),
            Right::Write => Some(Permission::Write),
            Right::Remote => Some(Permission::Remote),
            Right::Delete => Some(Permission::Delete),
            Right::Admin => Some(Permission::Users),
        }
    }
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
    // every signed-in user may see the roles (what their own allows); changing them is for
    // who manages users
    if under("/roles") {
        return if matches!(*method, Method::GET | Method::HEAD) {
            Right::SignedIn
        } else {
            Right::Admin
        };
    }
    if under("/auth") {
        return Right::SignedIn;
    }
    // Remote AEs: a search there lists the hospital's patients, identified, and a move brings
    // their studies in. Not for someone who may only read this archive.
    if under("/modalities") || under("/jobs") {
        return Right::Remote;
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

/// The index's users, as the session library reads them. The login, the role and what the
/// role allows travel in the access token: a change to a role reaches a user at their next
/// refresh, within the token's 15 minutes.
#[derive(Debug, Clone)]
pub struct IndexUsers(Index);

impl IndexUsers {
    async fn auth_user(&self, user: StoredUser) -> Result<AuthUser, StoreError> {
        // a role that is gone allows nothing
        let role = self.0.role(&user.role).await.map_err(StoreError::new)?;
        let permissions = role.map(|role| role.permissions).unwrap_or_default();
        let mut claims = serde_json::Map::new();
        claims.insert("role".into(), user.role.into());
        claims.insert("login".into(), user.login.into());
        claims.insert("perms".into(), permissions.into());
        Ok(AuthUser {
            id: user.id,
            password_hash: user.password_hash,
            active: user.active,
            claims,
        })
    }
}

impl UserStore for IndexUsers {
    async fn by_login(&self, login: &str) -> Result<Option<AuthUser>, StoreError> {
        let user = self
            .0
            .user_by_login(&login.to_lowercase())
            .await
            .map_err(StoreError::new)?;
        match user {
            Some(user) => Ok(Some(self.auth_user(user).await?)),
            None => Ok(None),
        }
    }

    async fn by_id(&self, user_id: &str) -> Result<Option<AuthUser>, StoreError> {
        match self.0.user(user_id).await.map_err(StoreError::new)? {
            Some(user) => Ok(Some(self.auth_user(user).await?)),
            None => Ok(None),
        }
    }
}

type Sessions = Auth<IndexUsers, SqlRefreshStore>;

/// The login of a running server.
#[derive(Clone)]
pub struct Login {
    sessions: Arc<Sessions>,
    /// Signs download tokens: the sessions' secret, one minute.
    downloads: AccessTokens,
    index: Index,
    cookie: CookieSettings,
}

/// How long a download token is good for: time for the browser to start the download.
const DOWNLOAD_TOKEN_SECONDS: u64 = 60;
/// The claim of a download token: the path, query included, it is good for.
const DOWNLOAD_CLAIM: &str = "dl";

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
            .create_user(&new_id()?, &login, &hashed(password).await?, ADMIN_ROLE)
            .await
            .context("making the first administrator")?;
        tracing::info!(%login, "first administrator made; PACS_ADMIN_PASSWORD is no longer read");
    }
    let downloads = access
        .clone()
        .with_ttl(std::time::Duration::from_secs(DOWNLOAD_TOKEN_SECONDS))
        .context("the download tokens")?;
    let sessions = Arc::new(Auth::new(IndexUsers(index.clone()), access, refresh));
    Ok(Some(Login {
        sessions,
        downloads,
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
    /// The id of their role.
    pub role: String,
    pub permissions: Vec<Permission>,
}

impl SignedIn {
    pub fn may(&self, right: Right) -> bool {
        right
            .permission()
            .is_none_or(|needed| self.permissions.contains(&needed))
    }
}

fn signed_in(claims: &Claims) -> Option<SignedIn> {
    Some(SignedIn {
        id: claims.sub().to_owned(),
        login: claims.get("login")?.as_str()?.to_owned(),
        role: claims.get("role")?.as_str()?.to_owned(),
        // a permission this version does not know is left out, not trusted
        permissions: claims
            .get("perms")?
            .as_array()?
            .iter()
            .filter_map(|p| p.as_str()?.parse().ok())
            .collect(),
    })
}

/// Before every route: let open requests through, refuse the others without a valid access
/// token (401) or with a role that does not allow them (403).
async fn guard(State(login): State<Login>, mut request: Request, next: Next) -> Response {
    let (method, path) = (request.method().clone(), request.uri().path().to_owned());
    if is_open(&method, &path) {
        return next.run(request).await;
    }
    let invalid =
        || refused(StatusCode::UNAUTHORIZED, "Invalid or expired access token").into_response();
    let bearer = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim().to_owned());
    let (query_token, without_token) = split_token(request.uri().query());
    let claims = match (bearer, query_token) {
        (Some(token), _) => {
            let Ok(claims) = login.sessions.access.verify(&token) else {
                return invalid();
            };
            // a download token is for a URL, and for nothing else
            if claims.get(DOWNLOAD_CLAIM).is_some() {
                return invalid();
            }
            claims
        }
        (None, Some(token)) => {
            let Ok(claims) = login.sessions.access.verify(&token) else {
                return invalid();
            };
            // Only a download token may travel in a URL, only for GET, only for its own path.
            let asked = match &without_token {
                Some(query) => format!("{path}?{query}"),
                None => path.clone(),
            };
            let for_this = claims.get(DOWNLOAD_CLAIM).and_then(Value::as_str) == Some(&asked);
            if method != Method::GET || !for_this {
                return invalid();
            }
            // the routes never see the token
            let stripped = asked.parse().ok().and_then(|path_and_query| {
                let mut parts = request.uri().clone().into_parts();
                parts.path_and_query = Some(path_and_query);
                axum::http::Uri::from_parts(parts).ok()
            });
            let Some(uri) = stripped else {
                return invalid();
            };
            *request.uri_mut() = uri;
            claims
        }
        (None, None) => {
            return refused(StatusCode::UNAUTHORIZED, "Not authenticated").into_response();
        }
    };
    // A token of ours without a role we know: made by another version, refuse it.
    let Some(user) = signed_in(&claims) else {
        return invalid();
    };
    if !user.may(right_for(&method, &path)) {
        tracing::info!(login = %user.login, role = %user.role, %method, %path, "refused: role");
        return refused(StatusCode::FORBIDDEN, "Your role does not allow this").into_response();
    }
    request.extensions_mut().insert(user);
    next.run(request).await
}

/// The `token` parameter of a query, and the query without it (`None` when nothing is left).
fn split_token(query: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(query) = query else {
        return (None, None);
    };
    let (mut token, mut rest) = (None, Vec::new());
    for pair in query.split('&') {
        match pair.strip_prefix("token=") {
            Some(value) if token.is_none() => token = Some(value.to_owned()),
            _ => rest.push(pair),
        }
    }
    let rest = (!rest.is_empty()).then(|| rest.join("&"));
    (token, rest)
}

/// `uri` for the log: a download token in it is a credential for a minute, so it is not
/// written out.
pub fn for_log(uri: &axum::http::Uri) -> String {
    let (token, rest) = split_token(uri.query());
    match (token, rest) {
        (None, _) => uri.to_string(),
        (Some(_), None) => format!("{}?token=...", uri.path()),
        (Some(_), Some(rest)) => format!("{}?{rest}&token=...", uri.path()),
    }
}

#[derive(Deserialize)]
struct DownloadRequest {
    path: String,
}

/// `POST /auth/download-token`: a token for one `GET` of `path` (with its query, if any), for
/// a link the browser follows itself. Refused for a path the user may not read.
async fn download_token(
    State(login): State<Login>,
    Extension(user): Extension<SignedIn>,
    asked: Result<Json<DownloadRequest>, JsonRejection>,
) -> Result<Json<Value>, Refusal> {
    let asked = body(asked)?.path;
    let path = asked.split('?').next().unwrap_or_default();
    let usable = asked.starts_with('/')
        && asked.len() <= 4096
        && !asked.contains(['#', ' ', '\r', '\n'])
        && split_token(asked.split_once('?').map(|(_, query)| query))
            .0
            .is_none()
        && asked.parse::<axum::http::uri::PathAndQuery>().is_ok();
    if !usable {
        return Err(unprocessable(
            "path: a path of this server, starting with /".into(),
        ));
    }
    // nothing of the accounts by URL, and nothing that needs no token anyway
    let right = right_for(&Method::GET, path);
    if matches!(right, Right::SignedIn | Right::Admin) || is_open(&Method::GET, path) {
        return Err(refused(
            StatusCode::FORBIDDEN,
            "No download token for this path",
        ));
    }
    if !user.may(right) {
        return Err(refused(
            StatusCode::FORBIDDEN,
            "Your role does not allow this",
        ));
    }
    let mut claims = serde_json::Map::new();
    let permissions: Vec<&str> = user.permissions.iter().map(|p| p.as_str()).collect();
    claims.insert("role".into(), user.role.into());
    claims.insert("perms".into(), permissions.into());
    claims.insert("login".into(), user.login.into());
    claims.insert(DOWNLOAD_CLAIM.into(), asked.into());
    let token = login
        .downloads
        .issue(&user.id, &claims)
        .map_err(|e| internal("making a download token", e))?;
    Ok(Json(
        json!({ "token": token, "expires_in": DOWNLOAD_TOKEN_SECONDS }),
    ))
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
    let permissions: Vec<&str> = user.permissions.iter().map(|p| p.as_str()).collect();
    Json(
        json!({ "id": user.id, "login": user.login, "role": user.role, "permissions": permissions }),
    )
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
        .update_user(&user.id, &change, ADMIN_ROLE)
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
    let role = known_role(&login, &new.role).await?;
    check_new_password(&new.password).map_err(unprocessable)?;
    let id = new_id().map_err(|e| internal("making a user id", e))?;
    let hash = hashed(new.password)
        .await
        .map_err(|e| internal("hashing a password", e))?;
    let user = login
        .index
        .create_user(&id, &name, &hash, &role)
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
    let role = match update.role.as_deref() {
        Some(role) => Some(known_role(&login, role).await?),
        None => None,
    };
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
        role,
        active: update.active,
    };
    let user = login
        .index
        .update_user(&id, &change, ADMIN_ROLE)
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
        .delete_user(&id, ADMIN_ROLE)
        .await
        .map_err(|e| from_index("removing a user", e))?;
    if !deleted {
        return Err(refused(StatusCode::NOT_FOUND, "No such user"));
    }
    end_sessions(&login, &id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The id of an existing role, or the 422 that says there is none.
async fn known_role(login: &Login, role: &str) -> Result<String, Refusal> {
    match login.index.role(role).await {
        Ok(Some(found)) => Ok(found.id),
        Ok(None) => Err(unprocessable(format!(
            "{role:?} is not a role of this server"
        ))),
        Err(e) => Err(from_index("reading a role", e)),
    }
}

fn shown_role(role: &pacs_index::StoredRole) -> Value {
    json!({
        "id": role.id,
        "name": role.name,
        "permissions": role.permissions,
        // the one role that cannot be changed or removed
        "locked": role.id == ADMIN_ROLE,
    })
}

async fn list_roles(State(login): State<Login>) -> Result<Json<Value>, Refusal> {
    let roles = login
        .index
        .roles()
        .await
        .map_err(|e| from_index("listing roles", e))?;
    Ok(Json(Value::Array(roles.iter().map(shown_role).collect())))
}

/// A role's name as shown: 1 to 40 characters, none of them a control character.
fn role_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
        return Err("a role's name is 1 to 40 characters".into());
    }
    Ok(name.to_owned())
}

/// The id made from a role's name: its letters and digits in lower case, the rest as `-`.
fn role_id(name: &str) -> Result<String, String> {
    let id: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let id = id
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if id.is_empty() {
        return Err("a role's name needs at least one letter or digit (a to z, 0 to 9)".into());
    }
    Ok(id)
}

/// Permissions as stored: each one known, none twice, in the order of [`Permission::ALL`].
fn permission_list(asked: &[String]) -> Result<Vec<String>, String> {
    let asked = asked
        .iter()
        .map(|p| p.parse::<Permission>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Permission::ALL
        .into_iter()
        .filter(|p| asked.contains(p))
        .map(|p| p.as_str().to_owned())
        .collect())
}

#[derive(Deserialize)]
struct NewRole {
    name: String,
    #[serde(default)]
    permissions: Vec<String>,
}

async fn create_role(
    State(login): State<Login>,
    new: Result<Json<NewRole>, JsonRejection>,
) -> Result<Response, Refusal> {
    let new = body(new)?;
    let name = role_name(&new.name).map_err(unprocessable)?;
    let id = role_id(&name).map_err(unprocessable)?;
    let permissions = permission_list(&new.permissions).map_err(unprocessable)?;
    let role = login
        .index
        .create_role(&id, &name, &permissions)
        .await
        .map_err(|e| from_index("making a role", e))?;
    Ok((StatusCode::CREATED, Json(shown_role(&role))).into_response())
}

#[derive(Deserialize)]
struct RoleUpdate {
    name: Option<String>,
    permissions: Option<Vec<String>>,
}

/// `PUT /roles/{id}`: a new name, new permissions, or both. The users who have the role get
/// the change at their next refresh, within 15 minutes.
async fn update_role(
    State(login): State<Login>,
    Path(id): Path<String>,
    update: Result<Json<RoleUpdate>, JsonRejection>,
) -> Result<Response, Refusal> {
    let update = body(update)?;
    if id == ADMIN_ROLE {
        return Err(refused(
            StatusCode::CONFLICT,
            "The admin role always has every permission and cannot be changed",
        ));
    }
    let name = update
        .name
        .as_deref()
        .map(role_name)
        .transpose()
        .map_err(unprocessable)?;
    let permissions = update
        .permissions
        .as_deref()
        .map(permission_list)
        .transpose()
        .map_err(unprocessable)?;
    let role = login
        .index
        .update_role(&id, name.as_deref(), permissions.as_deref())
        .await
        .map_err(|e| from_index("changing a role", e))?
        .ok_or_else(|| refused(StatusCode::NOT_FOUND, "No such role"))?;
    Ok(Json(shown_role(&role)).into_response())
}

async fn delete_role(
    State(login): State<Login>,
    Path(id): Path<String>,
) -> Result<Response, Refusal> {
    if id == ADMIN_ROLE {
        return Err(refused(
            StatusCode::CONFLICT,
            "The admin role cannot be removed",
        ));
    }
    let deleted = login
        .index
        .delete_role(&id)
        .await
        .map_err(|e| from_index("removing a role", e))?;
    if !deleted {
        return Err(refused(StatusCode::NOT_FOUND, "No such role"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

impl Login {
    /// The sign-in routes (`/auth/login`, `/auth/refresh`, `/auth/logout`,
    /// `/auth/logout-all`), `GET /auth/me`, `POST /auth/password`, `/users` and `/roles`.
    pub fn router(&self) -> anyhow::Result<Router> {
        let sessions: Router = auth_router(self.sessions.clone(), self.cookie.clone())
            .context("the refresh cookie")?;
        let own = Router::new()
            .route("/auth/me", get(me))
            .route("/auth/password", post(change_password))
            .route("/auth/download-token", post(download_token))
            .route("/users", get(list_users).post(create_user))
            .route("/users/{id}", put(update_user).delete(delete_user))
            .route("/roles", get(list_roles).post(create_role))
            .route("/roles/{id}", put(update_role).delete(delete_role))
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
    fn a_right_asks_for_a_permission_and_a_user_has_it_or_not() {
        let user = |permissions: &[Permission]| super::SignedIn {
            id: "u1".into(),
            login: "ada".into(),
            role: "custom".into(),
            permissions: permissions.to_vec(),
        };
        let rights = [
            Right::SignedIn,
            Right::Read,
            Right::Write,
            Right::Remote,
            Right::Delete,
            Right::Admin,
        ];
        let allowed = |permissions: &[Permission]| rights.map(|r| user(permissions).may(r));
        // their own account is every signed-in user's, whatever the role allows
        assert_eq!(allowed(&[]), [true, false, false, false, false, false]);
        assert_eq!(
            allowed(&[Permission::Read]),
            [true, true, false, false, false, false]
        );
        // permissions do not include one another: this role deletes but cannot upload
        assert_eq!(
            allowed(&[Permission::Read, Permission::Delete]),
            [true, true, false, false, true, false]
        );
        assert_eq!(allowed(&Permission::ALL), [true; 6]);
        for permission in Permission::ALL {
            assert_eq!(permission.as_str().parse(), Ok(permission));
        }
        assert!("Read".parse::<Permission>().is_err());
        assert!("admin".parse::<Permission>().is_err());
        assert!("".parse::<Permission>().is_err());
    }

    #[test]
    fn role_names_ids_and_permission_lists() {
        assert_eq!(role_name("  Night shift "), Ok("Night shift".into()));
        for bad in ["", "   ", "a\u{0}b", &"x".repeat(41)] {
            assert!(role_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(role_id("Night shift"), Ok("night-shift".into()));
        assert_eq!(role_id("  R&D / PhD 2 "), Ok("r-d-phd-2".into()));
        assert!(role_id("---").is_err());
        assert!(role_id("\u{e9}t\u{e9}").is_ok());
        assert!(role_id("\u{4e2d}\u{6587}").is_err());

        let list = |asked: &[&str]| {
            permission_list(&asked.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>())
        };
        // stored once each, in one order, whatever was sent
        assert_eq!(
            list(&["delete", "read", "read"]),
            Ok(vec!["read".into(), "delete".into()])
        );
        assert_eq!(list(&[]), Ok(vec![]));
        assert!(list(&["read", "everything"]).is_err());
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
            (Method::GET, "/modalities", Remote),
            (Method::GET, "/modalities/hospital/studies", Remote),
            (Method::POST, "/modalities/hospital/move", Remote),
            (Method::GET, "/jobs/1", Remote),
            (Method::GET, "/roles", SignedIn),
            (Method::POST, "/roles", Admin),
            (Method::PUT, "/roles/editor", Admin),
            (Method::DELETE, "/roles/editor", Admin),
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
    fn the_token_of_a_query_and_the_log() {
        let split = |q| split_token(Some(q));
        assert_eq!(split_token(None), (None, None));
        assert_eq!(split("a=1"), (None, Some("a=1".into())));
        assert_eq!(split("token=T"), (Some("T".into()), None));
        assert_eq!(
            split("a=1&token=T&b=2"),
            (Some("T".into()), Some("a=1&b=2".into()))
        );
        // only a parameter named token, and only the first
        assert_eq!(split("mytoken=T"), (None, Some("mytoken=T".into())));
        assert_eq!(
            split("token=T&token=U"),
            (Some("T".into()), Some("token=U".into()))
        );

        let log = |uri: &str| for_log(&uri.parse().unwrap());
        assert_eq!(log("/studies?limit=2"), "/studies?limit=2");
        assert_eq!(
            log("/studies/a/archive?token=SECRET"),
            "/studies/a/archive?token=..."
        );
        assert_eq!(
            log("/tools/create-archive?resources=a,b&token=SECRET"),
            "/tools/create-archive?resources=a,b&token=..."
        );
        assert!(!log("/x?token=SECRET&y=1").contains("SECRET"));
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
