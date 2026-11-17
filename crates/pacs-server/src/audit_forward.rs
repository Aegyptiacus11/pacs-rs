//! The audit trail sent on to an audit record repository: syslog over TLS (RFC 5425), the
//! transport of IHE's Record Audit Event [ITI-20].
//!
//! What is sent is the audit file (`PACS_AUDIT_FILE`), line by line: the file is the queue.
//! A position kept beside it (`<file>.sent`) says how far the repository has been given
//! the file, so that a repository that cannot be reached loses nothing: the lines wait in
//! the file, however long, across restarts, and go in order once it answers again.
//!
//! By RFC 5425: the sender is the TLS client; each message is framed by its length in
//! octets and a space (4.3); the port is 6514 unless said (4.1); the connection ends with a
//! close_notify (4.4); the repository's certificate is validated up to a configured
//! authority and must name the configured host (5.2). Not done: authorizing a peer by its
//! certificate's fingerprint (5.1), and the cipher suite the RFC made mandatory in 2009,
//! TLS_RSA_WITH_AES_128_CBC_SHA, which BCP 195 has since ruled out.
//!
//! TLS gives no acknowledgement of a message, only of bytes handed to the connection: a
//! repository that stops between taking a message off the wire and keeping it loses that
//! message, as with any sender of this protocol.

use std::future::Future;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::Settings;
use crate::dimse_tls::{authorities, identity};

/// The port of syslog over TLS (RFC 5425, 4.1).
pub const DEFAULT_PORT: u16 = 6514;
/// The longest wait before trying a repository that could not be reached again.
const MAX_RETRY: Duration = Duration::from_secs(60);
/// How long a connection or a write may take.
const TIMEOUT: Duration = Duration::from_secs(30);
/// How long a new connection is watched before anything is sent on it: a repository that
/// refuses the caller's certificate says so only after the handshake, in TLS 1.3.
const SETTLE: Duration = Duration::from_millis(500);
/// The file is looked at this often even when nothing said it grew.
const POLL: Duration = Duration::from_secs(5);

/// Sends the audit file to a repository.
pub struct Forward {
    host: String,
    port: u16,
    tls: Arc<rustls::ClientConfig>,
    file: PathBuf,
    position: PathBuf,
}

/// `host` or `host:port`, an IPv6 address in brackets.
fn address(text: &str) -> anyhow::Result<(String, u16)> {
    let text = text.trim();
    let bad = || format!("PACS_AUDIT_REPOSITORY: {text:?} is not host or host:port");
    let (host, port) = match text.rsplit_once(':') {
        // the colons of a bare IPv6 address are not a port
        Some((host, port)) if !host.contains(':') || host.ends_with(']') => {
            let port: u16 = port.parse().ok().filter(|p| *p != 0).with_context(bad)?;
            (host, port)
        }
        _ => (text, DEFAULT_PORT),
    };
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty() || host.contains(['/', ' ', '@']) {
        bail!(bad());
    }
    Ok((host.to_owned(), port))
}

