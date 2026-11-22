//! DIMSE over TLS (PS3.15 B.12), from the settings to the wire: pacs-rs as listener and as
//! caller, with certificates made here, and every way a peer is refused.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use dicom_ul::ClientAssociationOptions;
use pacs_archive::Archive;
use pacs_core::real::RealSeries;
use pacs_dimse::command::{Command, PRIORITY_MEDIUM, field, tag};
use pacs_dimse::message::{self, Incoming};
use pacs_dimse::{AeTitle, Status};
use pacs_server::{Settings, dimse_config, open_archive, serve_dimse};
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, KeyUsagePurpose};

const CT: &str = "1.2.840.10008.5.1.4.1.1.2";
const WAIT: Duration = Duration::from_secs(10);

/// A certificate authority that signs what the tests need, its files in one directory.
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
        let certificate = params.self_signed(&key).unwrap();
        std::fs::write(dir.join(format!("{name}.ca.pem")), certificate.pem()).unwrap();
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

    /// A certificate for `hosts` and its key: the two files.
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

/// A pacs-rs listening for DIMSE with `extra` settings: its archive and its address.
async fn listening(dir: &Path, extra: &[&str]) -> (Archive, SocketAddr) {
    std::fs::create_dir_all(dir).unwrap();
    let settings = settings(dir, extra);
    let archive = open_archive(&settings).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = dimse_config(&settings).unwrap();
    tokio::spawn(serve_dimse(
        listener,
        archive.clone(),
        config,
        Vec::new(),
        std::future::pending(),
    ));
    (archive, address)
}

/// C-ECHO from a pacs-rs configured with `extra` to the AE `PACSRS` at `host:port`,
/// reached as `suffix` says (`/tls` or nothing).
async fn echo(
    dir: &Path,
    extra: &[&str],
    host: &str,
    port: u16,
    suffix: &str,
) -> Result<(), String> {
    let remote = format!("there=PACSRS@{host}:{port}{suffix}");
    let mut args = vec![
        "--dimse-remotes",
        remote.as_str(),
        "--dimse-ae-title",
        "CALLER",
    ];
    args.extend(extra);
    let config = dimse_config(&settings(dir, &args)).map_err(|e| format!("{e:#}"))?;
    let remote = config.remote("there").unwrap();
    match pacs_dimse::echo(remote, &AeTitle::new("CALLER").unwrap(), WAIT).await {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("{e:#}")),
    }
}

