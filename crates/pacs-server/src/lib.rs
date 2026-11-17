//! The pacs-rs server: settings, assembly of the HTTP application, and serving it.
//!
//! Settings come from flags or `PACS_*` environment variables. Secrets (the S3 keys) come
//! only from the environment, or from a file named by the variable with `_FILE` appended
//! (Docker and Portainer secrets), never from flags, which other users can read in `ps`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use axum::Router;
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::routing::get;
use clap::Parser;
use pacs_archive::{Archive, Compression};
use pacs_dimse::{AeTitle, RemoteAe};
use pacs_index::Index;
use pacs_store::{S3Config, Store};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

mod annotations;
pub mod api;
pub mod audit;
pub mod audit_forward;
mod audit_routes;
pub mod auth;
mod dimse_tls;
pub mod limits;
mod modalities;
pub mod projects;
mod protocols;
mod scope;
mod ui;
mod worklist;

#[derive(Debug, Clone, Parser)]
#[command(name = "pacs-server", version, about = "pacs-rs: a PACS in Rust")]
pub struct Settings {
    /// Address to listen on.
    #[arg(long, env = "PACS_LISTEN", default_value = "127.0.0.1:8080")]
    pub listen: SocketAddr,

    /// Public URL of the DICOMweb root, for Retrieve URLs. Without it they are built from
    /// each request's Host and X-Forwarded-* headers.
    #[arg(long, env = "PACS_BASE_URL")]
    pub base_url: Option<String>,

    /// Path the DICOMweb root is served under.
    #[arg(long, env = "PACS_DICOMWEB_PATH", default_value = "/dicom-web")]
    pub dicomweb_path: String,

    /// The index: a SQLite file (created if missing), or `postgres://user@host/database`
    /// with the password in PACS_INDEX_PASSWORD (or PACS_INDEX_PASSWORD_FILE).
    #[arg(long, env = "PACS_INDEX", default_value = "data/index.sqlite")]
    pub index: String,

    /// TLS to a Postgres index: `prefer` (encrypted when the server offers it, its
    /// certificate not checked), `verify-full` (encrypted, and the certificate must be
    /// good for the host in PACS_INDEX), or `off`.
    #[arg(long, env = "PACS_INDEX_TLS", value_enum, default_value = "prefer")]
    pub index_tls: IndexTls,

    /// The certificate authority (PEM) that signed the Postgres server's certificate, for
    /// `verify-full`. Without it, the web's root certificates.
    #[arg(long, env = "PACS_INDEX_CA_FILE")]
    pub index_ca_file: Option<PathBuf>,

    /// Where files go: `file:<directory>` or `s3://<bucket>[/<prefix>]`.
    #[arg(long, env = "PACS_STORAGE", default_value = "file:data/files")]
    pub storage: String,

    /// S3 endpoint for a non-AWS store, e.g. `http://localhost:8333`.
    #[arg(long, env = "PACS_S3_ENDPOINT")]
    pub s3_endpoint: Option<String>,

    #[arg(long, env = "PACS_S3_REGION", default_value = "us-east-1")]
    pub s3_region: String,

    /// Replace an instance that is sent again, instead of keeping the first copy.
    #[arg(long, env = "PACS_OVERWRITE_INSTANCES", default_value_t = false)]
    pub overwrite_instances: bool,

    /// How uncompressed pixel data is stored: `jpeg-ls` (lossless, each frame checked to
    /// decode back exactly) or `off` (as received). Compressed instances are always kept
    /// as received.
    #[arg(long, env = "PACS_COMPRESSION", value_enum, default_value_t = StoredCompression::JpegLs)]
    pub compression: StoredCompression,

    /// Largest accepted request body, in bytes (STOW-RS, uploads).
    #[arg(long, env = "PACS_MAX_UPLOAD_BYTES", default_value_t = 1 << 30)]
    pub max_upload_bytes: usize,

    /// Memory that reading files may take at once beyond the files themselves, in bytes, by
    /// estimate; a file that would take more alone is refused (see docs/dicomweb.md).
    #[arg(long, env = "PACS_MAX_READ_BYTES", default_value_t = pacs_archive::DEFAULT_READ_LIMIT)]
    pub max_read_bytes: u64,

    /// Check that a server with these settings answers on /healthz, exit 0 if it does and
    /// 1 if not, and do nothing else: a health check for images without a shell or curl.
    #[arg(long)]
    pub healthcheck: bool,

