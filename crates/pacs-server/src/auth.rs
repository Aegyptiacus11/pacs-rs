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
//! | `identities` | the studies as stored; without it, only the studies of projects, de-identified |
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
//! A program that connects without a person (a viewer, a pipeline) is a client, made under
//! `/clients` with a role: see [`token`] for how it gets its access tokens and what they
//! allow.
//!
//! Open without a token: `/healthz`, the web interface's own files (not `/ui/api/`), the
//! sign-in routes, `/auth/token`, and `OPTIONS`. Everything else, a route that does not exist included, is
//! refused without one.
//!
//! DIMSE has no login here: `PACS_DIMSE_KNOWN_CALLERS_ONLY` and the network decide who may
//! open an association.

use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Context;
use axum::extract::rejection::{FormRejection, JsonRejection};
use axum::extract::{Form, Path, Request, State};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, PRAGMA, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json, Router};
use base64::Engine;
use jwt_sessions::http::{CookieSettings, auth_router};
use jwt_sessions::{
    AccessTokens, Auth, AuthUser, Claims, RefreshTokens, SqlRefreshStore, StoreError, UserStore,
    check_password, hash_password,
};
use utoipa::ToSchema;
use utoipa::openapi::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth_schema::{
    Credentials, DownloadRequest, DownloadToken, Me, Refused, Role, Session, Token, TokenRefused,
    TokenRequest, Tool, User,
};
use pacs_index::{Index, SeenBy, StoredClient, StoredUser, UserChange};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::Settings;

/// One thing a role may allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Read,
    Write,
    Remote,
    Delete,
    Users,
    /// The studies as stored, with the patient's identity; without it, only the studies of
    /// projects, de-identified.
    Identities,
}

impl Permission {
    pub const ALL: [Permission; 6] = [
        Permission::Read,
        Permission::Write,
        Permission::Remote,
        Permission::Delete,
        Permission::Users,
        Permission::Identities,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Permission::Read => "read",
            Permission::Write => "write",
            Permission::Remote => "remote",
            Permission::Delete => "delete",
            Permission::Users => "users",
            Permission::Identities => "identities",
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
                format!(
                    "{text:?} is not a permission: read, write, remote, delete, users or identities"
                )
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
            "/healthz"
                | "/readyz"
                | "/"
                | "/ui"
                | "/auth/login"
                | "/auth/refresh"
                | "/auth/logout"
                | "/auth/token"
                | "/auth/methods"
                | "/auth/oidc/start"
                | "/auth/oidc/callback"
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
    if under("/ui/api/server") {
        return Right::Admin;
    }
    if under("/trash") {
        return Right::Delete;
    }
    if under("/users") || under("/clients") || under("/projects") || under("/tools/check-files") {
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
    /// Signs the access tokens of clients: the same secret, five minutes, with this server
    /// as issuer and audience, so that neither kind of token passes for the other.
    clients: AccessTokens,
    issuer: String,
    index: Index,
    cookie: CookieSettings,
    /// The names of the projects of `PACS_PROJECTS`: what an account can be limited to.
    projects: Arc<Vec<String>>,
    /// The OpenID provider people sign in through, when `PACS_OIDC_ISSUER` is set: then no
    /// password signs anyone in.
    provider: Option<crate::oidc::Provider>,
    /// Signs the state cookie of a sign-in through the provider: a key of its own, derived
    /// from the sessions' secret, so that it passes for no other token.
    oidc_states: AccessTokens,
}

/// The answer to a password where the provider is the only way in.
const PROVIDER_ONLY: &str =
    "This server signs people in through its OpenID provider, not with a password";

/// How long a download token is good for: time for the browser to start the download.
const DOWNLOAD_TOKEN_SECONDS: u64 = 60;
/// The claim of a download token: the path, query included, it is good for.
const DOWNLOAD_CLAIM: &str = "dl";

/// How long a client's access token is good for. IUA (ITI TF-2: 3.71.5) asks for an hour or
/// less and recommends five minutes or less.
pub const CLIENT_TOKEN_SECONDS: u64 = 300;
/// The claim that marks a client's access token, and names the client.
const CLIENT_CLAIM: &str = "client_id";

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
    let issuer = settings.auth_issuer.trim().to_owned();
    if issuer.is_empty() || issuer.chars().any(|c| c.is_whitespace() || c.is_control()) {
        anyhow::bail!("PACS_AUTH_ISSUER: a name without spaces, e.g. pacs-rs or this server's URL");
    }
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

    let provider = match &settings.oidc_issuer {
        Some(issuer) => Some(open_provider(settings, issuer, &env).await?),
        None => None,
    };
    if let Some(provider) = &provider {
        if index.role(provider.role()).await?.is_none() {
            anyhow::bail!(
                "PACS_OIDC_ROLE: there is no role {:?} (the roles are under /roles)",
                provider.role()
            );
        }
        tracing::info!(issuer = %provider.issuer(), "people sign in through the OpenID provider only");
    } else if index.user_count().await? == 0 {
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
    let clients = access
        .clone()
        .with_ttl(std::time::Duration::from_secs(CLIENT_TOKEN_SECONDS))
        .context("the clients' access tokens")?
        .with_issuer(issuer.clone())
        .with_audience(issuer.clone());
    let oidc_states = AccessTokens::new(&format!("{secret}\noidc-state"))
        .context("the sign-in state")?
        .with_ttl(crate::oidc::STATE_TTL)
        .context("the sign-in state")?;
    let sessions = Arc::new(Auth::new(IndexUsers(index.clone()), access, refresh));
    Ok(Some(Login {
        sessions,
        downloads,
        clients,
        issuer,
        index: index.clone(),
        cookie,
        projects: Arc::new(
            crate::projects::project_configs(settings)?
                .into_iter()
                .map(|project| project.profile.name)
                .collect(),
        ),
        provider,
        oidc_states,
    }))
}

/// The provider of `PACS_OIDC_ISSUER`, with the rest of its settings, discovered.
async fn open_provider(
    settings: &Settings,
    issuer: &str,
    env: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<crate::oidc::Provider> {
    let needed = |value: &Option<String>, name: &str| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .with_context(|| format!("PACS_OIDC_ISSUER is set: {name} must be set too"))
    };
    let client_id = needed(&settings.oidc_client_id, "PACS_OIDC_CLIENT_ID")?;
    let redirect_url = needed(&settings.oidc_redirect_url, "PACS_OIDC_REDIRECT_URL")?;
    let client_secret = crate::secret("PACS_OIDC_CLIENT_SECRET", &env)?.context(
        "PACS_OIDC_ISSUER is set: PACS_OIDC_CLIENT_SECRET (or PACS_OIDC_CLIENT_SECRET_FILE) \
         must hold the client's secret at the provider",
    )?;
    let mut admins = Vec::new();
    for admin in &settings.oidc_admins {
        let admin = admin.trim().to_lowercase();
        if admin.is_empty() {
            continue;
        }
        if !admin.contains('@') || admin.chars().any(char::is_whitespace) {
            anyhow::bail!("PACS_OIDC_ADMINS: {admin:?} is not an email address");
        }
        admins.push(admin);
    }
    let audience = match settings.oidc_audience.as_deref().map(str::trim) {
        Some(audience) if !audience.is_empty() => audience.to_owned(),
        _ => url::Url::parse(&redirect_url)
            .ok()
            .filter(|url| url.has_host())
            .map(|url| url.origin().ascii_serialization())
            .context("PACS_OIDC_REDIRECT_URL: not a URL with a host")?,
    };
    crate::oidc::Provider::discover(crate::oidc::ProviderSettings {
        issuer: issuer.trim().to_owned(),
        client_id,
        client_secret,
        redirect_url,
        scopes: settings
            .oidc_scopes
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        role: settings.oidc_role.trim().to_owned(),
        admins,
        audience,
    })
    .await
    .context("PACS_OIDC_ISSUER")
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
    /// A client, not a person: `id` is the client's and `login` its name.
    pub client: bool,
}

/// The projects a request's account is limited to, put there by the guard beside
/// [`SeenBy`]; absent for an account that sees everything.
#[derive(Debug, Clone)]
pub struct Limited(pub Vec<String>);

/// The key of a user's projects in the index.
fn user_account(id: &str) -> String {
    format!("user:{id}")
}

/// The key of a client's projects in the index.
fn client_account(id: &str) -> String {
    format!("client:{id}")
}

impl SignedIn {
    /// The key of this account's projects in the index.
    fn account(&self) -> String {
        if self.client {
            client_account(&self.id)
        } else {
            user_account(&self.id)
        }
    }

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
        client: false,
    })
}

