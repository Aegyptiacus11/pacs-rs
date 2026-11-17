//! The answers of the login's routes as types, for the API's description. As in
//! `pacs_rest::schema`: they refuse a field they do not name, and `tests/auth.rs` reads the
//! routes' real answers into them.

use serde::Deserialize;
use utoipa::ToSchema;

/// A request refused by the login: why.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Refused {
    pub detail: String,
}

/// A login and its password.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    /// The login. The field keeps the name the sessions library gives it.
    pub email: String,
    pub password: String,
}

/// A session begun or carried on: the access token to send as `Authorization: Bearer`. The
/// refresh token comes beside it, in an `HttpOnly` cookie.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Session {
    /// A JWT, good for 15 minutes.
    pub access_token: String,
    /// `bearer`.
    pub token_type: String,
}

/// Who a token is.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Me {
    pub id: String,
    /// The user's login, or a tool's name.
    pub login: String,
    /// The id of their role.
    pub role: String,
    /// What the role allows: of `read`, `write`, `remote`, `delete`, `users`.
    pub permissions: Vec<String>,
    /// The projects the account is limited to; `null` when it sees every study.
    #[schema(required)]
    pub projects: Option<Vec<String>>,
}

/// A link to make for a browser to follow.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DownloadRequest {
    /// A path of this server, with its query if any: `/studies/{id}/archive`.
    pub path: String,
}

/// A token good for one `GET` of one path, to put in its URL as `?token=`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DownloadToken {
    pub token: String,
    /// Seconds it is good for.
    pub expires_in: u64,
}

/// A user.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct User {
    pub id: String,
    /// What they sign in with, in lower case.
    pub login: String,
    /// The id of their role.
    pub role: String,
    /// `false` for an account that may no longer sign in.
    pub active: bool,
    /// When the account was made, in milliseconds since 1970.
    pub created_ms: i64,
    /// The projects the user is limited to; `null` when they see every study.
    #[schema(required)]
    pub projects: Option<Vec<String>>,
}

/// A role: a name for a set of permissions.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Role {
    /// Made from the name: its letters and digits in lower case.
    pub id: String,
    pub name: String,
    /// Of `read`, `write`, `remote`, `delete`, `users`.
    pub permissions: Vec<String>,
    /// `true` for the one role that cannot be changed or removed, `admin`.
    pub locked: bool,
}

/// A tool: a program that connects without a person.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    /// What it presents, with its secret, to get a token.
    pub id: String,
    pub name: String,
    /// The id of its role.
    pub role: String,
    /// The login of the user who made it.
    pub created_by: String,
    pub created_ms: i64,
    /// When it last got a token; `null` when it never has.
    #[schema(required)]
    pub last_token_ms: Option<i64>,
    /// The projects the tool is limited to; `null` when it sees every study.
    #[schema(required)]
    pub projects: Option<Vec<String>>,
    /// Its secret: in the answer that makes the tool, and never again.
    pub secret: Option<String>,
}

/// An access token for a tool (OAuth 2.1, client credentials).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Token {
    /// A JWT, to send as `Authorization: Bearer`.
    pub access_token: String,
    /// `Bearer`.
    pub token_type: String,
    /// The permissions it carries, separated by spaces.
    pub scope: String,
    /// Seconds it is good for.
    pub expires_in: u64,
}

/// A token request refused, as OAuth 2.1 words it.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenRefused {
    /// `invalid_client`, `invalid_request`, `unsupported_grant_type`, `invalid_scope` or
    /// `invalid_target`.
    pub error: String,
    pub error_description: String,
}

/// The form of a token request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TokenRequest {
    /// `client_credentials`.
    pub grant_type: String,
    /// Permissions separated by spaces, to get fewer than the role gives.
    pub scope: Option<String>,
    /// This server's name (`PACS_AUTH_ISSUER`); another one is refused.
    pub resource: Option<String>,
    /// `urn:ietf:params:oauth:token-type:jwt`, the only kind.
    pub requested_token_type: Option<String>,
}
