//! pacs-rs against other implementations: DCMTK's echoscu, storescu and storescp, and
//! pynetdicom. Ignored by default; with the tools installed:
//!
//! ```text
//! cargo test -p pacs-dimse --test interop -- --ignored
//! ```
//!
//! DCMTK is found on PATH, or in `PACS_TEST_DCMTK_BIN`; pynetdicom in the Python named by
//! `PACS_TEST_PYTHON` (default `python3`).

mod common;

use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use common::*;
use pacs_core::synthetic::SyntheticSeries;
use pacs_dimse::{AeTitle, RemoteAe};

fn dcmtk(tool: &str) -> Command {
    let path = match std::env::var_os("PACS_TEST_DCMTK_BIN") {
        Some(dir) => PathBuf::from(dir).join(tool),
        None => PathBuf::from(tool),
    };
    let mut command = Command::new(path);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command
}

fn python() -> Command {
    let mut command = Command::new(std::env::var("PACS_TEST_PYTHON").unwrap_or("python3".into()));
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command
}

async fn run(mut command: Command) -> Output {
    let shown = format!("{command:?}");
    tokio::task::spawn_blocking(move || command.output())
        .await
        .unwrap()
        .unwrap_or_else(|e| panic!("{shown} could not start ({e}); is the tool installed?"))
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Synthetic CT slices written as files, for storescu.
fn files(seed: u32, slices: u32) -> (tempfile::TempDir, Vec<PathBuf>, SyntheticSeries) {
    let dir = tempfile::tempdir().unwrap();
    let series = SyntheticSeries::new(seed, slices);
    let paths = (0..slices)
        .map(|s| {
            let path = dir.path().join(format!("{s}.dcm"));
            std::fs::write(&path, series.slice_bytes(s)).unwrap();
            path
        })
        .collect();
    (dir, paths, series)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Wait until something listens on `port`.
async fn listening(port: u16) {
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("nothing listens on port {port}");
}

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_echoscu_is_answered() {
    let addr = start(config(), archive().await).await;
    let mut echoscu = dcmtk("echoscu");
    echoscu.args([
        "-aet",
        "ECHOSCU",
        "-aec",
        "PACSRS",
        "127.0.0.1",
        &addr.port().to_string(),
    ]);
    let output = run(echoscu).await;
    assert!(output.status.success(), "{}", text(&output));
}

/// echoscu calling a title pacs-rs does not have is refused.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_echoscu_to_another_title_is_rejected() {
    let addr = start(config(), archive().await).await;
    let mut echoscu = dcmtk("echoscu");
    echoscu.args(["-aec", "NOT_US", "127.0.0.1", &addr.port().to_string()]);
    let output = run(echoscu).await;
    assert!(!output.status.success());
    assert!(
        text(&output).contains("Called AE Title Not Recognized"),
        "{}",
        text(&output)
    );
}

/// storescu sends a series; every instance is in the archive. Sent again, still Success.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_storescu_stores_a_series() {
    let archive = archive().await;
    let addr = start(config(), archive.clone()).await;
    let (_dir, paths, series) = files(71, 3);
    for _ in 0..2 {
        let mut storescu = dcmtk("storescu");
        storescu
            .args([
                "-aet",
                "STORESCU",
                "-aec",
                "PACSRS",
                "127.0.0.1",
                &addr.port().to_string(),
            ])
            .args(&paths);
        let output = run(storescu).await;
        assert!(output.status.success(), "{}", text(&output));
        assert!(
            !text(&output).contains("Status"),
            "a non-Success status: {}",
            text(&output)
        );
    }
    for slice in 0..3 {
        assert!(
            archive
                .index()
                .rest_instance_by_uid(&series.instance_uid(slice))
                .await
                .unwrap()
                .is_some()
        );
    }
}