#[tokio::test]
async fn pacs_rs_calls_pacs_rs_over_tls() {
    let dir = tempfile::tempdir().unwrap();
    let ca = Authority::new(dir.path(), "site");
    let other = Authority::new(dir.path(), "elsewhere");
    let (cert, key) = ca.issue("listener", &["127.0.0.1"]);
    let (caller_cert, caller_key) = ca.issue("caller", &["caller.test"]);
    let (stranger_cert, stranger_key) = other.issue("stranger", &["stranger.test"]);
    let (ca_file, other_file) = (ca.file(), other.file());
    let (archive, address) = listening(
        &dir.path().join("listener"),
        &[
            "--dimse-tls-cert-file",
            &cert,
            "--dimse-tls-key-file",
            &key,
            "--dimse-tls-ca-file",
            &ca_file,
        ],
    )
    .await;
    let port = address.port();
    let caller = dir.path().join("caller");
    std::fs::create_dir_all(&caller).unwrap();
    let with_certificate = [
        "--dimse-tls-ca-file",
        ca_file.as_str(),
        "--dimse-tls-cert-file",
        caller_cert.as_str(),
        "--dimse-tls-key-file",
        caller_key.as_str(),
        // only its calls are under test: it asks nothing of callers of its own
        "--dimse-tls-client-auth",
        "off",
    ];

    // both sides show a certificate the other's authority signed
    echo(&caller, &with_certificate, "127.0.0.1", port, "/tls")
        .await
        .unwrap();

    // refused by the listener: no certificate, and one of another authority
    let none = echo(
        &caller,
        &["--dimse-tls-ca-file", &ca_file],
        "127.0.0.1",
        port,
        "/tls",
    )
    .await;
    assert!(none.is_err(), "a caller without a certificate was taken");
    let stranger = [
        "--dimse-tls-ca-file",
        ca_file.as_str(),
        "--dimse-tls-cert-file",
        stranger_cert.as_str(),
        "--dimse-tls-key-file",
        stranger_key.as_str(),
        "--dimse-tls-client-auth",
        "off",
    ];
    let refused = echo(&caller, &stranger, "127.0.0.1", port, "/tls").await;
    assert!(
        refused.is_err(),
        "a certificate of another authority was taken"
    );

    // refused by the caller: a listener whose certificate its authorities did not sign,
    // and one reached under a name its certificate does not have
    let mut mistrusting = with_certificate;
    mistrusting[1] = other_file.as_str();
    let refused = echo(&caller, &mistrusting, "127.0.0.1", port, "/tls").await;
    assert!(
        refused.is_err(),
        "a listener of another authority was called"
    );
    let refused = echo(&caller, &with_certificate, "localhost", port, "/tls").await;
    assert!(
        refused.is_err(),
        "a certificate for 127.0.0.1 was taken for localhost"
    );

    // a caller that does not speak TLS gets nothing from a TLS port
    let plain = echo(&caller, &[], "127.0.0.1", port, "").await;
    assert!(
        plain.is_err(),
        "a plain association was accepted on a TLS port"
    );

    // and a remote AE marked /tls that does not speak it is not called in plain
    let (_, plain_address) = listening(&dir.path().join("plain"), &[]).await;
    let refused = echo(
        &caller,
        &with_certificate,
        "127.0.0.1",
        plain_address.port(),
        "/tls",
    )
    .await;
    assert!(
        refused.is_err(),
        "a plain listener was called as if over TLS"
    );
    echo(&caller, &[], "127.0.0.1", plain_address.port(), "")
        .await
        .unwrap();

    // an instance stored over TLS is in the archive like any other
    let config = dimse_config(&settings(
        &caller,
        &[
            "--dimse-remotes",
            &format!("there=PACSRS@127.0.0.1:{port}/tls"),
            "--dimse-tls-ca-file",
            &ca_file,
            "--dimse-tls-cert-file",
            &caller_cert,
            "--dimse-tls-key-file",
            &caller_key,
            "--dimse-tls-client-auth",
            "off",
        ],
    ))
    .unwrap();
    let tls: Arc<rustls::ClientConfig> = config.remote("there").unwrap().tls.clone().unwrap().0;
    let series = RealSeries::new(121, 1);
    let file = series.slice_bytes(0);
    let meta = u32::from_le_bytes(file[140..144].try_into().unwrap()) as usize;
    let mut association = ClientAssociationOptions::new()
        .calling_ae_title("CALLER")
        .called_ae_title("PACSRS")
        .with_presentation_context(CT, vec!["1.2.840.10008.1.2.1"])
        .tls_config(tls)
        .server_name("127.0.0.1")
        .establish_tls_async(address)
        .await
        .unwrap();
    let context = association.presentation_contexts()[0].id;
    let request = Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, CT)
        .with_us(tag::COMMAND_FIELD, field::C_STORE_RQ)
        .with_us(tag::MESSAGE_ID, 1)
        .with_us(tag::PRIORITY, PRIORITY_MEDIUM)
        .with_us(tag::COMMAND_DATA_SET_TYPE, 0)
        .with_ui(tag::AFFECTED_SOP_INSTANCE_UID, &series.instance_uid(0));
    message::send(
        &mut association,
        context,
        &request,
        Some(&file[144 + meta..]),
    )
    .await
    .unwrap();
    let Incoming::Message(response) = message::receive(&mut association, 0).await.unwrap() else {
        panic!("no C-STORE-RSP");
    };
    assert_eq!(response.command.status(), Ok(Status::SUCCESS));
    association.release().await.unwrap();
    assert!(
        archive
            .index()
            .study(&series.study_uid())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn without_client_authentication_any_tls_caller_is_taken() {
    let dir = tempfile::tempdir().unwrap();
    let ca = Authority::new(dir.path(), "site");
    let (cert, key) = ca.issue("listener", &["127.0.0.1", "localhost"]);
    let ca_file = ca.file();
    let (_, address) = listening(
        &dir.path().join("listener"),
        &[
            "--dimse-tls-cert-file",
            &cert,
            "--dimse-tls-key-file",
            &key,
            "--dimse-tls-client-auth",
            "off",
        ],
    )
    .await;
    let caller = dir.path().join("caller");
    std::fs::create_dir_all(&caller).unwrap();
    let trusting = ["--dimse-tls-ca-file", ca_file.as_str()];
    // no certificate of its own, and by either name of the listener's certificate
    echo(&caller, &trusting, "127.0.0.1", address.port(), "/tls")
        .await
        .unwrap();
    echo(&caller, &trusting, "localhost", address.port(), "/tls")
        .await
        .unwrap();
    // still TLS only
    assert!(
        echo(&caller, &[], "127.0.0.1", address.port(), "")
            .await
            .is_err()
    );
}

#[test]
fn settings_that_do_not_add_up_stop_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let ca = Authority::new(dir.path(), "site");
    let (cert, key) = ca.issue("listener", &["127.0.0.1"]);
    let (_, other_key) = ca.issue("other", &["127.0.0.1"]);
    let ca_file = ca.file();
    let empty = dir.path().join("empty.pem").display().to_string();
    std::fs::write(&empty, "").unwrap();
    let missing = dir.path().join("missing.pem").display().to_string();
    let tls_remote = "--dimse-remotes=there=PACS@pacs.test:2762/tls";

    for (extra, why) in [
        (vec!["--dimse-tls-cert-file", &cert], "go together"),
        (vec!["--dimse-tls-key-file", &key], "go together"),
        // a listener that asks callers for certificates needs authorities to check them by
        (
            vec!["--dimse-tls-cert-file", &cert, "--dimse-tls-key-file", &key],
            "PACS_DIMSE_TLS_CLIENT_AUTH=off",
        ),
        (vec![tls_remote], "reached over TLS"),
        (vec!["--dimse-tls-ca-file", &ca_file], "nothing reads it"),
        (
            vec![
                "--dimse-tls-cert-file",
                &cert,
                "--dimse-tls-key-file",
                &key,
                "--dimse-tls-client-auth",
                "off",
                "--dimse-tls-ca-file",
                &ca_file,
            ],
            "nothing reads it",
        ),
        (
            vec![
                "--dimse-tls-cert-file",
                &empty,
                "--dimse-tls-key-file",
                &key,
                "--dimse-tls-client-auth",
                "off",
            ],
            "holds no certificate",
        ),
        (
            vec![
                "--dimse-tls-cert-file",
                &missing,
                "--dimse-tls-key-file",
                &key,
                "--dimse-tls-client-auth",
                "off",
            ],
            "reading PACS_DIMSE_TLS_CERT_FILE",
        ),
        (
            vec![
                "--dimse-tls-cert-file",
                &cert,
                "--dimse-tls-key-file",
                &missing,
                "--dimse-tls-client-auth",
                "off",
            ],
            "reading PACS_DIMSE_TLS_KEY_FILE",
        ),
        (
            vec![
                "--dimse-tls-cert-file",
                &cert,
                "--dimse-tls-key-file",
                &other_key,
                "--dimse-tls-client-auth",
                "off",
            ],
            "do not go together",
        ),
        (
            vec![tls_remote, "--dimse-tls-ca-file", &missing],
            "reading PACS_DIMSE_TLS_CA_FILE",
        ),
        (
            vec![tls_remote, "--dimse-tls-ca-file", &empty],
            "holds no certificate",
        ),
    ] {
        let error = dimse_config(&settings(dir.path(), &extra)).unwrap_err();
        assert!(format!("{error:#}").contains(why), "{extra:?}: {error:#}");
    }

    // and those that do: plain by default, TLS for the listener, for calls, for both
    let config = dimse_config(&settings(
        dir.path(),
        &["--dimse-remotes=there=PACS@pacs.test:104"],
    ))
    .unwrap();
    assert!(config.tls.is_none() && !config.remotes[0].secure && config.remotes[0].tls.is_none());
    let config = dimse_config(&settings(
        dir.path(),
        &[
            "--dimse-tls-cert-file",
            &cert,
            "--dimse-tls-key-file",
            &key,
            "--dimse-tls-ca-file",
            &ca_file,
        ],
    ))
    .unwrap();
    assert!(config.tls.is_some());
    let config = dimse_config(&settings(
        dir.path(),
        &[tls_remote, "--dimse-tls-ca-file", &ca_file],
    ))
    .unwrap();
    assert!(config.tls.is_none() && config.remotes[0].secure && config.remotes[0].tls.is_some());
    // a plain remote beside a TLS one stays plain
    let config = dimse_config(&settings(
        dir.path(),
        &[
            "--dimse-remotes=a=PACS@pacs.test:2762/tls,b=OLD@old.test:104",
            "--dimse-tls-ca-file",
            &ca_file,
        ],
    ))
    .unwrap();
    assert!(config.remote("a").unwrap().tls.is_some());
    assert!(config.remote("b").unwrap().tls.is_none());
}

