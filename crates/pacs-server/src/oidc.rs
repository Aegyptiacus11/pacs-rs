//! Signing in through an OpenID provider, when `PACS_OIDC_ISSUER` is set: the authorization
//! code flow of OpenID Connect Core 1.0 (section 3.1), with PKCE (RFC 7636, `S256`).
//!
//! The provider is then the only way in for people: `POST /auth/login` is refused, and so is
//! every password. Whoever the provider lets through gets an account at their first sign-in,
//! with the role `PACS_OIDC_ROLE`; the addresses of `PACS_OIDC_ADMINS`, when the provider
//! says it verified them, get `admin`. After that, pacs-rs gives the same session as after a
//! password: an access token, and the refresh cookie that `/auth/refresh` rotates.
//!
//! ```text
//! GET /auth/oidc/start?next=<path in the web interface>
//!     -> 303 to the provider's authorization endpoint, with a state cookie (10 minutes)
//! GET /auth/oidc/callback?code=...&state=...
//!     -> 303 to the web interface, with the refresh cookie; or with ?signin_error=<reason>
//! GET /auth/methods
//!     -> {"password": bool, "provider": bool}: what the sign-in page offers
//! ```
//!
//! What the callback checks, in the order of Core 3.1.3.7 and RFC 7636: the state against the
//! cookie set at the start (CSRF, Core 3.1.2.1), the code exchanged with the PKCE verifier and
//! the client's secret, then the ID token: signed by a key the provider publishes (never by a
//! shared secret, never unsigned), its issuer the provider's, its audience this client (and
//! `azp` this client when there are several audiences), not expired, and its nonce the one
//! sent. A user is the provider's issuer and subject (Core 5.7: the only stable key); an email
//! links an existing account only when the provider says it verified it.
//!
//! The provider's settings are read at start from its discovery document (OpenID Connect
//! Discovery 1.0), whose issuer must be the one configured (Discovery 4.3). Its keys are
//! fetched again when a token names one not seen yet, at most once a minute, so a rotation at
//! the provider needs no restart here.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use axum::http::header::{CACHE_CONTROL, COOKIE, LOCATION, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

/// How long a sign-in may take at the provider: the life of the state cookie.
const STATE_SECONDS: u64 = 600;
/// The state cookie's name.
pub const STATE_COOKIE: &str = "pacs_oidc";
/// Keys are fetched again for an unknown key id at most this often.
const KEYS_REFETCH: Duration = Duration::from_secs(60);
/// Clock difference allowed with the provider on `exp`, `iat` and `nbf`.
const LEEWAY_SECONDS: u64 = 60;
/// How long the provider has to answer one request.
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(15);

/// The signatures accepted on the provider's tokens: public-key ones only. An HMAC (`HS*`)
/// would be keyed with the client's secret, which this server also holds, and `none` signs
/// nothing.
const ACCEPTED: [Algorithm; 9] = [
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::ES256,
    Algorithm::ES384,
    Algorithm::EdDSA,
];

/// What the settings say of the provider.
#[derive(Debug, Clone)]
pub struct ProviderSettings {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    /// This server's callback as the provider sends the browser to it:
    /// `https://<host>/auth/oidc/callback`, with the proxy's prefix if there is one.
    pub redirect_url: String,
    pub scopes: String,
    /// The role of an account made at its first sign-in.
    pub role: String,
    /// Lower case.
    pub admins: Vec<String>,
    /// What a tool's access token from the provider must name as its audience: this server
    /// (RFC 9068 4, RFC 8707).
    pub audience: String,
}

/// What the discovery document says, of what is used here.
#[derive(Debug, Clone, Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    introspection_endpoint: Option<String>,
}

struct Keys {
    set: JwkSet,
    fetched: Instant,
}

struct Inner {
    settings: ProviderSettings,
    discovery: Discovery,
    http: reqwest::Client,
    keys: RwLock<Keys>,
}

/// A provider, discovered and with its keys.
#[derive(Clone)]
pub struct Provider(Arc<Inner>);

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("issuer", &self.0.settings.issuer)
            .field("client_id", &self.0.settings.client_id)
            .finish_non_exhaustive()
    }
}

/// HTTPS, or plain HTTP to this machine only (a provider in a test or beside the server).
fn usable_url(what: &str, url: &str) -> anyhow::Result<url::Url> {
    let parsed = url::Url::parse(url).with_context(|| format!("{what}: {url:?} is not a URL"))?;
    let loopback = matches!(parsed.host(), Some(url::Host::Domain("localhost")))
        || matches!(parsed.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
        || matches!(parsed.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback());
    match parsed.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => bail!("{what}: {url} must be https (plain http only to localhost)"),
    }
    if parsed.fragment().is_some() {
        bail!("{what}: {url} must not have a fragment");
    }
    Ok(parsed)
}