/// What a client's token allows now: its scope, less what its role no longer gives. `None`
/// when the client is gone (revoked) or the token is not one of ours.
async fn client_of(login: &Login, claims: &Claims) -> Result<Option<SignedIn>, pacs_index::Error> {
    let (Some(id), Some(scope)) = (
        claims.get(CLIENT_CLAIM).and_then(Value::as_str),
        claims.get("scope").and_then(Value::as_str),
    ) else {
        return Ok(None);
    };
    let Some(client) = login.index.client(id).await? else {
        return Ok(None);
    };
    // a role that is gone allows nothing
    let role = login.index.role(&client.role).await?;
    let allowed = role.map(|role| role.permissions).unwrap_or_default();
    let permissions = scope
        .split(' ')
        .filter(|p| allowed.iter().any(|a| a == p))
        .filter_map(|p| p.parse().ok())
        .filter(|p| CLIENT_SCOPES.contains(p))
        .collect();
    Ok(Some(SignedIn {
        id: client.id,
        login: client.name,
        role: client.role,
        permissions,
        client: true,
    }))
}

/// A client's request refused. IUA (ITI TF-2: 3.72.4.3) asks for 401 whether the token or
/// its scope fails; the challenge says which.
/// The scopes a provider gives a tool, as it names them there: `pacs:read`, `pacs:write`,
/// `pacs:remote`, `pacs:delete`. Prefixed, since the provider serves other applications too.
const PROVIDER_SCOPE_PREFIX: &str = "pacs:";

/// The tool a provider's access token is for, with what it may do: the tool registered under
/// the token's client id (`client_id`, or `azp` from a provider that does not send it), with
/// its role, narrowed by the token's `pacs:` scopes when it has any. `None`: a token the
/// provider did not issue for this server, or for no tool registered here.
async fn provider_client(
    login: &Login,
    provider: &crate::oidc::Provider,
    token: &str,
) -> Result<Option<SignedIn>, pacs_index::Error> {
    let claims = match provider.access_claims(token).await {
        Ok(claims) => claims,
        Err(failure) => {
            if matches!(
                failure,
                crate::oidc::Failure::Provider(_) | crate::oidc::Failure::Internal(_)
            ) {
                tracing::error!(
                    detail = failure.detail(),
                    "a tool's token could not be checked"
                );
            } else {
                tracing::info!(detail = failure.detail(), "a tool's token was refused");
            }
            return Ok(None);
        }
    };
    let Some(id) = claims
        .get("client_id")
        .or_else(|| claims.get("azp"))
        .and_then(Value::as_str)
    else {
        tracing::info!("a provider's token names no client");
        return Ok(None);
    };
    let Some(client) = login.index.client(id).await? else {
        tracing::info!(client = %id, "a provider's token for a tool not registered here");
        return Ok(None);
    };
    // a role that is gone allows nothing
    let role = login.index.role(&client.role).await?;
    let allowed: Vec<Permission> = role
        .map(|role| role.permissions)
        .unwrap_or_default()
        .iter()
        .filter_map(|p| p.parse().ok())
        .filter(|p| CLIENT_SCOPES.contains(p))
        .collect();
    let asked: Vec<Permission> = claims
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split(' ')
        .filter_map(|scope| scope.strip_prefix(PROVIDER_SCOPE_PREFIX))
        .filter_map(|p| p.parse().ok())
        .collect();
    // the scopes only narrow the role, and no pacs: scope at all leaves it whole
    let permissions = if asked.is_empty() {
        allowed
    } else {
        allowed.into_iter().filter(|p| asked.contains(p)).collect()
    };
    Ok(Some(SignedIn {
        id: client.id,
        login: client.name,
        role: client.role,
        permissions,
        client: true,
    }))
}

fn client_refused(error: &'static str, detail: &str) -> Response {
    let mut response = refused(StatusCode::UNAUTHORIZED, detail).into_response();
    let challenge = match error {
        "insufficient_scope" => HeaderValue::from_static("Bearer error=\"insufficient_scope\""),
        _ => HeaderValue::from_static("Bearer error=\"invalid_token\""),
    };
    response.headers_mut().insert(WWW_AUTHENTICATE, challenge);
    response
}

/// Before every route: let open requests through, refuse the others without a valid access
/// token (401) or with a role that does not allow them (403). The answer to a request that
/// came with a valid token carries its [`SignedIn`], refused or not.
/// Rights per project: for an account limited to some projects, or whose role does not have
/// `identities`, the request is marked with the account ([`SeenBy`], which bounds every
/// read of the archive behind the guard), and a limited one with its projects ([`Limited`]). Read from the index at each request, so a change holds
/// at once, for tokens already given out too.
///
/// The answer is the mark, for an account that is limited: the guard leaves it on the
/// response too, so that the audit trail around it can tell a study that is not there from
/// one the account may not see.
async fn bound(
    login: &Login,
    account: &SignedIn,
    request: &mut Request,
) -> Result<Option<SeenBy>, Refusal> {
    let key = account.account();
    let projects = login
        .index
        .account_projects(&key)
        .await
        .map_err(|e| internal("reading an account's projects", e))?;
    // read from the role now, not from the token: taking `identities` away holds at the
    // account's next request
    let role = login
        .index
        .role(&account.role)
        .await
        .map_err(|e| internal("reading a role", e))?;
    let identities = account.permissions.contains(&Permission::Identities)
        && role.is_some_and(|role| role.permissions.iter().any(|p| p == "identities"));
    if projects.is_none() && identities {
        return Ok(None);
    }
    let seen = SeenBy(key);
    request.extensions_mut().insert(seen.clone());
    if let Some(projects) = projects {
        request.extensions_mut().insert(Limited(projects));
    }
    Ok(Some(seen))
}