    /// Origins browsers may call from, comma-separated, or `*`. None by default.
    #[arg(long, env = "PACS_CORS_ORIGINS", value_delimiter = ',')]
    pub cors_origins: Vec<String>,

    /// Serve the web interface at /ui/ (`/` redirects there). Without PACS_AUTH whoever
    /// reaches it can delete, as with the REST API.
    #[arg(long, env = "PACS_UI", default_value_t = true, action = clap::ArgAction::Set)]
    pub ui: bool,

    /// Ask for a login on every HTTP route but /healthz and the web interface's files (see
    /// `pacs_server::auth`). Needs PACS_AUTH_SECRET (or PACS_AUTH_SECRET_FILE), the signing
    /// secret of at least 32 bytes, and for the first start PACS_ADMIN_PASSWORD (or
    /// PACS_ADMIN_PASSWORD_FILE). Off by default: anyone who reaches the server can then read
    /// and delete everything.
    #[arg(long, env = "PACS_AUTH", default_value_t = false, action = clap::ArgAction::Set)]
    pub auth: bool,

    /// The login of the first administrator, made when PACS_AUTH is on and there is no user.
    #[arg(long, env = "PACS_ADMIN_LOGIN", default_value = "admin")]
    pub admin_login: String,

    /// Send the refresh cookie over HTTPS only. Turn off only to try the login over plain
    /// HTTP on localhost.
    #[arg(long, env = "PACS_AUTH_COOKIE_SECURE", default_value_t = true, action = clap::ArgAction::Set)]
    pub auth_cookie_secure: bool,

    /// The path of the refresh cookie as the browser sees it, when a proxy serves pacs-rs
    /// under a prefix, e.g. `/pacs/auth`. By default `/auth`.
    #[arg(long, env = "PACS_AUTH_COOKIE_PATH")]
    pub auth_cookie_path: Option<String>,

    /// The name this server gives itself in the access tokens of tools: their issuer (`iss`)
    /// and audience (`aud`), and what a tool may name as `resource` when it asks for one.
    #[arg(long, env = "PACS_AUTH_ISSUER", default_value = "pacs-rs")]
    pub auth_issuer: String,

    /// Seconds a client has to send a request's head (see `pacs_server::limits`); 0: no
    /// limit.
    #[arg(long, env = "PACS_HEADER_TIMEOUT", default_value_t = 30)]
    pub header_timeout: u64,

    /// Seconds a request's body may pause before the request fails; 0: no limit.
    #[arg(long, env = "PACS_BODY_TIMEOUT", default_value_t = 60)]
    pub body_timeout: u64,

    /// Seconds within which a request without a body (GET, HEAD, DELETE, OPTIONS) must be
    /// answered, or it gets 503; 0: no limit.
    #[arg(long, env = "PACS_REQUEST_TIMEOUT", default_value_t = 300)]
    pub request_timeout: u64,

    /// Seconds a write to a client may stall before its connection is closed; 0: no limit.
    #[arg(long, env = "PACS_WRITE_TIMEOUT", default_value_t = 60)]
    pub write_timeout: u64,

    /// Connections open at once; further ones wait to be accepted. 0: no bound.
    #[arg(long, env = "PACS_MAX_CONNECTIONS", default_value_t = 1024)]
    pub max_connections: usize,

    /// Requests served at once; one more gets 503. 0: no bound.
    #[arg(long, env = "PACS_MAX_REQUESTS", default_value_t = 256)]
    pub max_requests: usize,

    /// Requests served at once for one address; one more gets 429. Behind a reverse proxy
    /// every request has the proxy's address: raise it, or 0 for no bound.
    #[arg(long, env = "PACS_MAX_REQUESTS_PER_CLIENT", default_value_t = 64)]
    pub max_requests_per_client: usize,

    /// Keep an audit trail in this file (see `pacs_server::audit`): one line per event,
    /// appended, in the format a hospital's audit repository reads. None by default: no
    /// trail. A file that cannot be opened stops the start.
    #[arg(long, env = "PACS_AUDIT_FILE")]
    pub audit_file: Option<PathBuf>,

    /// The name this server reports under in its audit messages (`AuditSourceID`).
    #[arg(long, env = "PACS_AUDIT_SOURCE_ID", default_value = "pacs-rs")]
    pub audit_source_id: String,