/// The forwarding the settings ask for; none without `PACS_AUDIT_REPOSITORY`. What is asked
/// without what it needs stops the start.
pub fn open(settings: &Settings) -> anyhow::Result<Option<Forward>> {
    let own = (
        &settings.audit_repository_cert_file,
        &settings.audit_repository_key_file,
    );
    let Some(repository) = &settings.audit_repository else {
        if settings.audit_repository_ca_file.is_some() || own.0.is_some() || own.1.is_some() {
            bail!("PACS_AUDIT_REPOSITORY_* are set, and PACS_AUDIT_REPOSITORY names no repository");
        }
        return Ok(None);
    };
    let (host, port) = address(repository)?;
    let Some(file) = &settings.audit_file else {
        bail!(
            "PACS_AUDIT_REPOSITORY needs PACS_AUDIT_FILE: the file is what is sent, and what waits when the repository cannot be reached"
        );
    };
    let Some(ca) = &settings.audit_repository_ca_file else {
        bail!(
            "PACS_AUDIT_REPOSITORY needs PACS_AUDIT_REPOSITORY_CA_FILE: the authorities that sign the repository's certificate"
        );
    };
    let roots = authorities(ca, "PACS_AUDIT_REPOSITORY_CA_FILE")?;
    ServerName::try_from(host.clone()).with_context(|| {
        format!("PACS_AUDIT_REPOSITORY: {host:?} is not a host name or an address")
    })?;
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .context("the TLS versions of the audit repository's connection")?
    .with_root_certificates(roots);
    let settings_named = (
        "PACS_AUDIT_REPOSITORY_CERT_FILE",
        "PACS_AUDIT_REPOSITORY_KEY_FILE",
    );
    let tls = match own {
        (Some(certificate), Some(key)) => {
            let (chain, key) = identity(certificate, key, settings_named)?;
            builder.with_client_auth_cert(chain, key).context(
                "PACS_AUDIT_REPOSITORY_CERT_FILE and PACS_AUDIT_REPOSITORY_KEY_FILE do not go together",
            )?
        }
        (None, None) => builder.with_no_client_auth(),
        _ => {
            bail!("PACS_AUDIT_REPOSITORY_CERT_FILE and PACS_AUDIT_REPOSITORY_KEY_FILE go together")
        }
    };
    let mut position = file.clone().into_os_string();
    position.push(".sent");
    Ok(Some(Forward {
        host,
        port,
        tls: Arc::new(tls),
        file: file.clone(),
        position: PathBuf::from(position),
    }))
}

/// One syslog message framed for the wire: its length in octets, a space, the message.
pub fn frame(message: &[u8]) -> Vec<u8> {
    let mut framed = format!("{} ", message.len()).into_bytes();
    framed.extend_from_slice(message);
    framed
}

async fn read_position(path: &Path) -> u64 {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => text.trim().parse().unwrap_or(0),
        Err(_) => 0,
    }
}

/// Keeps the position by writing beside it and renaming, so that a stop in the middle
/// leaves the old one or the new one, never half of one.
async fn keep_position(path: &Path, position: u64) -> std::io::Result<()> {
    let mut beside = path.to_owned().into_os_string();
    beside.push(".new");
    tokio::fs::write(&beside, position.to_string()).await?;
    tokio::fs::rename(&beside, path).await
}

/// Whether the repository has closed the connection, or refused it, as far as can be told
/// by waiting `wait` for it to say so. A repository sends nothing else on this connection.
async fn gone(stream: &mut TlsStream<TcpStream>, wait: Duration) -> bool {
    let mut byte = [0u8; 1];
    match tokio::time::timeout(wait, stream.read(&mut byte)).await {
        // nothing to read: the connection stands
        Err(_) => false,
        Ok(Ok(read)) => read == 0,
        Ok(Err(_)) => true,
    }
}