async fn guard(State(login): State<Login>, mut request: Request, next: Next) -> Response {
    let (method, path) = (request.method().clone(), request.uri().path().to_owned());
    if login.provider.is_some() && method == Method::POST && path == "/auth/login" {
        return refused(StatusCode::FORBIDDEN, PROVIDER_ONLY).into_response();
    }
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
                // not a user's: a client's, which names this server as issuer and audience,
                // or one the provider gave a tool
                let client = if let Ok(claims) = login.clients.verify(&token) {
                    client_of(&login, &claims).await
                } else if let Some(provider) = &login.provider {
                    provider_client(&login, provider, &token).await
                } else {
                    return invalid();
                };
                let client = match client {
                    Ok(Some(client)) => client,
                    Ok(None) => {
                        return client_refused("invalid_token", "Invalid or expired access token");
                    }
                    Err(e) => return internal("reading a client", e).into_response(),
                };
                // nothing of the accounts for a client, whatever its role
                let right = right_for(&method, &path);
                if right.permission().is_none() || !client.may(right) {
                    tracing::info!(client = %client.login, role = %client.role, %method, %path, "refused: scope");
                    let mut response = client_refused(
                        "insufficient_scope",
                        "This token's scope does not allow this",
                    );
                    response.extensions_mut().insert(client);
                    return response;
                }
                let seen = match bound(&login, &client, &mut request).await {
                    Ok(seen) => seen,
                    Err(refusal) => return refusal.into_response(),
                };
                request.extensions_mut().insert(client.clone());
                let mut response = next.run(request).await;
                response.extensions_mut().insert(client);
                if let Some(seen) = seen {
                    response.extensions_mut().insert(seen);
                }
                return response;
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
        let mut response =
            refused(StatusCode::FORBIDDEN, "Your role does not allow this").into_response();
        response.extensions_mut().insert(user);
        return response;
    }
    let seen = match bound(&login, &user, &mut request).await {
        Ok(seen) => seen,
        Err(refusal) => return refusal.into_response(),
    };
    request.extensions_mut().insert(user.clone());
    let mut response = next.run(request).await;
    // who asked, for the audit trail around the guard
    response.extensions_mut().insert(user);
    if let Some(seen) = seen {
        response.extensions_mut().insert(seen);
    }
    response
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

/// Makes a link a browser can follow.
///
/// A token good for one minute and one `GET` of one path, to append to it as `?token=`: for a download the browser starts itself, where no header can be set. Refused for a path the account may not read.
#[utoipa::path(
    post,
    path = "/auth/download-token",
    tag = "Signing in",
    request_body(content = DownloadRequest, description = "The path the link is for."),
    responses(
        (status = 200, description = "The token.", body = DownloadToken),
        (status = 403, description = "The account may not read this path, or the path is one no link is made for (the accounts).", body = Refused),
        (status = 422, description = "Not a path of this server.", body = Refused)
    )
)]
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

/// Who the token is.
///
/// For a user or a tool, with the projects it is limited to.
#[utoipa::path(
    get,
    path = "/auth/me",
    tag = "Signing in",
    responses(
        (status = 200, description = "The account.", body = Me)
    )
)]
async fn me(
    Extension(user): Extension<SignedIn>,
    limited: Option<Extension<Limited>>,
) -> Json<Value> {
    let permissions: Vec<&str> = user.permissions.iter().map(|p| p.as_str()).collect();
    let projects = limited.map(|Extension(Limited(projects))| projects);
    Json(json!({
        "id": user.id, "login": user.login, "role": user.role, "permissions": permissions,
        "projects": projects,
    }))
}

/// Lists the server's projects.
///
/// The names an account can be limited to: those of `PACS_PROJECTS`.
#[utoipa::path(
    get,
    path = "/projects",
    tag = "Users",
    responses(
        (status = 200, description = "The projects' names.", body = Vec<String>)
    )
)]
async fn list_projects(State(login): State<Login>) -> Json<Value> {
    Json(json!(*login.projects))
}

/// A field of an update that may be absent (no change), `null` or a value.
fn given<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// The projects asked for an account, each one a project of this server, none twice; or
/// the 422 that names the one that is not.
fn known_projects(
    login: &Login,
    asked: Option<Vec<String>>,
) -> Result<Option<Vec<String>>, Refusal> {
    let Some(mut asked) = asked else {
        return Ok(None);
    };
    if let Some(unknown) = asked.iter().find(|name| !login.projects.contains(name)) {
        return Err(unprocessable(format!(
            "{unknown:?} is not a project of this server"
        )));
    }
    asked.sort();
    asked.dedup();
    Ok(Some(asked))
}

/// Keeps an account's projects (`None`: it sees everything) and answers with them.
async fn keep_projects(
    login: &Login,
    account: &str,
    projects: Option<Vec<String>>,
) -> Result<Value, Refusal> {
    login
        .index
        .set_account_projects(account, projects.as_deref())
        .await
        .map_err(|e| from_index("keeping an account's projects", e))?;
    Ok(json!(projects))
}

/// An account's projects as shown: a list, or `null` when it sees everything.
async fn shown_projects(login: &Login, account: &str) -> Result<Value, Refusal> {
    let projects = login
        .index
        .account_projects(account)
        .await
        .map_err(|e| from_index("reading an account's projects", e))?;
    Ok(json!(projects))
}

/// A user's own change of password.
#[derive(Deserialize, ToSchema)]
struct PasswordChange {
    current_password: String,
    new_password: String,
}

/// Changes the user's own password.
///
/// Every session of the user ends, this one included. Not for a tool, which has no password.
#[utoipa::path(
    post,
    path = "/auth/password",
    tag = "Signing in",
    request_body(content = PasswordChange, description = "The password now, and the new one: 12 to 1024 bytes."),
    responses(
        (status = 204, description = "Changed."),
        (status = 403, description = "The current password is wrong.", body = Refused),
        (status = 422, description = "The new password is too short or too long.", body = Refused)
    )
)]
async fn change_password(
    State(login): State<Login>,
    Extension(user): Extension<SignedIn>,
    change: Result<Json<PasswordChange>, JsonRejection>,
) -> Result<Response, Refusal> {
    if login.provider.is_some() {
        return Err(refused(StatusCode::FORBIDDEN, PROVIDER_ONLY));
    }
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

/// Lists the users.
#[utoipa::path(
    get,
    path = "/users",
    tag = "Users",
    responses(
        (status = 200, description = "Every user, by login.", body = Vec<User>)
    )
)]
async fn list_users(State(login): State<Login>) -> Result<Json<Value>, Refusal> {
    let users = login
        .index
        .users()
        .await
        .map_err(|e| from_index("listing users", e))?;
    let mut list = Vec::with_capacity(users.len());
    for user in &users {
        let mut one = shown(user);
        one["projects"] = shown_projects(&login, &user_account(&user.id)).await?;
        list.push(one);
    }
    Ok(Json(Value::Array(list)))
}