    /// Send the audit trail to this audit record repository as well, over TLS: `host` or
    /// `host:port` (6514 unless given). Needs PACS_AUDIT_FILE, which is what is sent.
    #[arg(long, env = "PACS_AUDIT_REPOSITORY")]
    pub audit_repository: Option<String>,

    /// The certificate authorities (PEM) that sign the repository's certificate.
    #[arg(long, env = "PACS_AUDIT_REPOSITORY_CA_FILE")]
    pub audit_repository_ca_file: Option<PathBuf>,

    /// The certificate pacs-rs shows the repository, with its chain (PEM), and
    #[arg(long, env = "PACS_AUDIT_REPOSITORY_CERT_FILE")]
    pub audit_repository_cert_file: Option<PathBuf>,

    /// its private key (PEM).
    #[arg(long, env = "PACS_AUDIT_REPOSITORY_KEY_FILE")]
    pub audit_repository_key_file: Option<PathBuf>,

    /// Address DIMSE (C-ECHO, C-STORE) listens on, e.g. `0.0.0.0:11112`. None by default:
    /// no DIMSE port is opened.
    #[arg(long, env = "PACS_DIMSE_LISTEN")]
    pub dimse_listen: Option<SocketAddr>,

    /// pacs-rs's AE title: what remote AEs call it, and what it calls them as.
    #[arg(long, env = "PACS_DIMSE_AE_TITLE", default_value = "PACSRS")]
    pub dimse_ae_title: AeTitle,

    /// Further AE titles accepted on the same port, comma-separated.
    #[arg(long, env = "PACS_DIMSE_EXTRA_AE_TITLES", value_delimiter = ',')]
    pub dimse_extra_ae_titles: Vec<AeTitle>,

    /// The AEs pacs-rs can call, comma-separated, each `id=AE_TITLE@host:port`; the id names
    /// it in the REST API (`/modalities/{id}`).
    #[arg(long, env = "PACS_DIMSE_REMOTES", value_delimiter = ',')]
    pub dimse_remotes: Vec<RemoteAe>,

    /// Accept associations only from the AE titles of PACS_DIMSE_REMOTES.
    #[arg(long, env = "PACS_DIMSE_KNOWN_CALLERS_ONLY", default_value_t = false)]
    pub dimse_known_callers_only: bool,

    /// Associations served at once; further ones are rejected as busy, to be retried.
    #[arg(long, env = "PACS_DIMSE_MAX_ASSOCIATIONS", default_value_t = 16)]
    pub dimse_max_associations: usize,

    /// Seconds to wait for a remote AE: to connect, for its next message, for an answer.
    #[arg(long, env = "PACS_DIMSE_TIMEOUT", default_value_t = 30)]
    pub dimse_timeout: u64,

    /// A JSON file listing projects: each an AE title whose instances are de-identified on
    /// the way in by its profile (see `pacs_server::projects`). None by default: everything is
    /// stored as received.
    #[arg(long, env = "PACS_PROJECTS")]
    pub projects: Option<PathBuf>,

    /// Where instances a project quarantines are kept, identified, never indexed or served:
    /// `file:<directory>` or `s3://<bucket>[/<prefix>]`. Without it they are refused.
    #[arg(long, env = "PACS_QUARANTINE")]
    pub quarantine: Option<String>,

    /// Seconds to wait between two C-MOVE responses: a remote AE may send none while it sends
    /// a large study to pacs-rs.
    #[arg(long, env = "PACS_DIMSE_MOVE_TIMEOUT", default_value_t = 600)]
    pub dimse_move_timeout: u64,

    /// pacs-rs's certificate for DIMSE over TLS, with its chain (PEM). With it and its key
    /// the DIMSE port speaks TLS only, and pacs-rs shows this certificate to the remote AEs
    /// it calls over TLS.
    #[arg(long, env = "PACS_DIMSE_TLS_CERT_FILE")]
    pub dimse_tls_cert_file: Option<PathBuf>,

    /// The private key of that certificate (PEM).
    #[arg(long, env = "PACS_DIMSE_TLS_KEY_FILE")]
    pub dimse_tls_key_file: Option<PathBuf>,

    /// The certificate authorities (PEM) pacs-rs accepts its DIMSE peers by: the callers'
    /// certificates, and those of the remote AEs it calls over TLS.
    #[arg(long, env = "PACS_DIMSE_TLS_CA_FILE")]
    pub dimse_tls_ca_file: Option<PathBuf>,