/// storescu proposing Implicit VR Little Endian only (`-xi`): DCMTK re-encodes the
/// Explicit VR file, and it is stored as it arrived.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_storescu_in_implicit_vr_is_stored() {
    let archive = archive().await;
    let addr = start(config(), archive.clone()).await;
    let (_dir, paths, series) = files(72, 1);
    let mut storescu = dcmtk("storescu");
    storescu
        .args([
            "-xi",
            "-aec",
            "PACSRS",
            "127.0.0.1",
            &addr.port().to_string(),
        ])
        .args(&paths);
    let output = run(storescu).await;
    assert!(output.status.success(), "{}", text(&output));
    let entry = archive
        .index()
        .rest_instance_by_uid(&series.instance_uid(0))
        .await
        .unwrap()
        .unwrap();
    let stored = archive.file_at(&entry.object_key).await.unwrap();
    let instance = pacs_core::Instance::from_bytes(&stored).unwrap();
    assert_eq!(instance.key.transfer_syntax_uid, IMPLICIT_VR_LE);
}

/// pacs-rs's C-ECHO SCU reaches DCMTK's storescp.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn pacs_rs_echo_reaches_dcmtk_storescp() {
    let port = free_port();
    let out = tempfile::tempdir().unwrap();
    let mut storescp = dcmtk("storescp");
    storescp
        .args(["-aet", "STORESCP", "-od"])
        .arg(out.path())
        .arg(port.to_string());
    let _storescp = Killed(
        storescp
            .spawn()
            .expect("storescp could not start; is DCMTK installed?"),
    );
    listening(port).await;
    let remote: RemoteAe = format!("dcmtk=STORESCP@127.0.0.1:{port}").parse().unwrap();
    pacs_dimse::echo(
        &remote,
        &AeTitle::new("PACSRS").unwrap(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
}

/// A pynetdicom SCU: C-ECHO, then C-STORE of a series.
#[tokio::test]
#[ignore = "needs pynetdicom"]
async fn pynetdicom_echo_and_store() {
    let archive = archive().await;
    let addr = start(config(), archive.clone()).await;
    let (_dir, paths, series) = files(73, 2);
    let script = r#"
import sys
from pydicom import dcmread
from pynetdicom import AE
from pynetdicom.sop_class import Verification, CTImageStorage
port, files = int(sys.argv[1]), sys.argv[2:]
ae = AE(ae_title="PYNETDICOM")
ae.add_requested_context(Verification)
ae.add_requested_context(CTImageStorage)
assoc = ae.associate("127.0.0.1", port, ae_title="PACSRS")
assert assoc.is_established, "no association"
assert assoc.send_c_echo().Status == 0
for f in files:
    status = assoc.send_c_store(dcmread(f))
    assert status.Status == 0, hex(status.Status)
assoc.release()
"#;
    let mut py = python();
    py.args(["-c", script, &addr.port().to_string()])
        .args(&paths);
    let output = run(py).await;
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        archive
            .index()
            .rest_instance_by_uid(&series.instance_uid(1))
            .await
            .unwrap()
            .is_some()
    );
}

/// pacs-rs's C-ECHO SCU reaches a pynetdicom Verification SCP.
#[tokio::test]
#[ignore = "needs pynetdicom"]
async fn pacs_rs_echo_reaches_pynetdicom() {
    let port = free_port();
    let script = r#"
import sys
from pynetdicom import AE
from pynetdicom.sop_class import Verification
ae = AE(ae_title="PYSCP")
ae.add_supported_context(Verification)
ae.start_server(("127.0.0.1", int(sys.argv[1])), block=True)
"#;
    let mut py = python();
    py.args(["-c", script, &port.to_string()]);
    let _scp = Killed(py.spawn().expect("python could not start"));
    listening(port).await;
    let remote: RemoteAe = format!("py=PYSCP@127.0.0.1:{port}").parse().unwrap();
    pacs_dimse::echo(
        &remote,
        &AeTitle::new("PACSRS").unwrap(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
}