impl Provider {
    /// Read the provider's discovery document and keys; fails when it cannot be reached or
    /// says another issuer, so that a wrong setting stops the start.
    pub async fn discover(settings: ProviderSettings) -> anyhow::Result<Self> {
        usable_url("PACS_OIDC_ISSUER", &settings.issuer)?;
        usable_url("PACS_OIDC_REDIRECT_URL", &settings.redirect_url)?;
        if !settings.scopes.split(' ').any(|scope| scope == "openid") {
            bail!("PACS_OIDC_SCOPES: must include openid");
        }
        let http = reqwest::Client::builder()
            .timeout(PROVIDER_TIMEOUT)
            // the provider's answers are read as they are, never followed elsewhere
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("the HTTP client for the provider")?;
        // Discovery 4.1: the issuer without a terminating slash, then the well-known path
        let well_known = format!(
            "{}/.well-known/openid-configuration",
            settings.issuer.trim_end_matches('/')
        );
        let discovery: Discovery = fetch_json(&http, &well_known)
            .await
            .with_context(|| format!("the provider's discovery document ({well_known})"))?;
        if discovery.issuer != settings.issuer {
            bail!(
                "PACS_OIDC_ISSUER is {:?} but the provider names itself {:?}: set it to the \
                 provider's issuer exactly",
                settings.issuer,
                discovery.issuer
            );
        }
        for (what, url) in [
            ("authorization_endpoint", &discovery.authorization_endpoint),
            ("token_endpoint", &discovery.token_endpoint),
            ("jwks_uri", &discovery.jwks_uri),
        ] {
            usable_url(what, url)?;
        }
        if let Some(url) = &discovery.introspection_endpoint {
            usable_url("introspection_endpoint", url)?;
        }
        let set: JwkSet = fetch_json(&http, &discovery.jwks_uri)
            .await
            .with_context(|| format!("the provider's keys ({})", discovery.jwks_uri))?;
        Ok(Self(Arc::new(Inner {
            settings,
            discovery,
            http,
            keys: RwLock::new(Keys {
                set,
                fetched: Instant::now(),
            }),
        })))
    }

    pub fn issuer(&self) -> &str {
        &self.0.settings.issuer
    }

    pub fn client_id(&self) -> &str {
        &self.0.settings.client_id
    }

    pub fn role(&self) -> &str {
        &self.0.settings.role
    }

    pub fn is_admin(&self, email: &str) -> bool {
        self.0.settings.admins.iter().any(|admin| admin == email)
    }

    pub fn introspection_endpoint(&self) -> Option<&str> {
        self.0.discovery.introspection_endpoint.as_deref()
    }

    pub fn audience(&self) -> &str {
        &self.0.settings.audience
    }