    /// Whether a caller must show a certificate signed by one of those authorities:
    /// `required`, or `off` to take any caller that speaks TLS.
    #[arg(
        long,
        env = "PACS_DIMSE_TLS_CLIENT_AUTH",
        value_enum,
        default_value = "required"
    )]
    pub dimse_tls_client_auth: ClientAuth,
}

/// Whether the DIMSE listener asks its callers for a certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ClientAuth {
    /// A caller without a certificate signed by a known authority is refused.
    Required,
    /// Any caller that speaks TLS is taken: the connection is encrypted, the caller is
    /// not identified.
    Off,
}

/// How the connection to a Postgres index is protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum IndexTls {
    /// Never encrypted.
    Off,
    /// Encrypted when the server offers it; who the server is, is not checked.
    Prefer,
    /// Encrypted, and the server's certificate must chain to a known authority and name
    /// the host connected to. Refused otherwise.
    VerifyFull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum StoredCompression {
    JpegLs,
    Off,
}

impl From<StoredCompression> for Compression {
    fn from(c: StoredCompression) -> Self {
        match c {
            StoredCompression::JpegLs => Compression::JpegLs,
            StoredCompression::Off => Compression::Off,
        }
    }
}

/// Where the files are kept, parsed from `--storage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageLocation {
    Directory(PathBuf),
    S3 { bucket: String, prefix: String },
}

impl StorageLocation {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        if let Some(path) = text.strip_prefix("file:") {
            if path.is_empty() {
                bail!("file: storage needs a directory, e.g. file:data/files");
            }
            return Ok(StorageLocation::Directory(PathBuf::from(path)));
        }
        if let Some(rest) = text.strip_prefix("s3://") {
            let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
            if bucket.is_empty() {
                bail!("s3 storage needs a bucket, e.g. s3://dicom");
            }
            return Ok(StorageLocation::S3 {
                bucket: bucket.to_owned(),
                prefix: prefix.trim_matches('/').to_owned(),
            });
        }
        bail!("storage must be file:<directory> or s3://<bucket>[/<prefix>], not {text:?}")
    }
}

/// A secret from `NAME`, or from the file `NAME_FILE` points to (trailing newline dropped).
pub fn secret(name: &str, env: impl Fn(&str) -> Option<String>) -> anyhow::Result<Option<String>> {
    if let Some(value) = env(name) {
        return Ok(Some(value));
    }
    let file_var = format!("{name}_FILE");
    match env(&file_var) {
        Some(path) => {
            let value = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {file_var} ({path})"))?;
            Ok(Some(value.trim_end_matches(['\n', '\r']).to_owned()))
        }
        None => Ok(None),
    }
}

pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The store `location` names (`file:` or `s3://`), with the S3 settings and keys.
pub fn open_store(location: &str, settings: &Settings) -> anyhow::Result<Store> {
    Ok(match StorageLocation::parse(location)? {
        StorageLocation::Directory(dir) => {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("creating the storage directory {}", dir.display()))?;
            Store::local(&dir, "")?
        }
        StorageLocation::S3 { bucket, prefix } => {
            let key = |name| {
                secret(name, process_env)?
                    .with_context(|| format!("{name} (or {name}_FILE) is needed for S3 storage"))
            };
            let config = S3Config {
                endpoint: settings.s3_endpoint.clone(),
                region: settings.s3_region.clone(),
                bucket,
                access_key_id: key("PACS_S3_ACCESS_KEY_ID")?,
                secret_access_key: key("PACS_S3_SECRET_ACCESS_KEY")?,
            };
            Store::s3(&config, &prefix)?
        }
    })
}

/// Open the store and the index the settings name.
pub async fn open_archive(settings: &Settings) -> anyhow::Result<Archive> {
    let store = open_store(&settings.storage, settings)?;
    let index = match IndexLocation::parse(&settings.index) {
        IndexLocation::Postgres(url) => {
            let url = match secret("PACS_INDEX_PASSWORD", process_env)? {
                Some(password) => with_password(&url, &password)?,
                None => url,
            };
            let url = with_tls(&url, settings.index_tls, settings.index_ca_file.as_deref())?;
            Index::connect(&url)
                .await
                .with_context(|| format!("opening the Postgres index {}", without_password(&url)))?
        }
        IndexLocation::Sqlite(path) => {
            if settings.index_tls == IndexTls::VerifyFull || settings.index_ca_file.is_some() {
                bail!(
                    "PACS_INDEX_TLS=verify-full and PACS_INDEX_CA_FILE are for a Postgres index; \
                     PACS_INDEX names a SQLite file"
                );
            }
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("creating the index directory {}", parent.display())
                })?;
            }
            Index::open(&path)
                .await
                .with_context(|| format!("opening the index {}", path.display()))?
        }
    };
    Ok(Archive::new(store, index)
        .with_overwrite(settings.overwrite_instances)
        .with_compression(settings.compression.into())
        .with_read_limit(settings.max_read_bytes))
}