/// A user to add.
#[derive(Deserialize, ToSchema)]
struct NewUser {
    login: String,
    password: String,
    role: String,
    /// The projects to limit the user to; absent or `null`: every study.
    #[serde(default)]
    projects: Option<Vec<String>>,
}

/// Adds a user.
#[utoipa::path(
    post,
    path = "/users",
    tag = "Users",
    request_body(content = NewUser, description = "The login, the first password, the role, and the projects to limit them to."),
    responses(
        (status = 201, description = "The user.", body = User),
        (status = 409, description = "The login is taken.", body = Refused),
        (status = 422, description = "A login with spaces, a password too short, a role or a project this server does not have.", body = Refused)
    )
)]
async fn create_user(
    State(login): State<Login>,
    new: Result<Json<NewUser>, JsonRejection>,
) -> Result<Response, Refusal> {
    if login.provider.is_some() {
        return Err(refused(
            StatusCode::FORBIDDEN,
            "This server signs people in through its OpenID provider: an account is made at its first sign-in",
        ));
    }
    let new = body(new)?;
    let name = normal_login(&new.login).map_err(unprocessable)?;
    let role = known_role(&login, &new.role).await?;
    let projects = known_projects(&login, new.projects)?;
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
    let mut shown = shown(&user);
    shown["projects"] = keep_projects(&login, &user_account(&user.id), projects).await?;
    Ok((StatusCode::CREATED, Json(shown)).into_response())
}

/// What to change of a user. What is left out stays as it is.
#[derive(Deserialize, ToSchema)]
struct UserUpdate {
    password: Option<String>,
    role: Option<String>,
    active: Option<bool>,
    /// Absent: no change. `null`: every study. A list: only those projects.
    #[serde(default, deserialize_with = "given")]
    #[schema(value_type = Option<Vec<String>>)]
    projects: Option<Option<Vec<String>>>,
}

/// Changes a user.
///
/// A new password or a deactivation ends the user's sessions at once. A new role reaches their access token at its next refresh, within 15 minutes. New projects hold from their next request.
#[utoipa::path(
    put,
    path = "/users/{id}",
    tag = "Users",
    params(("id" = String, Path, description = "The user's id.")),
    request_body(content = UserUpdate, description = "What to change; what is left out stays."),
    responses(
        (status = 200, description = "The user after the change.", body = User),
        (status = 404, description = "No such user.", body = Refused),
        (status = 409, description = "The last active administrator cannot be deactivated or given another role.", body = Refused),
        (status = 422, description = "A password too short, a role or a project this server does not have.", body = Refused)
    )
)]
async fn update_user(
    State(login): State<Login>,
    Path(id): Path<String>,
    update: Result<Json<UserUpdate>, JsonRejection>,
) -> Result<Response, Refusal> {
    let update = body(update)?;
    if login.provider.is_some() && update.password.is_some() {
        return Err(refused(StatusCode::FORBIDDEN, PROVIDER_ONLY));
    }
    let role = match update.role.as_deref() {
        Some(role) => Some(known_role(&login, role).await?),
        None => None,
    };
    let projects = match update.projects {
        Some(asked) => Some(known_projects(&login, asked)?),
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
    let account = user_account(&user.id);
    let mut shown = shown(&user);
    shown["projects"] = match projects {
        Some(projects) => keep_projects(&login, &account, projects).await?,
        None => shown_projects(&login, &account).await?,
    };
    Ok(Json(shown).into_response())
}

/// Removes a user.
///
/// Their sessions end at once and the login is free again.
#[utoipa::path(
    delete,
    path = "/users/{id}",
    tag = "Users",
    params(("id" = String, Path, description = "The user's id.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 404, description = "No such user.", body = Refused),
        (status = 409, description = "The last active administrator cannot be removed.", body = Refused)
    )
)]
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
    keep_projects(&login, &user_account(&id), None).await?;
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

/// Lists the roles.
///
/// For any signed-in user, who may see what each role allows.
#[utoipa::path(
    get,
    path = "/roles",
    tag = "Roles",
    responses(
        (status = 200, description = "Every role.", body = Vec<Role>)
    )
)]
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

/// A role to add.
#[derive(Deserialize, ToSchema)]
struct NewRole {
    name: String,
    #[serde(default)]
    permissions: Vec<String>,
}

/// Adds a role.
#[utoipa::path(
    post,
    path = "/roles",
    tag = "Roles",
    request_body(content = NewRole, description = "Its name and its permissions."),
    responses(
        (status = 201, description = "The role.", body = Role),
        (status = 409, description = "A role has the id this name makes.", body = Refused),
        (status = 422, description = "A name too long or without a letter or digit, or a permission that is not one.", body = Refused)
    )
)]
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

/// What to change of a role. What is left out stays as it is.
#[derive(Deserialize, ToSchema)]
struct RoleUpdate {
    name: Option<String>,
    permissions: Option<Vec<String>>,
}

/// Changes a role.
///
/// Its name, its permissions, or both. New permissions reach each user's access token at its next refresh, and a tool at its next request.
#[utoipa::path(
    put,
    path = "/roles/{id}",
    tag = "Roles",
    params(("id" = String, Path, description = "The role's id.")),
    request_body(content = RoleUpdate, description = "What to change; what is left out stays."),
    responses(
        (status = 200, description = "The role after the change.", body = Role),
        (status = 404, description = "No such role.", body = Refused),
        (status = 409, description = "The `admin` role cannot be changed.", body = Refused),
        (status = 422, description = "A name or a permission that is not valid.", body = Refused)
    )
)]
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

/// Removes a role.
#[utoipa::path(
    delete,
    path = "/roles/{id}",
    tag = "Roles",
    params(("id" = String, Path, description = "The role's id.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 404, description = "No such role.", body = Refused),
        (status = 409, description = "The `admin` role cannot be removed, nor a role a user or a tool still has.", body = Refused)
    )
)]
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

/// What a client's token may hold: everything but the accounts.
const CLIENT_SCOPES: [Permission; 5] = [
    Permission::Read,
    Permission::Write,
    Permission::Remote,
    Permission::Delete,
    Permission::Identities,
];