impl Forward {
    pub fn repository(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    async fn connect(&self) -> anyhow::Result<TlsStream<TcpStream>> {
        let name = ServerName::try_from(self.host.clone())?;
        let connecting = async {
            let socket = TcpStream::connect((self.host.as_str(), self.port)).await?;
            TlsConnector::from(self.tls.clone())
                .connect(name, socket)
                .await
        };
        let mut stream = tokio::time::timeout(TIMEOUT, connecting)
            .await
            .context("no answer in time")??;
        if gone(&mut stream, SETTLE).await {
            bail!("the repository closed the connection: it may not accept pacs-rs's certificate");
        }
        Ok(stream)
    }

    /// The whole lines of the file from `position` on, and where they end. A file shorter
    /// than the position was replaced by a new one: it is sent from its start.
    async fn pending(&self, position: u64) -> std::io::Result<(Vec<Vec<u8>>, u64)> {
        let mut file = tokio::fs::File::open(&self.file).await?;
        let length = file.metadata().await?.len();
        let from = if length < position { 0 } else { position };
        if length == from {
            return Ok((Vec::new(), from));
        }
        file.seek(SeekFrom::Start(from)).await?;
        let mut bytes = Vec::new();
        // a bound on what one pass holds in memory; the rest comes in the next
        file.take(4 << 20).read_to_end(&mut bytes).await?;
        let mut lines = Vec::new();
        let mut end = from;
        let mut start = 0;
        while let Some(length) = bytes[start..].iter().position(|b| *b == b'\n') {
            let line = &bytes[start..start + length];
            if !line.is_empty() {
                lines.push(line.to_vec());
            }
            start += length + 1;
            end = from + start as u64;
        }
        Ok((lines, end))
    }

    /// Sends what the file has from `position`, one frame a line, and keeps the position
    /// after each pass. Returns when there is nothing more for now.
    async fn send_pending(
        &self,
        stream: &mut TlsStream<TcpStream>,
        position: &mut u64,
    ) -> anyhow::Result<usize> {
        let mut sent = 0;
        loop {
            let (lines, end) = self
                .pending(*position)
                .await
                .context("reading the audit file")?;
            if lines.is_empty() {
                if end != *position {
                    *position = end;
                    keep_position(&self.position, end)
                        .await
                        .context("keeping the position")?;
                }
                return Ok(sent);
            }
            // A connection the repository has closed still takes a write: asked first, so
            // that the lines are not counted as sent to nobody.
            if gone(stream, Duration::from_millis(1)).await {
                bail!("the repository closed the connection");
            }
            let mut out = Vec::new();
            for line in &lines {
                out.extend(frame(line));
            }
            let writing = async {
                stream.write_all(&out).await?;
                stream.flush().await
            };
            tokio::time::timeout(TIMEOUT, writing)
                .await
                .context("the repository stopped reading")?
                .context("writing to the repository")?;
            sent += lines.len();
            *position = end;
            keep_position(&self.position, end)
                .await
                .context("keeping the position")?;
        }
    }

    /// Sends the file to the repository until `stop` completes, then what is left, once.
    /// `written` tells it of new lines. A repository that cannot be reached, or whose
    /// certificate does not check out, is tried again, each time after a longer wait, up
    /// to a minute: the lines stay in the file meanwhile.
    pub async fn run(self, written: Arc<Notify>, stop: impl Future<Output = ()> + Send) {
        tokio::pin!(stop);
        let mut position = read_position(&self.position).await;
        let mut retry = Duration::from_secs(1);
        let mut down_since_logged = false;
        let mut stopping = false;
        loop {
            let attempt = async {
                let mut stream = self.connect().await.context("connecting")?;
                let sent = self.send_pending(&mut stream, &mut position).await;
                anyhow::Ok((stream, sent))
            };
            match attempt.await {
                Ok((mut stream, Ok(_))) => {
                    if std::mem::take(&mut down_since_logged) {
                        tracing::info!(repository = %self.repository(), "the audit repository is reached again");
                    }
                    retry = Duration::from_secs(1);
                    // connected and up to date: send each new line as it is written
                    loop {
                        if stopping {
                            // RFC 5425, 4.4: a close_notify before the connection closes
                            let _ = tokio::time::timeout(TIMEOUT, stream.shutdown()).await;
                            return;
                        }
                        tokio::select! {
                            () = &mut stop => stopping = true,
                            () = written.notified() => {}
                            () = tokio::time::sleep(POLL) => {}
                        }
                        if let Err(error) = self.send_pending(&mut stream, &mut position).await {
                            tracing::warn!(repository = %self.repository(), error = %format!("{error:#}"), "the audit repository stopped taking messages; they wait in the audit file");
                            down_since_logged = true;
                            break;
                        }
                    }
                }
                Ok((_, Err(error))) | Err(error) => {
                    if !down_since_logged {
                        tracing::warn!(repository = %self.repository(), error = %format!("{error:#}"), "the audit repository cannot be reached; the messages wait in the audit file");
                    }
                    down_since_logged = true;
                }
            }
            if stopping {
                tracing::warn!(repository = %self.repository(), "stopping with audit messages not sent; they are in the audit file and go at the next start");
                return;
            }
            tokio::select! {
                () = &mut stop => stopping = true,
                () = tokio::time::sleep(retry) => {}
            }
            retry = (retry * 2).min(MAX_RETRY);
        }
    }
}