/// The DIMSE side's configuration, from the settings. A remote AE named twice is an error.
pub fn dimse_config(settings: &Settings) -> anyhow::Result<pacs_dimse::Config> {
    let mut config = pacs_dimse::Config::new(settings.dimse_ae_title.clone());
    config.extra_ae_titles = settings.dimse_extra_ae_titles.clone();
    for remote in &settings.dimse_remotes {
        if config.remote(&remote.id).is_some() {
            bail!(
                "remote AE {} is named twice in PACS_DIMSE_REMOTES",
                remote.id
            );
        }
        config.remotes.push(remote.clone());
    }
    config.known_callers_only = settings.dimse_known_callers_only;
    if settings.dimse_max_associations == 0 {
        bail!("PACS_DIMSE_MAX_ASSOCIATIONS must be at least 1");
    }
    config.max_associations = settings.dimse_max_associations;
    config.timeout = std::time::Duration::from_secs(settings.dimse_timeout.max(1));
    config.move_timeout = std::time::Duration::from_secs(settings.dimse_move_timeout.max(1));
    config.max_data_set_bytes = settings.max_upload_bytes;
    let tls = dimse_tls::open(settings, config.remotes.iter().any(|remote| remote.secure))?;
    config.tls = tls.listener;
    for remote in config.remotes.iter_mut().filter(|remote| remote.secure) {
        remote.tls = tls.caller.clone();
    }
    Ok(config)
}

/// Serve DIMSE on `listener` until `shutdown` completes: C-ECHO, and C-STORE into
/// `archive` exactly as a REST upload, or through a project's de-identification for its AE
/// title.
pub async fn serve_dimse(
    listener: tokio::net::TcpListener,
    archive: Archive,
    config: pacs_dimse::Config,
    projects: Vec<std::sync::Arc<pacs_dimse::Project>>,
    shutdown: impl Future<Output = ()> + Send,
) {
    serve_dimse_audited(
        listener,
        archive,
        config,
        projects,
        audit::Audit::off(),
        shutdown,
    )
    .await
}

/// [`serve_dimse`], with what associations do written to `audit` (see [`audit`]).
pub async fn serve_dimse_audited(
    listener: tokio::net::TcpListener,
    archive: Archive,
    config: pacs_dimse::Config,
    projects: Vec<std::sync::Arc<pacs_dimse::Project>>,
    audit: audit::Audit,
    shutdown: impl Future<Output = ()> + Send,
) {
    let mut scp = pacs_dimse::Scp::new(config, pacs_dimse::Destination::Archive(archive));
    if audit.is_on() {
        scp = scp.with_observer(std::sync::Arc::new(audit));
    }
    for project in projects {
        scp = scp.with_route(
            project.ae_title.clone(),
            pacs_dimse::Destination::Project(project),
        );
    }
    std::sync::Arc::new(scp).serve(listener, shutdown).await;
}

/// Where the index is, parsed from `--index`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexLocation {
    Sqlite(PathBuf),
    Postgres(String),
}

impl IndexLocation {
    pub fn parse(text: &str) -> Self {
        if text.starts_with("postgres://") || text.starts_with("postgresql://") {
            IndexLocation::Postgres(text.to_owned())
        } else {
            IndexLocation::Sqlite(PathBuf::from(text.strip_prefix("sqlite:").unwrap_or(text)))
        }
    }
}