/// Client secrets are 32 random bytes in hexadecimal: nothing to guess, so a plain SHA-256
/// is what the index keeps.
fn new_secret() -> anyhow::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no random bytes: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn secret_hash(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Equal, in a time that does not depend on where they differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// The id and secret of `Authorization: Basic`, as a client sends them to the token route.
pub(crate) fn basic_credentials(header: Option<&HeaderValue>) -> Option<(String, String)> {
    let (scheme, encoded) = header?.to_str().ok()?.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let (id, secret) = std::str::from_utf8(&decoded).ok()?.split_once(':')?;
    Some((id.to_owned(), secret.to_owned()))
}

/// The answer of the token route: never kept by a cache (OAuth 2.1, 3.2.3).
fn token_reply(status: StatusCode, body: Value) -> Response {
    let mut response = (status, Json(body)).into_response();
    let headers = response.headers_mut();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

/// A token request refused, as OAuth 2.1 (3.2.4) words it.
fn token_error(error: &str, description: &str) -> Response {
    let unknown_client = error == "invalid_client";
    let status = if unknown_client {
        StatusCode::UNAUTHORIZED
    } else {
        StatusCode::BAD_REQUEST
    };
    let mut response = token_reply(
        status,
        json!({ "error": error, "error_description": description }),
    );
    if unknown_client {
        response
            .headers_mut()
            .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Basic"));
    }
    response
}

/// The scope a client gets: what it asks for, or without a `scope` everything its role
/// gives a client. Asking for more than that is an error, not a smaller token.
fn granted_scope(asked: Option<&str>, role: &[String]) -> Result<Vec<Permission>, String> {
    let allowed: Vec<Permission> = CLIENT_SCOPES
        .into_iter()
        .filter(|p| role.iter().any(|r| r == p.as_str()))
        .collect();
    let Some(asked) = asked else {
        return Ok(allowed);
    };
    let mut wanted = Vec::new();
    for name in asked.split(' ').filter(|name| !name.is_empty()) {
        let known = CLIENT_SCOPES.into_iter().find(|p| p.as_str() == name);
        match known.filter(|p| allowed.contains(p)) {
            Some(permission) => wanted.push(permission),
            None => return Err(format!("scope {name:?} is not one this client may have")),
        }
    }
    // in the order of the permissions, each once
    Ok(allowed.into_iter().filter(|p| wanted.contains(p)).collect())
}

/// Gives a tool an access token.
///
/// The client credentials grant, as IHE's Internet User Authorization profile uses it (Get Access Token, ITI-71): the tool's id and secret go in `Authorization: Basic`, the rest in a form. The token is good for five minutes; the tool asks again when it has run out.
#[utoipa::path(
    post,
    path = "/auth/token",
    tag = "Tools",
    request_body(content = TokenRequest, content_type = "application/x-www-form-urlencoded", description = "The grant, and optionally less than the role gives."),
    responses(
        (status = 200, description = "The token.", body = Token),
        (status = 400, description = "The request is not a client credentials grant, or asks for a scope the role does not give or for another server.", body = TokenRefused),
        (status = 401, description = "The id and secret are missing or not those of a tool.", body = TokenRefused)
    )
)]
async fn token(
    State(login): State<Login>,
    headers: axum::http::HeaderMap,
    form: Result<Form<Vec<(String, String)>>, FormRejection>,
) -> Response {
    if login.provider.is_some() {
        return token_error(
            "unauthorized_client",
            "tools get their access tokens from this server's OpenID provider",
        );
    }
    let Ok(Form(form)) = form else {
        return token_error(
            "invalid_request",
            "the body is a form: application/x-www-form-urlencoded",
        );
    };
    let mut fields = std::collections::HashMap::new();
    for (name, value) in &form {
        if fields.insert(name.as_str(), value.as_str()).is_some() {
            return token_error("invalid_request", &format!("{name} is given twice"));
        }
    }
    match fields.get("grant_type") {
        Some(&"client_credentials") => {}
        Some(_) => {
            return token_error(
                "unsupported_grant_type",
                "the grant type is client_credentials",
            );
        }
        None => return token_error("invalid_request", "grant_type is missing"),
    }
    let wrong = || token_error("invalid_client", "Unknown client or wrong secret");
    let Some((id, secret)) = basic_credentials(headers.get(AUTHORIZATION)) else {
        return wrong();
    };
    let client = match login.index.client(&id).await {
        Ok(client) => client,
        Err(e) => return internal("reading a client", e).into_response(),
    };
    // hashed and compared even for an unknown id, so that both take the same time
    let stored = client
        .as_ref()
        .map(|c| c.secret_hash.as_str())
        .unwrap_or("");
    let (Some(client), true) = (&client, same(&secret_hash(&secret), stored)) else {
        return wrong();
    };
    if let Some(kind) = fields.get("requested_token_type")
        && *kind != "urn:ietf:params:oauth:token-type:jwt"
    {
        return token_error("invalid_request", "the only token type is the JWT one");
    }
    if let Some(resource) = fields.get("resource")
        && *resource != login.issuer
    {
        return token_error(
            "invalid_target",
            &format!("the resource here is {}", login.issuer),
        );
    }
    let role = match login.index.role(&client.role).await {
        Ok(role) => role.map(|role| role.permissions).unwrap_or_default(),
        Err(e) => return internal("reading a role", e).into_response(),
    };
    let scope = match granted_scope(fields.get("scope").copied(), &role) {
        Ok(scope) if scope.is_empty() => {
            return token_error("invalid_scope", "this client's role allows it nothing");
        }
        Ok(scope) => scope,
        Err(description) => return token_error("invalid_scope", &description),
    };
    let scope = scope
        .iter()
        .map(|p| p.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let mut claims = serde_json::Map::new();
    claims.insert(CLIENT_CLAIM.into(), client.id.as_str().into());
    claims.insert("scope".into(), scope.as_str().into());
    let token = match login.clients.issue(&client.id, &claims) {
        Ok(token) => token,
        Err(e) => return internal("making a client's access token", e).into_response(),
    };
    if let Err(e) = login.index.client_got_token(&client.id).await {
        return internal("noting a client's token", e).into_response();
    }
    tracing::info!(client = %client.name, %scope, "access token given to a client");
    token_reply(
        StatusCode::OK,
        json!({
            "access_token": token,
            "token_type": "Bearer",
            "scope": scope,
            "expires_in": CLIENT_TOKEN_SECONDS,
        }),
    )
}

fn shown_client(client: &StoredClient) -> Value {
    json!({
        "id": client.id,
        "name": client.name,
        "role": client.role,
        "created_by": client.created_by,
        "created_ms": client.created_ms,
        "last_token_ms": client.last_token_ms,
    })
}

/// A client's name as shown: 1 to 60 characters, none of them a control character.
fn client_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err("a name is 1 to 60 characters".into());
    }
    Ok(name.to_owned())
}

/// Lists the tools.
#[utoipa::path(
    get,
    path = "/clients",
    tag = "Tools",
    responses(
        (status = 200, description = "Every tool, oldest first; never a secret.", body = Vec<Tool>)
    )
)]
async fn list_clients(State(login): State<Login>) -> Result<Json<Value>, Refusal> {
    let clients = login
        .index
        .clients()
        .await
        .map_err(|e| from_index("listing clients", e))?;
    let mut list = Vec::with_capacity(clients.len());
    for client in &clients {
        let mut one = shown_client(client);
        one["projects"] = shown_projects(&login, &client_account(&client.id)).await?;
        list.push(one);
    }
    Ok(Json(Value::Array(list)))
}