    /// The claims of a tool's access token: a JWT checked with the provider's keys (RFC
    /// 9068), anything else asked of its introspection endpoint (RFC 7662, IUA ITI-102).
    /// Either way the provider issued it, for this server, and it has not expired.
    pub async fn access_claims(&self, token: &str) -> Result<Map<String, Value>, Failure> {
        let bad = |why: &str| Failure::Token(why.to_owned());
        if jsonwebtoken::decode_header(token).is_ok() {
            let claims = self.verified(token, &self.0.settings.audience).await?;
            // an ID token says who signed in, to a client: it is not an access token
            if claims.contains_key("nonce") {
                return Err(bad("an ID token, not an access token"));
            }
            return Ok(claims);
        }
        let Some(endpoint) = &self.0.discovery.introspection_endpoint else {
            return Err(bad(
                "not a JWT, and the provider has no introspection endpoint",
            ));
        };
        let response = self
            .0
            .http
            .post(endpoint)
            .header(axum::http::header::AUTHORIZATION, self.client_basic())
            .form(&[("token", token), ("token_type_hint", "access_token")])
            .send()
            .await
            .map_err(|e| Failure::Provider(format!("the introspection endpoint: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(Failure::Provider(format!(
                "the introspection endpoint answered {status}"
            )));
        }
        let claims: Map<String, Value> = response
            .json()
            .await
            .map_err(|e| Failure::Provider(format!("the introspection endpoint's answer: {e}")))?;
        // RFC 7662 2.2: only "active": true says anything; every other claim is optional
        if claims.get("active") != Some(&Value::Bool(true)) {
            return Err(bad("not active"));
        }
        if let Some(issuer) = claims.get("iss")
            && issuer.as_str() != Some(self.0.settings.issuer.as_str())
        {
            return Err(bad("issued by another provider"));
        }
        let audience = self.0.settings.audience.as_str();
        let for_us = match claims.get("aud") {
            Some(Value::String(aud)) => aud == audience,
            Some(Value::Array(aud)) => aud.iter().any(|a| a.as_str() == Some(audience)),
            _ => false,
        };
        if !for_us {
            return Err(bad("issued for another audience"));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| Failure::Internal(e.to_string()))?
            .as_secs();
        if claims
            .get("exp")
            .and_then(Value::as_u64)
            .is_some_and(|exp| exp + LEEWAY_SECONDS < now)
        {
            return Err(bad("expired"));
        }
        Ok(claims)
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.0.http
    }

    /// The URL the browser is sent to, for a sign-in with these values.
    pub fn authorization_url(&self, state: &str, nonce: &str, challenge: &str) -> String {
        let settings = &self.0.settings;
        let mut url = url::Url::parse(&self.0.discovery.authorization_endpoint)
            .expect("checked at discovery");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &settings.client_id)
            .append_pair("redirect_uri", &settings.redirect_url)
            .append_pair("scope", &settings.scopes)
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
        url.into()
    }

    /// The client's own credentials, as RFC 6749 2.3.1 says to send them: each part
    /// form-encoded, then HTTP Basic.
    pub fn client_basic(&self) -> String {
        let encode =
            |part: &str| url::form_urlencoded::byte_serialize(part.as_bytes()).collect::<String>();
        let settings = &self.0.settings;
        let pair = format!(
            "{}:{}",
            encode(&settings.client_id),
            encode(&settings.client_secret)
        );
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(pair)
        )
    }

    /// Exchange the code for the provider's tokens (Core 3.1.3.1); the ID token.
    pub async fn exchange(&self, code: &str, verifier: &str) -> Result<String, Failure> {
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", self.0.settings.redirect_url.as_str()),
            ("code_verifier", verifier),
        ];
        let response = self
            .0
            .http
            .post(&self.0.discovery.token_endpoint)
            .header(axum::http::header::AUTHORIZATION, self.client_basic())
            .form(&form)
            .send()
            .await
            .map_err(|e| Failure::Provider(format!("the token endpoint: {e}")))?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|e| Failure::Provider(format!("the token endpoint's answer: {e}")))?;
        if !status.is_success() {
            let error = body["error"].as_str().unwrap_or("unknown");
            return Err(Failure::Refused(format!(
                "the provider refused the code ({status}, {error})"
            )));
        }
        body["id_token"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Failure::Provider("the token endpoint gave no ID token".into()))
    }

    /// The key that signed a token with this header: from the published set, fetched again
    /// once when the key id is new to it.
    async fn key_for(&self, header: &jsonwebtoken::Header) -> Result<DecodingKey, Failure> {
        let bad = |why: &str| Failure::Token(why.to_owned());
        if !ACCEPTED.contains(&header.alg) {
            return Err(bad("signed with an algorithm that is not accepted"));
        }
        let pick = |set: &JwkSet| -> Option<Jwk> {
            match &header.kid {
                Some(kid) => set.find(kid).cloned(),
                // no key id: only a set of one key leaves no doubt
                None if set.keys.len() == 1 => set.keys.first().cloned(),
                None => None,
            }
        };
        let found = pick(&self.0.keys.read().await.set);
        let jwk = match found {
            Some(jwk) => jwk,
            None => {
                let mut keys = self.0.keys.write().await;
                if keys.fetched.elapsed() >= KEYS_REFETCH {
                    let set: JwkSet = fetch_json(&self.0.http, &self.0.discovery.jwks_uri)
                        .await
                        .map_err(|e| Failure::Provider(format!("the provider's keys: {e:#}")))?;
                    *keys = Keys {
                        set,
                        fetched: Instant::now(),
                    };
                }
                pick(&keys.set)
                    .ok_or_else(|| bad("signed by a key the provider does not publish"))?
            }
        };
        // a key for one algorithm is not used with another (RFC 7517 4.4)
        if let Some(alg) = jwk.common.key_algorithm
            && format!("{alg:?}") != format!("{:?}", header.alg)
        {
            return Err(bad("signed with another algorithm than its key's"));
        }
        DecodingKey::from_jwk(&jwk).map_err(|_| bad("signed by a key that cannot be read"))
    }

    /// The claims of a token this provider signed for `audience`, checked as Core 3.1.3.7
    /// says (and RFC 9068 4 for an access token): signature, issuer, audience, expiry.
    pub async fn verified(
        &self,
        token: &str,
        audience: &str,
    ) -> Result<Map<String, Value>, Failure> {
        let header = jsonwebtoken::decode_header(token)
            .map_err(|_| Failure::Token("not a signed JWT".into()))?;
        let key = self.key_for(&header).await?;
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[&self.0.settings.issuer]);
        validation.set_audience(&[audience]);
        // `sub` is required of an ID token by `identity`; a tool's access token is known by its
        // client, and Rauthy's (client credentials) has no `sub`
        validation.set_required_spec_claims(&["iss", "aud", "exp"]);
        validation.leeway = LEEWAY_SECONDS;
        validation.validate_nbf = true;
        let data = jsonwebtoken::decode::<Map<String, Value>>(token, &key, &validation)
            .map_err(|e| Failure::Token(format!("refused: {e}")))?;
        Ok(data.claims)
    }

    /// Who an ID token names, once it is checked against the nonce sent (Core 3.1.3.7).
    pub async fn identity(&self, id_token: &str, nonce: &str) -> Result<Identity, Failure> {
        let client = self.0.settings.client_id.as_str();
        let claims = self.verified(id_token, client).await?;
        let bad = |why: &str| Failure::Token(why.to_owned());
        // several audiences: the one it was issued to must be named, and be this client
        let several = claims["aud"].as_array().is_some_and(|aud| aud.len() > 1);
        match claims.get("azp").and_then(Value::as_str) {
            Some(azp) if azp != client => return Err(bad("issued to another client (azp)")),
            None if several => return Err(bad("several audiences and no azp")),
            _ => {}
        }
        if claims.get("iat").and_then(Value::as_u64).is_none() {
            return Err(bad("no iat"));
        }
        if claims.get("nonce").and_then(Value::as_str) != Some(nonce) {
            return Err(bad("not the nonce of this sign-in"));
        }
        let text = |name: &str| {
            claims
                .get(name)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let subject = text("sub").ok_or_else(|| bad("no subject"))?;
        if subject.len() > 255 {
            // Core 2: at most 255 ASCII characters
            return Err(bad("a subject over 255 characters"));
        }
        // Core 5.1: email_verified is a boolean; anything else is not a verification
        let verified = claims.get("email_verified") == Some(&Value::Bool(true));
        Ok(Identity {
            subject,
            email: text("email").map(|e| e.to_lowercase()),
            email_verified: verified,
            username: text("preferred_username").map(|u| u.to_lowercase()),
        })
    }
}