/// A Postgres URL with the TLS of `PACS_INDEX_TLS` put in, as the `sslmode` and
/// `sslrootcert` parameters. An error when the URL sets them itself (one place is enough),
/// when a certificate authority is given without `verify-full`, which is all that reads
/// it, or when its file cannot be read.
pub fn with_tls(url: &str, tls: IndexTls, ca_file: Option<&Path>) -> anyhow::Result<String> {
    let query = url.split_once('?').map_or("", |(_, query)| query);
    for parameter in ["sslmode", "sslrootcert", "ssl-mode", "ssl-root-cert"] {
        if query
            .split('&')
            .any(|pair| pair.split('=').next() == Some(parameter))
        {
            bail!(
                "the Postgres URL sets {parameter}; set the index's TLS with PACS_INDEX_TLS only"
            );
        }
    }
    let mut parameters = vec![format!(
        "sslmode={}",
        match tls {
            IndexTls::Off => "disable",
            IndexTls::Prefer => "prefer",
            IndexTls::VerifyFull => "verify-full",
        }
    )];
    if let Some(file) = ca_file {
        if tls != IndexTls::VerifyFull {
            bail!("PACS_INDEX_CA_FILE is read only with PACS_INDEX_TLS=verify-full");
        }
        std::fs::read(file)
            .with_context(|| format!("reading PACS_INDEX_CA_FILE {}", file.display()))?;
        let path = file
            .to_str()
            .with_context(|| format!("PACS_INDEX_CA_FILE is not UTF-8: {}", file.display()))?;
        let encoded: String = path
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect();
        parameters.push(format!("sslrootcert={encoded}"));
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    Ok(format!("{url}{separator}{}", parameters.join("&")))
}

/// `postgres://user@host/db` with `password` put in, percent-encoded. A URL that already
/// has a password is an error: one place for it is enough.
pub fn with_password(url: &str, password: &str) -> anyhow::Result<String> {
    let (scheme, rest) = url
        .split_once("://")
        .context("a Postgres URL looks like postgres://user@host/database")?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let Some((user, host)) = authority.rsplit_once('@') else {
        bail!("the Postgres URL needs a user (postgres://user@host/database) for its password");
    };
    if user.contains(':') {
        bail!("the Postgres URL has a password already; set it in one place only");
    }
    let encoded: String = password
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    let path = if path.is_empty() {
        String::new()
    } else {
        format!("/{path}")
    };
    Ok(format!("{scheme}://{user}:{encoded}@{host}{path}"))
}

/// A Postgres URL fit for logs.
pub fn without_password(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => match rest.split_once('@') {
            Some((user, host)) => {
                let user = user.split(':').next().unwrap_or(user);
                format!("{scheme}://{user}:***@{host}")
            }
            None => url.to_owned(),
        },
        None => url.to_owned(),
    }
}

/// The whole HTTP application: `/healthz`, the REST API at `/` (with the remote AEs at
/// `/modalities`), DICOMweb under
/// `dicomweb_path` and, unless turned off, the web interface at `/ui/` with its shared
/// hanging protocols at `/ui/api/protocols`, the viewer's measurements at
/// `/ui/api/annotations` and the paged study list at `/ui/api/studies`.
///
/// Without a login: with `PACS_AUTH` on, use [`app_with_login`], and this refuses.
pub fn app(archive: Archive, settings: &Settings) -> anyhow::Result<Router> {
    if settings.auth {
        bail!("PACS_AUTH is on: the application needs its login (open_login, app_with_login)");
    }
    app_with_login(archive, settings, None)
}

/// [`app`] behind `login`, when there is one: its routes are added and every route is
/// guarded (see [`auth`]). Without an audit trail: with `PACS_AUDIT_FILE` set, use
/// [`app_with_audit`], and this refuses.
pub fn app_with_login(
    archive: Archive,
    settings: &Settings,
    login: Option<auth::Login>,
) -> anyhow::Result<Router> {
    if settings.audit_file.is_some() {
        bail!(
            "PACS_AUDIT_FILE is set: the application needs its audit trail (open_audit, app_with_audit)"
        );
    }
    app_with_audit(archive, settings, login, audit::Audit::off())
}