/// A tool to add.
#[derive(Deserialize, ToSchema)]
struct NewClient {
    /// With an OpenID provider: the tool's client id there, which its tokens name. Without
    /// one it is not given: the server makes the id and a secret.
    #[serde(default)]
    client_id: Option<String>,
    name: String,
    role: String,
    /// The projects to limit the client to; absent or `null`: every study.
    #[serde(default)]
    projects: Option<Vec<String>>,
}

/// A client id at the provider, as a tool is registered under it: 1 to 128 visible ASCII
/// characters (RFC 6749 A.1 allows those).
fn provider_client_id(id: &str) -> Result<String, String> {
    let id = id.trim();
    if id.is_empty() || id.len() > 128 || !id.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("client_id: 1 to 128 visible ASCII characters".into());
    }
    Ok(id.to_owned())
}

/// Adds a tool.
///
/// The answer is the only place its secret ever appears: the index keeps a hash of it. With an OpenID provider, the tool is registered under its `client_id` there and has no secret here: it gets its tokens from the provider.
#[utoipa::path(
    post,
    path = "/clients",
    tag = "Tools",
    request_body(content = NewClient, description = "Its name, its role, and the projects to limit it to."),
    responses(
        (status = 201, description = "The tool, with its `secret`.", body = Tool),
        (status = 422, description = "A name too long, a role or a project this server does not have.", body = Refused)
    )
)]
async fn create_client(
    State(login): State<Login>,
    Extension(user): Extension<SignedIn>,
    new: Result<Json<NewClient>, JsonRejection>,
) -> Result<Response, Refusal> {
    let new = body(new)?;
    let name = client_name(&new.name).map_err(unprocessable)?;
    let role = known_role(&login, &new.role).await?;
    let projects = known_projects(&login, new.projects)?;
    // with a provider, the tool is the provider's client and has no secret here
    let (id, secret) = match (&login.provider, new.client_id) {
        (Some(_), Some(id)) => (provider_client_id(&id).map_err(unprocessable)?, None),
        (Some(_), None) => {
            return Err(unprocessable(
                "client_id: the tool's client id at the OpenID provider".into(),
            ));
        }
        (None, Some(_)) => {
            return Err(unprocessable(
                "client_id: only with an OpenID provider; this server makes the id".into(),
            ));
        }
        (None, None) => (
            new_id().map_err(|e| internal("making a client id", e))?,
            Some(new_secret().map_err(|e| internal("making a client secret", e))?),
        ),
    };
    // no secret: a stored hash that the hash of no secret is
    let hash = secret.as_deref().map(secret_hash).unwrap_or_default();
    let client = login
        .index
        .create_client(&id, &name, &hash, &role, &user.login)
        .await
        .map_err(|e| from_index("making a client", e))?;
    tracing::info!(client = %client.name, role = %client.role, by = %user.login, "client made");
    let mut shown = shown_client(&client);
    shown["projects"] = keep_projects(&login, &client_account(&client.id), projects).await?;
    if let Some(secret) = secret {
        shown["secret"] = secret.into();
    }
    Ok(token_reply(StatusCode::CREATED, shown))
}

/// What to change of a tool. What is left out stays as it is.
#[derive(Deserialize, ToSchema)]
struct ClientUpdate {
    name: Option<String>,
    role: Option<String>,
    /// Absent: no change. `null`: every study. A list: only those projects.
    #[serde(default, deserialize_with = "given")]
    #[schema(value_type = Option<Vec<String>>)]
    projects: Option<Option<Vec<String>>>,
}

/// Changes a tool.
///
/// Its name, role or projects. They hold from its next request, tokens it already has included.
#[utoipa::path(
    put,
    path = "/clients/{id}",
    tag = "Tools",
    params(("id" = String, Path, description = "The tool's id.")),
    request_body(content = ClientUpdate, description = "What to change; what is left out stays."),
    responses(
        (status = 200, description = "The tool after the change.", body = Tool),
        (status = 404, description = "No such tool.", body = Refused),
        (status = 422, description = "A name too long, a role or a project this server does not have.", body = Refused)
    )
)]
async fn update_client(
    State(login): State<Login>,
    Path(id): Path<String>,
    update: Result<Json<ClientUpdate>, JsonRejection>,
) -> Result<Response, Refusal> {
    let update = body(update)?;
    let name = update
        .name
        .as_deref()
        .map(client_name)
        .transpose()
        .map_err(unprocessable)?;
    let role = match update.role.as_deref() {
        Some(role) => Some(known_role(&login, role).await?),
        None => None,
    };
    let projects = match update.projects {
        Some(asked) => Some(known_projects(&login, asked)?),
        None => None,
    };
    let client = login
        .index
        .update_client(&id, name.as_deref(), role.as_deref())
        .await
        .map_err(|e| from_index("changing a client", e))?
        .ok_or_else(|| refused(StatusCode::NOT_FOUND, "No such client"))?;
    let account = client_account(&client.id);
    let mut shown = shown_client(&client);
    shown["projects"] = match projects {
        Some(projects) => keep_projects(&login, &account, projects).await?,
        None => shown_projects(&login, &account).await?,
    };
    Ok(Json(shown).into_response())
}

