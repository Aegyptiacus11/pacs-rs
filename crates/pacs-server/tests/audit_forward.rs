//! The audit trail sent to a repository over TLS (RFC 5425): a repository in the test
//! receives the frames; what waits while it is down, what is sent once and no more, and
//! what stops the start.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use pacs_server::Settings;
use pacs_server::audit_forward::{self, DEFAULT_PORT, frame};
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_rustls::TlsAcceptor;

struct Authority {
    dir: PathBuf,
    name: String,
    issuer: Issuer<'static, KeyPair>,
}

impl Authority {
    fn new(dir: &Path, name: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let key = KeyPair::generate().unwrap();
        std::fs::write(
            dir.join(format!("{name}.ca.pem")),
            params.self_signed(&key).unwrap().pem(),
        )
        .unwrap();
        Authority {
            dir: dir.to_owned(),
            name: name.to_owned(),
            issuer: Issuer::new(params, key),
        }
    }

    fn file(&self) -> String {
        self.dir
            .join(format!("{}.ca.pem", self.name))
            .display()
            .to_string()
    }

    fn issue(&self, name: &str, hosts: &[&str]) -> (String, String) {
        let hosts: Vec<String> = hosts.iter().map(|host| (*host).to_owned()).collect();
        let key = KeyPair::generate().unwrap();
        let certificate = CertificateParams::new(hosts)
            .unwrap()
            .signed_by(&key, &self.issuer)
            .unwrap();
        let files = (
            self.dir.join(format!("{name}.pem")),
            self.dir.join(format!("{name}.key")),
        );
        std::fs::write(&files.0, certificate.pem()).unwrap();
        std::fs::write(&files.1, key.serialize_pem()).unwrap();
        (files.0.display().to_string(), files.1.display().to_string())
    }
}

/// A repository: takes TLS connections on `listener` and keeps each framed message.
/// With `callers`, a caller must show a certificate one of those authorities signed.
struct Repository {
    address: SocketAddr,
    received: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
    /// The connections taken, to end them with the repository.
    connections: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl Repository {
    async fn start(port: u16, certificate: &(String, String), callers: Option<&str>) -> Repository {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&certificate.0)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let key = PrivateKeyDer::from_pem_file(&certificate.1).unwrap();
        let builder = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .unwrap();
        let builder = match callers {
            Some(file) => {
                let mut roots = rustls::RootCertStore::empty();
                for authority in CertificateDer::pem_file_iter(file).unwrap() {
                    roots.add(authority.unwrap()).unwrap();
                }
                let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
                    Arc::new(roots),
                    provider,
                )
                .build()
                .unwrap();
                builder.with_client_cert_verifier(verifier)
            }
            None => builder.with_no_client_auth(),
        };
        let acceptor = TlsAcceptor::from(Arc::new(builder.with_single_cert(chain, key).unwrap()));
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let kept = received.clone();
        let connections: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> = Arc::default();
        let taken = connections.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                let (acceptor, kept) = (acceptor.clone(), kept.clone());
                let connection = tokio::spawn(async move {
                    let Ok(mut stream) = acceptor.accept(socket).await else {
                        return;
                    };
                    // RFC 5425, 4.3: MSG-LEN SP SYSLOG-MSG, the length in octets
                    let mut buffer: Vec<u8> = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        while let Some(space) = buffer.iter().position(|b| *b == b' ') {
                            let length: usize = std::str::from_utf8(&buffer[..space])
                                .unwrap()
                                .parse()
                                .unwrap();
                            if buffer.len() < space + 1 + length {
                                break;
                            }
                            let message = buffer[space + 1..space + 1 + length].to_vec();
                            buffer.drain(..space + 1 + length);
                            kept.lock()
                                .unwrap()
                                .push(String::from_utf8(message).unwrap());
                        }
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                        }
                    }
                });
                taken.lock().unwrap().push(connection);
            }
        });
        Repository {
            address,
            received,
            task,
            connections,
        }
    }

    fn messages(&self) -> Vec<String> {
        self.received.lock().unwrap().clone()
    }

    /// Waits until `count` messages have arrived, and gives them all.
    async fn wait_for(&self, count: usize) -> Vec<String> {
        for _ in 0..200 {
            if self.messages().len() >= count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.messages()
    }

    /// Stops listening, and drops what it holds: the port is free again.
    async fn stop(self) -> u16 {
        self.task.abort();
        let _ = self.task.await;
        let connections: Vec<_> = self.connections.lock().unwrap().drain(..).collect();
        for connection in connections {
            connection.abort();
            let _ = connection.await;
        }
        self.address.port()
    }
}