/// Who signed in, as the ID token says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub subject: String,
    /// Lower case.
    pub email: Option<String>,
    pub email_verified: bool,
    /// Lower case.
    pub username: Option<String>,
}

impl Identity {
    /// The login of an account made for them, and whether the provider vouches for it: a
    /// verified email, else their user name, else the subject.
    pub fn login(&self) -> (String, bool) {
        match (&self.email, self.email_verified) {
            (Some(email), true) => (email.clone(), true),
            _ => (
                self.username
                    .clone()
                    .unwrap_or_else(|| self.subject.to_lowercase()),
                false,
            ),
        }
    }
}

/// Why a sign-in through the provider failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The browser came back without the sign-in this server started, or after it expired.
    State,
    /// The provider said no: an error at sign-in, or a code it would not exchange.
    Refused(String),
    /// The provider could not be reached or answered something unreadable.
    Provider(String),
    /// A token that does not pass the checks.
    Token(String),
    /// No account can be given: a login taken by an account it may not be linked to, a
    /// deactivated account, or the role to give gone.
    Account(String),
    /// Something of this server's.
    Internal(String),
}

impl Failure {
    /// The reason the web interface is told (`?signin_error=`).
    pub fn reason(&self) -> &'static str {
        match self {
            Self::State => "expired",
            Self::Refused(_) => "refused",
            Self::Provider(_) => "provider",
            Self::Token(_) => "token",
            Self::Account(_) => "account",
            Self::Internal(_) => "internal",
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            Self::State => "no sign-in of this browser is under way",
            Self::Refused(d)
            | Self::Provider(d)
            | Self::Token(d)
            | Self::Account(d)
            | Self::Internal(d) => d,
        }
    }
}