/// Removes a tool.
///
/// Its secret gets no more tokens, and those it has stop at its next request.
#[utoipa::path(
    delete,
    path = "/clients/{id}",
    tag = "Tools",
    params(("id" = String, Path, description = "The tool's id.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 404, description = "No such tool.", body = Refused)
    )
)]
async fn delete_client(
    State(login): State<Login>,
    Extension(user): Extension<SignedIn>,
    Path(id): Path<String>,
) -> Result<Response, Refusal> {
    let deleted = login
        .index
        .delete_client(&id)
        .await
        .map_err(|e| from_index("removing a client", e))?;
    if !deleted {
        return Err(refused(StatusCode::NOT_FOUND, "No such client"));
    }
    keep_projects(&login, &client_account(&id), None).await?;
    tracing::info!(client = %id, by = %user.login, "client removed");
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The login's own routes, each one by its `#[utoipa::path]`, so that none is served that
/// the API's description does not have.
fn described() -> OpenApiRouter<Login> {
    OpenApiRouter::new()
        .routes(routes!(me))
        .routes(routes!(change_password))
        .routes(routes!(download_token))
        .routes(routes!(list_users, create_user))
        .routes(routes!(update_user, delete_user))
        .routes(routes!(list_roles, create_role))
        .routes(routes!(update_role, delete_role))
        .routes(routes!(token))
        .routes(routes!(list_projects))
        .routes(routes!(list_clients, create_client))
        .routes(routes!(update_client, delete_client))
}

/// The four routes of the sessions library, described here since they are served from
/// there: `tests/auth.rs` holds each to what the running server answers.
mod session_routes {
    use super::{Credentials, Refused, Session};

    /// Signs in.
    ///
    /// The access token comes in the answer, and the refresh token in an `HttpOnly` cookie
    /// sent to `/auth/*` alone. A wrong password and an unknown login get the same answer.
    #[utoipa::path(
        post,
        path = "/auth/login",
        tag = "Signing in",
        request_body(content = Credentials, description = "The login and the password."),
        responses(
            (status = 200, description = "Signed in.", body = Session),
            (status = 401, description = "The login or the password is wrong, or the account is deactivated.", body = Refused)
        )
    )]
    #[allow(dead_code)]
    pub(super) fn login() {}

    /// Carries a session on.
    ///
    /// Spends the refresh cookie and sets its successor. A cookie already spent ends the
    /// session, since two parties then hold it.
    #[utoipa::path(
        post,
        path = "/auth/refresh",
        tag = "Signing in",
        responses(
            (status = 200, description = "A new access token, and the next cookie.", body = Session),
            (status = 401, description = "No cookie, or one that is spent, expired or of a session that ended.", body = Refused)
        )
    )]
    #[allow(dead_code)]
    pub(super) fn refresh() {}

    /// Signs out of this session.
    ///
    /// By the refresh cookie; the access token already given lasts until it runs out.
    #[utoipa::path(
        post,
        path = "/auth/logout",
        tag = "Signing in",
        responses((status = 204, description = "The session ended, if there was one."))
    )]
    #[allow(dead_code)]
    pub(super) fn logout() {}

    /// Signs out everywhere.
    ///
    /// Ends every session of the user the access token is.
    #[utoipa::path(
        post,
        path = "/auth/logout-all",
        tag = "Signing in",
        responses((status = 204, description = "Every session of the user ended."))
    )]
    #[allow(dead_code)]
    pub(super) fn logout_all() {}

    #[derive(utoipa::OpenApi)]
    #[openapi(paths(login, refresh, logout, logout_all))]
    pub(super) struct Described;
}

/// The description of the login's routes, for writing the reference: its own, and those
/// of the sessions library.
pub(crate) fn api() -> OpenApi {
    let mut api = described().into_openapi();
    api.merge(<session_routes::Described as utoipa::OpenApi>::openapi());
    api
}

/// A sign-in through the provider, as the audit trail records it: who the provider said it
/// was (or nothing, when it got no further), and whether they were let in.
#[derive(Debug, Clone)]
pub struct ProviderSignIn {
    pub login: String,
    pub accepted: bool,
    pub detail: Option<String>,
}

/// What the sign-in page offers.
async fn methods(State(login): State<Login>) -> Json<Value> {
    let provider = login.provider.is_some();
    Json(json!({ "password": !provider, "provider": provider }))
}

impl Login {
    /// Whether people sign in through an OpenID provider, and never with a password.
    pub fn has_provider(&self) -> bool {
        self.provider.is_some()
    }

    /// The state cookie of a sign-in through the provider, scoped to `/auth/oidc` (behind the
    /// proxy's prefix when there is one); `max_age` 0 removes it.
    fn state_cookie(&self, value: &str, max_age: u64) -> String {
        let base = self.cookie.path.as_deref().unwrap_or("/auth");
        let mut cookie = format!(
            "{}={value}; Max-Age={max_age}; Path={}/oidc; HttpOnly; SameSite=Lax",
            crate::oidc::STATE_COOKIE,
            base.trim_end_matches('/')
        );
        if self.cookie.secure {
            cookie.push_str("; Secure");
        }
        cookie
    }