fn settings(dir: &Path, extra: &[&str]) -> Settings {
    let mut args = vec![
        "pacs-server".to_owned(),
        "--index".into(),
        dir.join("index.sqlite").display().to_string(),
        "--storage".into(),
        format!("file:{}", dir.join("files").display()),
    ];
    args.extend(extra.iter().map(|s| (*s).to_owned()));
    Settings::try_parse_from(args).unwrap()
}

/// A forwarder running on the audit file of `dir`: how to tell it of a line, and how to
/// stop it and wait for it.
struct Running {
    written: Arc<Notify>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Running {
    fn start(dir: &Path, extra: &[&str]) -> Running {
        let forward = audit_forward::open(&settings(dir, extra)).unwrap().unwrap();
        let written = Arc::new(Notify::new());
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(forward.run(written.clone(), async {
            let _ = stopped.await;
        }));
        Running {
            written,
            stop: Some(stop),
            task,
        }
    }

    async fn stop(mut self) {
        let _ = self.stop.take().unwrap().send(());
        tokio::time::timeout(Duration::from_secs(20), self.task)
            .await
            .unwrap()
            .unwrap();
    }
}

fn append(file: &Path, lines: &[&str]) {
    use std::io::Write;
    let mut out = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(file)
        .unwrap();
    for line in lines {
        writeln!(out, "{line}").unwrap();
    }
}

#[test]
fn a_message_is_framed_by_its_length_in_octets() {
    assert_eq!(frame(b"<85>1 - - - - - - x"), b"19 <85>1 - - - - - - x");
    // octets, not characters
    assert_eq!(frame("é".as_bytes()), "2 é".as_bytes());
}

#[tokio::test]
async fn the_file_is_sent_in_order_and_waits_while_the_repository_is_down() {
    let dir = tempfile::tempdir().unwrap();
    let ca = Authority::new(dir.path(), "site");
    let certificate = ca.issue("repository", &["127.0.0.1"]);
    let own = ca.issue("pacs", &["pacs.test"]);
    let ca_file = ca.file();
    let file = dir.path().join("audit.log");
    let file_text = file.display().to_string();

    // lines written before any repository is there, one of them not ASCII
    append(&file, &["<85>1 one", "<84>1 two é", "<85>1 three"]);
    // a repository that asks its callers for a certificate
    let repository = Repository::start(0, &certificate, Some(&ca_file)).await;
    let at = format!("127.0.0.1:{}", repository.address.port());
    let extra = [
        "--audit-file",
        file_text.as_str(),
        "--audit-repository",
        at.as_str(),
        "--audit-repository-ca-file",
        ca_file.as_str(),
        "--audit-repository-cert-file",
        own.0.as_str(),
        "--audit-repository-key-file",
        own.1.as_str(),
    ];
    let running = Running::start(dir.path(), &extra);
    assert_eq!(
        repository.wait_for(3).await,
        ["<85>1 one", "<84>1 two é", "<85>1 three"]
    );

    // a new line goes as it is written
    append(&file, &["<85>1 four"]);
    running.written.notify_one();
    assert_eq!(repository.wait_for(4).await.len(), 4);

    // the repository goes away: what is written meanwhile waits in the file
    let port = repository.stop().await;
    append(&file, &["<85>1 five", "<85>1 six"]);
    running.written.notify_one();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    append(&file, &["<85>1 seven"]);
    running.written.notify_one();

    // and comes back: the three lines arrive, in order, and nothing sent before comes again
    let repository = Repository::start(port, &certificate, Some(&ca_file)).await;
    assert_eq!(
        repository.wait_for(3).await,
        ["<85>1 five", "<85>1 six", "<85>1 seven"]
    );
    running.stop().await;

    // a new start goes on from where the last one stopped: only what is new
    let before = repository.messages().len();
    append(&file, &["<85>1 eight"]);
    let running = Running::start(dir.path(), &extra);
    let received = repository.wait_for(before + 1).await;
    assert_eq!(received.len(), before + 1, "{received:?}");
    assert_eq!(received.last().unwrap(), "<85>1 eight");
    running.stop().await;
    let position: u64 = std::fs::read_to_string(dir.path().join("audit.log.sent"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(position, std::fs::metadata(&file).unwrap().len());

    // the file replaced by a shorter one (rotated): sent from its start
    std::fs::write(&file, "<85>1 new file\n").unwrap();
    let running = Running::start(dir.path(), &extra);
    let received = repository.wait_for(before + 2).await;
    assert_eq!(received.last().unwrap(), "<85>1 new file");
    // a line still being written, without its end, is not sent until it has one
    std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .map(|mut f| std::io::Write::write_all(&mut f, b"<85>1 half"))
        .unwrap()
        .unwrap();
    running.written.notify_one();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(repository.messages().len(), before + 2);
    append(&file, &[" and whole"]);
    running.written.notify_one();
    assert_eq!(
        repository.wait_for(before + 3).await.last().unwrap(),
        "<85>1 half and whole"
    );
    running.stop().await;
}

#[tokio::test]
async fn a_repository_that_does_not_check_out_gets_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let ca = Authority::new(dir.path(), "site");
    let other = Authority::new(dir.path(), "elsewhere");
    let certificate = ca.issue("repository", &["127.0.0.1"]);
    let stranger = other.issue("stranger", &["pacs.test"]);
    let (ca_file, other_file) = (ca.file(), other.file());
    let file = dir.path().join("audit.log");
    let file_text = file.display().to_string();
    append(&file, &["<85>1 secret"]);
    let repository = Repository::start(0, &certificate, Some(&ca_file)).await;
    let port = repository.address.port();
    let (by_address, by_name) = (format!("127.0.0.1:{port}"), format!("localhost:{port}"));

    for (what, extra) in [
        // signed by an authority pacs-rs was not given
        (
            "another authority",
            vec![
                "--audit-repository",
                by_address.as_str(),
                "--audit-repository-ca-file",
                other_file.as_str(),
            ],
        ),
        // reached under a name its certificate does not have
        (
            "another name",
            vec![
                "--audit-repository",
                by_name.as_str(),
                "--audit-repository-ca-file",
                ca_file.as_str(),
            ],
        ),
        // pacs-rs shows no certificate, or one the repository's authority did not sign
        (
            "no certificate",
            vec![
                "--audit-repository",
                by_address.as_str(),
                "--audit-repository-ca-file",
                ca_file.as_str(),
            ],
        ),
        (
            "a stranger's certificate",
            vec![
                "--audit-repository",
                by_address.as_str(),
                "--audit-repository-ca-file",
                ca_file.as_str(),
                "--audit-repository-cert-file",
                stranger.0.as_str(),
                "--audit-repository-key-file",
                stranger.1.as_str(),
            ],
        ),
    ] {
        let mut args = vec!["--audit-file", file_text.as_str()];
        args.extend(extra);
        let running = Running::start(dir.path(), &args);
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(
            repository.messages().is_empty(),
            "{what}: {:?}",
            repository.messages()
        );
        running.stop().await;
        // nothing counted as sent
        assert!(!dir.path().join("audit.log.sent").exists(), "{what}");
    }
}

#[test]
fn settings_that_do_not_add_up_stop_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let ca = Authority::new(dir.path(), "site");
    let (cert, key) = ca.issue("pacs", &["pacs.test"]);
    let ca_file = ca.file();
    let file = dir.path().join("audit.log").display().to_string();
    let missing = dir.path().join("missing.pem").display().to_string();
    let base = [
        "--audit-file",
        file.as_str(),
        "--audit-repository",
        "arr.test",
        "--audit-repository-ca-file",
        ca_file.as_str(),
    ];
    let with = |more: &[&'static str]| -> Vec<String> {
        base.iter()
            .map(|s| (*s).to_owned())
            .chain(more.iter().map(|s| (*s).to_owned()))
            .collect()
    };
    let open = |args: Vec<String>| {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        audit_forward::open(&settings(dir.path(), &args))
    };

    // nothing asked, nothing done
    assert!(open(vec![]).unwrap().is_none());
    assert!(
        open(vec!["--audit-file".into(), file.clone()])
            .unwrap()
            .is_none()
    );
    // the port of syslog over TLS unless another is given
    assert_eq!(
        open(with(&[])).unwrap().unwrap().repository(),
        format!("arr.test:{DEFAULT_PORT}")
    );
    let other = with(&[])
        .into_iter()
        .map(|s| {
            if s == "arr.test" {
                "[::1]:7000".into()
            } else {
                s
            }
        })
        .collect();
    assert_eq!(open(other).unwrap().unwrap().repository(), "::1:7000");
    open(with(&[])).unwrap();
    let mutual: Vec<String> = with(&[])
        .into_iter()
        .chain([
            "--audit-repository-cert-file".into(),
            cert.clone(),
            "--audit-repository-key-file".into(),
            key.clone(),
        ])
        .collect();
    open(mutual).unwrap();

    for (args, why) in [
        (
            vec![
                "--audit-repository",
                "arr.test",
                "--audit-repository-ca-file",
                ca_file.as_str(),
            ],
            "needs PACS_AUDIT_FILE",
        ),
        (
            vec![
                "--audit-file",
                file.as_str(),
                "--audit-repository",
                "arr.test",
            ],
            "needs PACS_AUDIT_REPOSITORY_CA_FILE",
        ),
        (
            vec![
                "--audit-file",
                file.as_str(),
                "--audit-repository-ca-file",
                ca_file.as_str(),
            ],
            "names no repository",
        ),
        (
            vec![
                "--audit-file",
                file.as_str(),
                "--audit-repository",
                "arr.test:0",
                "--audit-repository-ca-file",
                ca_file.as_str(),
            ],
            "is not host or host:port",
        ),
        (
            vec![
                "--audit-file",
                file.as_str(),
                "--audit-repository",
                "arr.test:x",
                "--audit-repository-ca-file",
                ca_file.as_str(),
            ],
            "is not host or host:port",
        ),
        (
            vec![
                "--audit-file",
                file.as_str(),
                "--audit-repository",
                "arr test",
                "--audit-repository-ca-file",
                ca_file.as_str(),
            ],
            "is not host or host:port",
        ),
        (
            vec![
                "--audit-file",
                file.as_str(),
                "--audit-repository",
                "arr.test",
                "--audit-repository-ca-file",
                missing.as_str(),
            ],
            "reading PACS_AUDIT_REPOSITORY_CA_FILE",
        ),
        (
            vec![
                "--audit-file",
                file.as_str(),
                "--audit-repository",
                "arr.test",
                "--audit-repository-ca-file",
                ca_file.as_str(),
                "--audit-repository-cert-file",
                cert.as_str(),
            ],
            "go together",
        ),
        (
            vec![
                "--audit-file",
                file.as_str(),
                "--audit-repository",
                "arr.test",
                "--audit-repository-ca-file",
                ca_file.as_str(),
                "--audit-repository-cert-file",
                cert.as_str(),
                "--audit-repository-key-file",
                missing.as_str(),
            ],
            "reading PACS_AUDIT_REPOSITORY_KEY_FILE",
        ),
    ] {
        let error = audit_forward::open(&settings(dir.path(), &args))
            .err()
            .unwrap_or_else(|| panic!("{args:?}: accepted"));
        assert!(format!("{error:#}").contains(why), "{args:?}: {error:#}");
    }
}