#[test]
fn a_remote_ae_is_marked_tls_after_its_port() {
    let parse = |text: &str| text.parse::<pacs_dimse::RemoteAe>();
    let remote = parse("pacs=PACS@pacs.test:2762/tls").unwrap();
    assert!(remote.secure);
    assert_eq!((remote.host.as_str(), remote.port), ("pacs.test", 2762));
    assert!(parse("pacs=PACS@[::1]:2762/tls").unwrap().secure);
    assert!(!parse("pacs=PACS@pacs.test:104").unwrap().secure);
    for bad in [
        "pacs=PACS@pacs.test:2762/ssl",
        "pacs=PACS@pacs.test:2762/",
        "pacs=PACS@pacs.test/tls",
    ] {
        assert!(parse(bad).is_err(), "{bad}");
    }
}

/// Over TLS the called AE title is inside the session, so every service is negotiated, and
/// an AE title's own capabilities hold per operation: a C-ECHO to an AE title that offers
/// storage only is refused 0122, one to pacs-rs's own AE title answered.
#[tokio::test]
async fn over_tls_an_ae_titles_capabilities_hold_per_operation() {
    let dir = tempfile::tempdir().unwrap();
    let ca = Authority::new(dir.path(), "site");
    let (cert, key) = ca.issue("listener", &["127.0.0.1"]);
    let ca_file = ca.file();
    let (_, address) = listening(
        &dir.path().join("listener"),
        &[
            "--dimse-tls-cert-file",
            &cert,
            "--dimse-tls-key-file",
            &key,
            "--dimse-tls-client-auth",
            "off",
            "--dimse-extra-ae-titles",
            "RESEARCH",
            "--dimse-capabilities",
            "RESEARCH=storage",
        ],
    )
    .await;
    let caller = dir.path().join("caller");
    std::fs::create_dir_all(&caller).unwrap();
    let call = async |called: &str| {
        let remote = format!("there={called}@127.0.0.1:{}/tls", address.port());
        let config = dimse_config(&settings(
            &caller,
            &[
                "--dimse-remotes",
                &remote,
                "--dimse-ae-title",
                "CALLER",
                "--dimse-tls-ca-file",
                &ca_file,
            ],
        ))
        .unwrap();
        let remote = config.remote("there").unwrap().clone();
        pacs_dimse::echo(&remote, &AeTitle::new("CALLER").unwrap(), WAIT).await
    };
    assert!(call("PACSRS").await.is_ok());
    match call("RESEARCH").await {
        Err(pacs_dimse::EchoError::Status(status)) => {
            assert_eq!(status, Status::SOP_CLASS_NOT_SUPPORTED)
        }
        other => panic!("{other:?}"),
    }
}