/// [`app_with_login`] with `audit` around every route (see [`audit`]).
pub fn app_with_audit(
    archive: Archive,
    settings: &Settings,
    login: Option<auth::Login>,
    audit: audit::Audit,
) -> anyhow::Result<Router> {
    if settings.auth != login.is_some() {
        bail!("PACS_AUTH and the login given to the application disagree");
    }
    if settings.audit_file.is_some() != audit.is_on() {
        bail!("PACS_AUDIT_FILE and the audit trail given to the application disagree");
    }
    let path = format!("/{}", settings.dicomweb_path.trim_matches('/'));
    if path == "/" {
        bail!("the DICOMweb path cannot be /: the REST API is served there");
    }
    let dimse = dimse_config(settings)?;
    let project_titles: Vec<(String, pacs_dimse::AeTitle)> = projects::project_configs(settings)?
        .into_iter()
        .map(|p| (p.profile.name, p.ae_title))
        .collect();
    let rest =
        pacs_rest::router(archive.clone(), settings.max_upload_bytes).merge(modalities::router(
            dimse.clone(),
            settings.dimse_listen.is_some(),
            project_titles.clone(),
        ));
    let index = archive.index().clone();
    let audit_index = index.clone();
    let dicomweb = pacs_dicomweb::router(
        archive,
        pacs_dicomweb::Config {
            base_url: settings.base_url.clone(),
            mount_path: path.clone(),
            max_upload_bytes: settings.max_upload_bytes,
        },
    );
    let mut app = Router::new()
        .route("/healthz", get(|| async { "ok\n" }))
        .route(
            "/openapi.json",
            get(|| async { axum::Json(api::description()) }),
        )
        .merge(rest)
        .nest(&path, dicomweb);
    if settings.ui {
        if !ui::built() {
            tracing::warn!(
                "the web interface was not built into this binary; /ui/ says how to build it"
            );
        }
        app = app
            .merge(ui::router(&path, login.is_some()))
            .merge(protocols::router(index.clone()))
            .merge(annotations::router(index.clone()))
            .merge(worklist::router(index));
    }
    if let Some(login) = &login {
        // the guard goes on after every route is there, and inside CORS, which answers
        // preflight requests itself
        app = login.protect(app.merge(login.router()?));
    }
    // outside the guard, which it watches; inside CORS, whose preflights are no events
    app = audit.observe(app, audit_index, &path, dimse, project_titles);
    if !settings.cors_origins.is_empty() {
        app = app.layer(cors(&settings.cors_origins)?);
    }
    // outside everything that works for a request, inside the line that logs it
    app = limits::Limits::from_settings(settings).layer(app);
    // one line per request at INFO; tower-http's default is DEBUG
    let trace = TraceLayer::new_for_http()
        // as tower-http's default span at INFO, but without a download token in the URI
        .make_span_with(|request: &axum::http::Request<axum::body::Body>| {
            tracing::info_span!(
                "request",
                method = %request.method(),
                uri = %auth::for_log(request.uri()),
                version = ?request.version(),
            )
        })
        .on_response(DefaultOnResponse::new().level(Level::INFO));
    Ok(app.layer(trace))
}

fn cors(origins: &[String]) -> anyhow::Result<CorsLayer> {
    let allow = if origins.iter().any(|o| o == "*") {
        AllowOrigin::any()
    } else {
        let origins = origins
            .iter()
            .map(|o| HeaderValue::from_str(o.trim()).with_context(|| format!("CORS origin {o:?}")))
            .collect::<anyhow::Result<Vec<_>>>()?;
        AllowOrigin::list(origins)
    };
    Ok(CorsLayer::new()
        .allow_origin(allow)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::ACCEPT,
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            HeaderName::from_static("x-requested-with"),
        ])
        .expose_headers([header::CONTENT_TYPE, header::CONTENT_LENGTH]))
}

/// Serve `app` on `listener` until `shutdown` completes, then finish open requests, with
/// the default limits on connections ([`limits::serve`] takes others).
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    limits::serve(listener, app, limits::Limits::default(), shutdown).await
}

/// Whether the server listening at `listen` answers 200 on `/healthz` within 5 seconds.
/// A wildcard address (0.0.0.0, ::) is reached on loopback.
pub async fn healthy(listen: SocketAddr) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut target = listen;
    if target.ip().is_unspecified() {
        target.set_ip(if target.is_ipv4() {
            std::net::Ipv4Addr::LOCALHOST.into()
        } else {
            std::net::Ipv6Addr::LOCALHOST.into()
        });
    }
    let check = async {
        let mut stream = tokio::net::TcpStream::connect(target).await.ok()?;
        let request =
            format!("GET /healthz HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut head = [0u8; 12];
        stream.read_exact(&mut head).await.ok()?;
        Some(head.starts_with(b"HTTP/1.1 200"))
    };
    matches!(
        tokio::time::timeout(std::time::Duration::from_secs(5), check).await,
        Ok(Some(true))
    )
}

/// Ctrl-C, or SIGTERM from Docker and systemd.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("shutting down");
}