async fn fetch_json<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    url: &str,
) -> anyhow::Result<T> {
    let response = http.get(url).send().await.context("no answer")?;
    let status = response.status();
    if !status.is_success() {
        bail!("HTTP {status}");
    }
    response.json().await.context("not the JSON expected")
}

/// `n` random bytes, URL-safe base64 without padding.
pub fn random(n: usize) -> anyhow::Result<String> {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no random bytes: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The PKCE challenge of a verifier (RFC 7636 4.2, `S256`).
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Where in the web interface to land after signing in: a path under it, relative, without a
/// scheme, host or `..` that would take the browser elsewhere; else its home.
pub fn landing(next: Option<&str>) -> String {
    let Some(next) = next.map(str::trim) else {
        return String::new();
    };
    let next = next.strip_prefix("/ui/").unwrap_or(next);
    let safe = next.len() <= 512
        && !next.starts_with('/')
        && !next.contains("..")
        && !next.contains("://")
        && !next.contains('\\')
        && next.bytes().all(|b| b.is_ascii_graphic());
    if safe { next.to_owned() } else { String::new() }
}

/// The value of cookie `name` in a request.
pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// What the callback is asked.
#[derive(Debug, Deserialize)]
pub struct Callback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// What `start` is asked.
#[derive(Debug, Deserialize)]
pub struct Start {
    pub next: Option<String>,
}

/// A redirect the browser follows, never stored.
pub fn see_other(location: &str, cookies: &[String]) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    match HeaderValue::from_str(location) {
        Ok(value) => {
            headers.insert(LOCATION, value);
        }
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    for cookie in cookies {
        match HeaderValue::from_str(cookie) {
            Ok(value) => {
                headers.append(SET_COOKIE, value);
            }
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
    response
}

/// The life of the state cookie, for the login to sign it with.
pub const STATE_TTL: Duration = Duration::from_secs(STATE_SECONDS);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_is_rfc_7636s_example() {
        // RFC 7636 appendix B
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn only_a_path_inside_the_web_interface_is_a_landing() {
        assert_eq!(landing(None), "");
        assert_eq!(landing(Some("studies/1.2.3")), "studies/1.2.3");
        assert_eq!(landing(Some("/ui/studies?x=1")), "studies?x=1");
        for elsewhere in [
            "//evil.example",
            "/elsewhere",
            "https://evil.example",
            "../admin",
            "a\\b",
            "a b",
        ] {
            assert_eq!(landing(Some(elsewhere)), "", "{elsewhere}");
        }
    }

    #[test]
    fn a_provider_url_is_https_or_plain_http_to_this_machine() {
        assert!(usable_url("x", "https://idp.example/realms/a").is_ok());
        assert!(usable_url("x", "http://localhost:8080/auth/v1").is_ok());
        assert!(usable_url("x", "http://127.0.0.1:8080").is_ok());
        assert!(usable_url("x", "http://[::1]:8080").is_ok());
        assert!(usable_url("x", "http://idp.example").is_err());
        assert!(usable_url("x", "https://idp.example#x").is_err());
        assert!(usable_url("x", "ftp://idp.example").is_err());
        assert!(usable_url("x", "not a url").is_err());
    }

    #[test]
    fn the_login_of_an_identity_is_vouched_for_only_by_a_verified_email() {
        let who = |email: Option<&str>, verified, username: Option<&str>| Identity {
            subject: "Sub-1".into(),
            email: email.map(String::from),
            email_verified: verified,
            username: username.map(String::from),
        };
        assert_eq!(
            who(Some("ada@example.com"), true, Some("ada")).login(),
            ("ada@example.com".into(), true)
        );
        assert_eq!(
            who(Some("ada@example.com"), false, Some("ada")).login(),
            ("ada".into(), false)
        );
        assert_eq!(who(None, false, None).login(), ("sub-1".into(), false));
    }

    #[test]
    fn the_client_credentials_are_form_encoded_then_basic() {
        // RFC 6749 2.3.1: a ':' in the secret must not split it
        let encode =
            |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
        assert_eq!(encode("a:b c"), "a%3Ab+c");
    }

    #[test]
    fn a_cookie_is_found_by_its_name_among_others() {
        let mut headers = HeaderMap::new();
        headers.append(COOKIE, HeaderValue::from_static("a=1; pacs_oidc=xyz"));
        headers.append(COOKIE, HeaderValue::from_static("b=2"));
        assert_eq!(cookie(&headers, STATE_COOKIE), Some("xyz"));
        assert_eq!(cookie(&headers, "b"), Some("2"));
        assert_eq!(cookie(&headers, "pacs"), None);
    }
}
