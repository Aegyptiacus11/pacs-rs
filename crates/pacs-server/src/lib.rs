//! The pacs-rs server: settings, assembly of the HTTP application, and serving it.
//!
//! Settings come from flags or `PACS_*` environment variables. Secrets (the S3 keys) come
//! only from the environment, or from a file named by the variable with `_FILE` appended
//! (Docker and Portainer secrets), never from flags, which other users can read in `ps`.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, bail};
use axum::Router;
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::routing::get;
use clap::Parser;
use pacs_archive::{Archive, Compression};
use pacs_index::Index;
use pacs_store::{S3Config, Store};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

mod protocols;
mod ui;

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

    /// Where files go: `file:<directory>` or `s3://<bucket>[/<prefix>]`.
    #[arg(long, env = "PACS_STORAGE", default_value = "file:data/files")]
    pub storage: String,

    /// S3 endpoint for a non-AWS store, e.g. `http://homeserver:8333`.
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

    /// Check that a server with these settings answers on /healthz, exit 0 if it does and
    /// 1 if not, and do nothing else: a health check for images without a shell or curl.
    #[arg(long)]
    pub healthcheck: bool,

    /// Origins browsers may call from, comma-separated, or `*`. None by default.
    #[arg(long, env = "PACS_CORS_ORIGINS", value_delimiter = ',')]
    pub cors_origins: Vec<String>,

    /// Serve the web interface at /ui/ (`/` redirects there). It has no login of its own:
    /// whoever reaches it can delete, as with the REST API.
    #[arg(long, env = "PACS_UI", default_value_t = true, action = clap::ArgAction::Set)]
    pub ui: bool,
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

fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Open the store and the index the settings name.
pub async fn open_archive(settings: &Settings) -> anyhow::Result<Archive> {
    let store = match StorageLocation::parse(&settings.storage)? {
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
    };
    let index = match IndexLocation::parse(&settings.index) {
        IndexLocation::Postgres(url) => {
            let url = match secret("PACS_INDEX_PASSWORD", process_env)? {
                Some(password) => with_password(&url, &password)?,
                None => url,
            };
            Index::connect(&url)
                .await
                .with_context(|| format!("opening the Postgres index {}", without_password(&url)))?
        }
        IndexLocation::Sqlite(path) => {
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
        .with_compression(settings.compression.into()))
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

/// The whole HTTP application: `/healthz`, the REST API at `/`, DICOMweb under
/// `dicomweb_path` and, unless turned off, the web interface at `/ui/` with its shared
/// hanging protocols at `/ui/api/protocols`.
pub fn app(archive: Archive, settings: &Settings) -> anyhow::Result<Router> {
    let path = format!("/{}", settings.dicomweb_path.trim_matches('/'));
    if path == "/" {
        bail!("the DICOMweb path cannot be /: the REST API is served there");
    }
    let rest = pacs_rest::router(archive.clone(), settings.max_upload_bytes);
    let index = archive.index().clone();
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
        .merge(rest)
        .nest(&path, dicomweb);
    if settings.ui {
        if !ui::built() {
            tracing::warn!(
                "the web interface was not built into this binary; /ui/ says how to build it"
            );
        }
        app = app.merge(ui::router(&path)).merge(protocols::router(index));
    }
    if !settings.cors_origins.is_empty() {
        app = app.layer(cors(&settings.cors_origins)?);
    }
    // one line per request at INFO; tower-http's default is DEBUG
    let trace = TraceLayer::new_for_http()
        .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
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

/// Serve `app` on `listener` until `shutdown` completes, then finish open requests.
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
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