    /// Check what the provider sent back and give the session: the user's id and login, the
    /// session, and the web interface's path to land on.
    async fn provider_session(
        &self,
        provider: &crate::oidc::Provider,
        headers: &axum::http::HeaderMap,
        back: crate::oidc::Callback,
        login_name: &mut String,
    ) -> Result<(jwt_sessions::Session, String), crate::oidc::Failure> {
        use crate::oidc::Failure;
        let state = crate::oidc::cookie(headers, crate::oidc::STATE_COOKIE)
            .and_then(|token| self.oidc_states.verify(token).ok())
            .ok_or(Failure::State)?;
        let field = |name: &str| {
            state
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        if back.state.as_deref() != Some(field("state").as_str()) || field("state").is_empty() {
            return Err(Failure::State);
        }
        if let Some(error) = back.error {
            // RFC 6749 4.1.2.1: an ASCII code; anything else is not repeated
            let error: String = error
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(64)
                .collect();
            return Err(Failure::Refused(format!("the provider answered {error}")));
        }
        let code = back
            .code
            .filter(|code| !code.is_empty())
            .ok_or_else(|| Failure::Refused("no code".into()))?;
        let id_token = provider.exchange(&code, &field("verifier")).await?;
        let identity = provider.identity(&id_token, &field("nonce")).await?;
        let (name, vouched) = identity.login();
        let name = normal_login(&name)
            .map_err(|e| Failure::Account(format!("the login {name:?}: {e}")))?;
        *login_name = name.clone();
        let admin = vouched && provider.is_admin(&name);
        let role = if admin { ADMIN_ROLE } else { provider.role() };
        let internal =
            |what: &str, e: &dyn std::fmt::Display| Failure::Internal(format!("{what}: {e}"));
        if self
            .index
            .role(role)
            .await
            .map_err(|e| internal("reading a role", &e))?
            .is_none()
        {
            return Err(Failure::Account(format!("the role {role} is gone")));
        }
        let id = new_id().map_err(|e| internal("making a user id", &e))?;
        // no password ever matches this account's
        let unmatchable = crate::oidc::random(32).map_err(|e| internal("random bytes", &e))?;
        let hash = hashed(unmatchable)
            .await
            .map_err(|e| internal("hashing", &e))?;
        let (user, link) = self
            .index
            .provider_user(&pacs_index::ProviderUser {
                issuer: provider.issuer(),
                subject: &identity.subject,
                id: &id,
                login: &name,
                password_hash: &hash,
                role,
                link_by_login: vouched,
            })
            .await
            .map_err(|e| match e {
                pacs_index::Error::Conflict(detail) => Failure::Account(detail),
                e => internal("finding the account", &e),
            })?;
        *login_name = user.login.clone();
        if admin && user.role != ADMIN_ROLE {
            let change = UserChange {
                role: Some(ADMIN_ROLE.into()),
                ..UserChange::default()
            };
            self.index
                .update_user(&user.id, &change, ADMIN_ROLE)
                .await
                .map_err(|e| internal("making an administrator", &e))?;
        }
        let session = self.sessions.start(&user.id).await.map_err(|e| match e {
            jwt_sessions::AuthError::InvalidCredentials => {
                Failure::Account("the account is deactivated".into())
            }
            e => internal("starting a session", &e),
        })?;
        tracing::info!(login = %user.login, ?link, "signed in through the OpenID provider");
        Ok((session, field("next")))
    }
}

/// Starts a sign-in through the provider: to its sign-in page, with a state cookie.
async fn oidc_start(
    State(login): State<Login>,
    axum::extract::Query(start): axum::extract::Query<crate::oidc::Start>,
) -> Response {
    let Some(provider) = &login.provider else {
        return refused(StatusCode::NOT_FOUND, "This server has no OpenID provider")
            .into_response();
    };
    let made = (|| -> anyhow::Result<(String, String, String)> {
        Ok((
            crate::oidc::random(32)?,
            crate::oidc::random(32)?,
            // RFC 7636 4.1: 43 to 128 characters
            crate::oidc::random(48)?,
        ))
    })();
    let (state, nonce, verifier) = match made {
        Ok(made) => made,
        Err(e) => return internal("starting a sign-in", e).into_response(),
    };
    let mut claims = serde_json::Map::new();
    claims.insert("state".into(), state.clone().into());
    claims.insert("nonce".into(), nonce.clone().into());
    claims.insert("verifier".into(), verifier.clone().into());
    claims.insert(
        "next".into(),
        crate::oidc::landing(start.next.as_deref()).into(),
    );
    let token = match login.oidc_states.issue("sign-in", &claims) {
        Ok(token) => token,
        Err(e) => return internal("starting a sign-in", e).into_response(),
    };
    let cookie = login.state_cookie(&token, crate::oidc::STATE_TTL.as_secs());
    let to = provider.authorization_url(&state, &nonce, &crate::oidc::challenge(&verifier));
    crate::oidc::see_other(&to, &[cookie])
}

/// Where the provider sends the browser back: the session, then the web interface.
async fn oidc_callback(
    State(login): State<Login>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(back): axum::extract::Query<crate::oidc::Callback>,
) -> Response {
    let Some(provider) = &login.provider else {
        return refused(StatusCode::NOT_FOUND, "This server has no OpenID provider")
            .into_response();
    };
    let clear = login.state_cookie("", 0);
    let mut name = String::new();
    // from /auth/oidc/callback, wherever a proxy mounts the server
    let (mut response, accepted, detail) = match login
        .provider_session(provider, &headers, back, &mut name)
        .await
    {
        Ok((session, next)) => {
            let refresh = login.cookie.set_cookie(
                &session.refresh_token,
                login.sessions.refresh.ttl().as_secs(),
                "/auth",
            );
            match refresh {
                Ok(refresh) => (
                    crate::oidc::see_other(&format!("../../ui/{next}"), &[clear, refresh]),
                    true,
                    None,
                ),
                Err(e) => return internal("the refresh cookie", e).into_response(),
            }
        }
        Err(failure) => {
            if matches!(
                failure,
                crate::oidc::Failure::Internal(_) | crate::oidc::Failure::Provider(_)
            ) {
                tracing::error!(
                    reason = failure.reason(),
                    detail = failure.detail(),
                    "a sign-in through the provider failed"
                );
            } else {
                tracing::info!(
                    reason = failure.reason(),
                    detail = failure.detail(),
                    "a sign-in through the provider was refused"
                );
            }
            (
                crate::oidc::see_other(
                    &format!("../../ui/?signin_error={}", failure.reason()),
                    &[clear],
                ),
                false,
                Some(failure.detail().to_owned()),
            )
        }
    };
    response.extensions_mut().insert(ProviderSignIn {
        login: name,
        accepted,
        detail,
    });
    response
}

impl Login {
    /// The sign-in routes (`/auth/login`, `/auth/refresh`, `/auth/logout`,
    /// `/auth/logout-all`), `GET /auth/me`, `POST /auth/password`, `/users`, `/roles`, `POST /auth/token` and
    /// `/clients`.
    pub fn router(&self) -> anyhow::Result<Router> {
        let sessions: Router = auth_router(self.sessions.clone(), self.cookie.clone())
            .context("the refresh cookie")?;
        let (own, _) = described().with_state(self.clone()).split_for_parts();
        let provider: Router = Router::new()
            .route("/auth/methods", axum::routing::get(methods))
            .route("/auth/oidc/start", axum::routing::get(oidc_start))
            .route("/auth/oidc/callback", axum::routing::get(oidc_callback))
            .with_state(self.clone());
        Ok(sessions.merge(own).merge(provider))
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
            client: false,
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
        for path in [
            "/auth/login",
            "/auth/refresh",
            "/auth/logout",
            "/auth/token",
        ] {
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
            (Method::POST, "/modalities/hospital/store", Remote),
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
            (Method::GET, "/clients", Admin),
            (Method::POST, "/clients", Admin),
            (Method::PUT, "/clients/c1", Admin),
            (Method::DELETE, "/clients/c1", Admin),
            (Method::GET, "/clientsx", Read),
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

    #[test]
    fn the_scope_a_client_gets() {
        let role = |list: &[&str]| list.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
        use Permission::*;
        let editor = role(&["read", "write", "remote"]);
        // without a scope: all its role gives a client
        assert_eq!(granted_scope(None, &editor), Ok(vec![Read, Write, Remote]));
        // never the accounts
        let admin = role(&["read", "write", "remote", "delete", "users"]);
        assert_eq!(
            granted_scope(None, &admin),
            Ok(vec![Read, Write, Remote, Delete])
        );
        assert!(granted_scope(Some("users"), &admin).is_err());
        // a narrower one, in the order of the permissions, each once
        assert_eq!(
            granted_scope(Some("write read  read"), &editor),
            Ok(vec![Read, Write])
        );
        // more than the role gives, or a name that is no permission: an error
        assert!(granted_scope(Some("read delete"), &editor).is_err());
        assert!(granted_scope(Some("Read"), &editor).is_err());
        // an empty scope asks for nothing, and a role may give nothing
        assert_eq!(granted_scope(Some(""), &editor), Ok(vec![]));
        assert_eq!(granted_scope(None, &role(&["users"])), Ok(vec![]));
    }

    #[test]
    fn basic_credentials_and_secrets() {
        let header = |text: &str| HeaderValue::from_str(text).unwrap();
        // "abc:s3:cret": the secret may hold a colon, the id may not
        assert_eq!(
            basic_credentials(Some(&header("Basic YWJjOnMzOmNyZXQ="))),
            Some(("abc".into(), "s3:cret".into()))
        );
        assert_eq!(
            basic_credentials(Some(&header("basic YWJjOnMzOmNyZXQ="))),
            Some(("abc".into(), "s3:cret".into()))
        );
        for refused in [
            "Bearer YWJjOnMzOmNyZXQ=",
            "Basic",
            "Basic !!!",
            "Basic YWJj",
            "",
        ] {
            assert_eq!(basic_credentials(Some(&header(refused))), None, "{refused}");
        }
        assert_eq!(basic_credentials(None), None);

        let (one, other) = (new_secret().unwrap(), new_secret().unwrap());
        assert_eq!(one.len(), 64);
        assert_ne!(one, other);
        assert_eq!(
            secret_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd") && !same("abc", "ab") && !same("", "a"));
        assert!(client_name(" Viewer ").is_ok_and(|name| name == "Viewer"));
        assert!(client_name(" ").is_err() && client_name(&"x".repeat(61)).is_err());
    }
}
