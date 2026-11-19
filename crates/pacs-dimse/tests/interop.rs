//! pacs-rs against other implementations: DCMTK's echoscu, storescu, storescp and dcmqrscp
//! (C-FIND and C-MOVE), and pynetdicom. Ignored by default; with the tools installed:
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
use pacs_core::real::RealSeries;
use pacs_core::synthetic::SyntheticSeries;
use pacs_dimse::{AeTitle, RemoteAe, StudyQuery};

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

/// A port for a tool that is told where to listen. Not one the system gives out by itself
/// (a listener bound to port 0 here, a tool's outgoing connection), which could take it
/// between this check and the tool's start: one below that range, and a different one at
/// each call.
fn free_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(0);
    // 20000 to 29999 is under the ephemeral range of Linux, macOS and Windows; where this
    // process starts in it depends on its ID, for two runs at once
    let start = (std::process::id() % 5000) as u16 * 2;
    for _ in 0..10_000 {
        let port = 20_000 + (start + NEXT.fetch_add(1, Ordering::Relaxed)) % 10_000;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free port between 20000 and 29999");
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

/// A study as a file: one synthetic slice with this patient and study date.
fn study_file(seed: u32, patient_id: &str, name: &str, date: &str) -> (Vec<u8>, String) {
    use dicom_core::{DataElement, PrimitiveValue, VR};
    use dicom_dictionary_std::tags;
    let series = SyntheticSeries::new(seed, 1).in_study(seed, 1);
    let mut object = pacs_core::Instance::from_bytes(&series.slice_bytes(0))
        .unwrap()
        .object;
    for (tag, vr, value) in [
        (tags::PATIENT_ID, VR::LO, patient_id),
        (tags::PATIENT_NAME, VR::PN, name),
        (tags::STUDY_DATE, VR::DA, date),
    ] {
        object.put(DataElement::new(tag, vr, PrimitiveValue::from(value)));
    }
    let mut file = Vec::new();
    object.write_all(&mut file).unwrap();
    (file, series.study_uid())
}

/// DCMTK's dcmqrscp, called QRSCP, holding three studies and `extra` more, indexed with
/// dcmqridx; with `pacs_rs_port`, it knows PACSRS at that port as a move destination.
async fn dcmqrscp(
    extra: u32,
    pacs_rs_port: Option<u16>,
) -> (Killed, RemoteAe, tempfile::TempDir, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    let storage = dir.path().join("QRSCP");
    std::fs::create_dir(&storage).unwrap();
    let mut uids = Vec::new();
    let mut files = Vec::new();
    let many: Vec<_> = (0..extra)
        .map(|i| {
            (
                1000 + i,
                format!("X{i}"),
                "MANY^PATIENTS".to_owned(),
                "20250101",
            )
        })
        .collect();
    for (seed, patient, name, date) in [
        (81, "P1".to_owned(), "DOE^JOHN".to_owned(), "20260105"),
        (82, "P2".into(), "DOE^JANE".into(), "20260210"),
        (83, "P3".into(), "ROE^RICHARD".into(), "20260301"),
    ]
    .into_iter()
    .chain(many)
    {
        let (file, uid) = study_file(seed, &patient, &name, date);
        let path = storage.join(format!("{seed}.dcm"));
        std::fs::write(&path, file).unwrap();
        files.push(path);
        uids.push(uid);
    }
    let mut index = dcmtk("dcmqridx");
    index.arg(&storage).args(&files);
    let output = run(index).await;
    assert!(output.status.success(), "{}", text(&output));
    let port = free_port();
    let config = dir.path().join("dcmqrscp.cfg");
    std::fs::write(
        &config,
        format!(
            "NetworkTCPPort = {port}\nMaxPDUSize = 16384\nMaxAssociations = 16\n\
             HostTable BEGIN\n{}HostTable END\nVendorTable BEGIN\nVendorTable END\n\
             AETable BEGIN\nQRSCP {} RW (10, 64mb) ANY\nAETable END\n",
            pacs_rs_port
                .map(|p| format!("pacsrs = (PACSRS, 127.0.0.1, {p})\n"))
                .unwrap_or_default(),
            storage.display()
        ),
    )
    .unwrap();
    let mut scp = dcmtk("dcmqrscp");
    scp.arg("-c").arg(&config);
    let scp = Killed(
        scp.spawn()
            .expect("dcmqrscp could not start; is DCMTK installed?"),
    );
    listening(port).await;
    let remote = format!("dcmtk=QRSCP@127.0.0.1:{port}").parse().unwrap();
    (scp, remote, dir, uids)
}

async fn find_uids(
    remote: &RemoteAe,
    query: StudyQuery,
    stop_after: Option<usize>,
) -> (pacs_dimse::FindOutcome, Vec<String>) {
    let mut uids = Vec::new();
    let outcome = pacs_dimse::find_studies(
        remote,
        &AeTitle::new("PACSRS").unwrap(),
        &query,
        Duration::from_secs(5),
        |m| {
            let uid = m
                .element(dicom_dictionary_std::tags::STUDY_INSTANCE_UID)
                .unwrap();
            uids.push(uid.to_str().unwrap().trim_end_matches('\0').to_owned());
            if stop_after.is_some_and(|n| uids.len() >= n) {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        },
    )
    .await
    .unwrap();
    uids.sort();
    (outcome, uids)
}

/// dcmqrscp answers pacs-rs's C-FIND: by Patient ID, by a Study Date range, by a wild card
/// on Patient's Name, and with no key at all.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn pacs_rs_c_find_against_dcmtk_dcmqrscp() {
    let (_scp, remote, _dir, uids) = dcmqrscp(0, None).await;
    let by_id = StudyQuery {
        patient_id: Some("P2".into()),
        ..Default::default()
    };
    let (outcome, found) = find_uids(&remote, by_id, None).await;
    assert!(outcome.complete(), "{outcome:?}");
    assert_eq!(found, [uids[1].clone()]);

    let by_date = StudyQuery {
        study_date: Some("20260201-".parse().unwrap()),
        ..Default::default()
    };
    assert_eq!(
        find_uids(&remote, by_date, None).await.1,
        [uids[1].clone(), uids[2].clone()]
    );

    let by_name = StudyQuery {
        patient_name: Some("DOE*".into()),
        ..Default::default()
    };
    assert_eq!(
        find_uids(&remote, by_name, None).await.1,
        [uids[0].clone(), uids[1].clone()]
    );

    let (outcome, all) = find_uids(&remote, StudyQuery::default(), None).await;
    assert_eq!(all.len(), 3);
    assert_eq!(outcome.filtered_out, 0);
}

/// Stopping after the first of a hundred matches: C-CANCEL-FIND-RQ to dcmqrscp. dcmqrscp
/// 3.6.9 looks for a cancel only before its final response, so it ends with Cancel or, when
/// the cancel comes too late, Success; either way the matches after the first are dropped
/// and the association is released (PS3.4 C.4.1.2.1).
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn pacs_rs_c_cancel_against_dcmtk_dcmqrscp() {
    let (_scp, remote, _dir, _) = dcmqrscp(100, None).await;
    let (outcome, found) = find_uids(&remote, StudyQuery::default(), Some(1)).await;
    assert_eq!(found.len(), 1);
    assert!(outcome.cancelled && !outcome.complete(), "{outcome:?}");
    assert!(
        [pacs_dimse::Status::CANCEL, pacs_dimse::Status::SUCCESS].contains(&outcome.status),
        "{outcome:?}"
    );
}

const PYNETDICOM_QR: &str = r#"
import sys, time
from pydicom.dataset import Dataset
from pynetdicom import AE, evt
from pynetdicom.sop_class import StudyRootQueryRetrieveInformationModelFind
STUDIES = [("P1", "DOE^JOHN", "20260105", "2.25.11"),
           ("P2", "Benaïssa^Amel", "20260210", "2.25.12"),
           ("P3", "ROE^JANE", "20260301", "2.25.13")]
def handle(event):
    q = event.identifier
    assert q.QueryRetrieveLevel == "STUDY"
    sent = 0
    for pid, name, date, uid in STUDIES:
        if q.PatientID and q.PatientID != pid:
            continue
        if q.PatientName and str(q.PatientName) != name:
            continue
        if sent:
            time.sleep(0.3)
        if event.is_cancelled:
            yield (0xFE00, None)
            return
        ds = Dataset()
        ds.SpecificCharacterSet = "ISO_IR 192"
        ds.QueryRetrieveLevel = "STUDY"
        ds.PatientID, ds.PatientName, ds.StudyDate, ds.StudyInstanceUID = pid, name, date, uid
        sent += 1
        yield (0xFF00, ds)
ae = AE(ae_title="PYQR")
ae.add_supported_context(StudyRootQueryRetrieveInformationModelFind)
ae.start_server(("127.0.0.1", int(sys.argv[1])), block=True,
                evt_handlers=[(evt.EVT_C_FIND, handle)])
"#;

async fn pynetdicom_qr() -> (Killed, RemoteAe) {
    let port = free_port();
    let mut py = python();
    py.args(["-c", PYNETDICOM_QR, &port.to_string()]);
    let scp = Killed(py.spawn().expect("python could not start"));
    listening(port).await;
    (scp, format!("py=PYQR@127.0.0.1:{port}").parse().unwrap())
}

/// A pynetdicom query SCP: a UTF-8 Patient's Name goes out and comes back intact (Specific
/// Character Set ISO_IR 192 both ways), and a C-CANCEL ends the search.
#[tokio::test]
#[ignore = "needs pynetdicom"]
async fn pacs_rs_c_find_against_pynetdicom() {
    let (_scp, remote) = pynetdicom_qr().await;
    let by_name = StudyQuery {
        patient_name: Some("Benaïssa^Amel".into()),
        ..Default::default()
    };
    let (outcome, found) = find_uids(&remote, by_name, None).await;
    assert!(outcome.complete());
    assert_eq!(found, ["2.25.12"]);

    let (outcome, found) = find_uids(&remote, StudyQuery::default(), Some(1)).await;
    assert_eq!(found.len(), 1);
    assert!(outcome.cancelled, "{outcome:?}");
    assert_eq!(outcome.status, pacs_dimse::Status::CANCEL);
}

async fn move_to(
    remote: &RemoteAe,
    studies: &[String],
    destination: &str,
) -> Result<pacs_dimse::MoveOutcome, pacs_dimse::MoveError> {
    pacs_dimse::move_studies(
        remote,
        &AeTitle::new("PACSRS").unwrap(),
        studies,
        &AeTitle::new(destination).unwrap(),
        Duration::from_secs(5),
        Duration::from_secs(30),
        |_| std::ops::ControlFlow::Continue(()),
    )
    .await
}

/// The whole pull: pacs-rs asks dcmqrscp by C-MOVE to send a study to PACSRS, dcmqrscp
/// stores it into pacs-rs's C-STORE SCP, and the study is in the archive.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn pacs_rs_pulls_a_study_from_dcmtk_dcmqrscp() {
    let archive = archive().await;
    let addr = start(config(), archive.clone()).await;
    let (_scp, remote, _dir, uids) = dcmqrscp(0, Some(addr.port())).await;
    let outcome = move_to(&remote, &[uids[1].clone()], "PACSRS")
        .await
        .unwrap();
    assert_eq!(outcome.status, pacs_dimse::Status::SUCCESS, "{outcome:?}");
    assert_eq!(outcome.progress.completed, Some(1));
    let study = |patient: &str, uid: &str| pacs_core::public_ids::study(patient, uid);
    let index = archive.index();
    assert!(
        index
            .rest_study(&study("P2", &uids[1]))
            .await
            .unwrap()
            .is_some(),
        "moved"
    );
    assert!(
        index
            .rest_study(&study("P1", &uids[0]))
            .await
            .unwrap()
            .is_none(),
        "not asked for"
    );
}

/// A move destination dcmqrscp does not know: Refused, Move Destination unknown (A801).
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_dcmqrscp_refuses_an_unknown_move_destination() {
    let (_scp, remote, _dir, uids) = dcmqrscp(0, None).await;
    match move_to(&remote, &[uids[0].clone()], "PACSRS").await {
        Err(pacs_dimse::MoveError::Failed { status, .. }) => {
            assert_eq!(status, pacs_dimse::Status::MOVE_DESTINATION_UNKNOWN)
        }
        other => panic!("{other:?}"),
    }
}

const PYNETDICOM_MOVE: &str = r#"
import sys
from pydicom import dcmread
from pynetdicom import AE, evt, StoragePresentationContexts
from pynetdicom.sop_class import StudyRootQueryRetrieveInformationModelMove
port, destination_port, files = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3:]
def handle(event):
    destination = event.move_destination
    if isinstance(destination, bytes):
        destination = destination.decode()
    if destination.strip() != "PACSRS":
        yield (None, None)
        return
    yield ("127.0.0.1", destination_port)
    yield len(files)
    for f in files:
        if event.is_cancelled:
            yield (0xFE00, None)
            return
        yield (0xFF00, dcmread(f))
ae = AE(ae_title="PYQR")
ae.add_supported_context(StudyRootQueryRetrieveInformationModelMove)
ae.requested_contexts = StoragePresentationContexts
ae.start_server(("127.0.0.1", port), block=True, evt_handlers=[(evt.EVT_C_MOVE, handle)])
"#;

/// A pynetdicom move SCP sends two instances to pacs-rs's C-STORE SCP; pacs-rs counts them
/// as they go and finds them in the archive. To a destination it does not know, A801.
#[tokio::test]
#[ignore = "needs pynetdicom"]
async fn pacs_rs_pulls_from_pynetdicom() {
    let archive = archive().await;
    let addr = start(config(), archive.clone()).await;
    let (_files_dir, paths, series) = files(74, 2);
    let port = free_port();
    let mut py = python();
    py.args([
        "-c",
        PYNETDICOM_MOVE,
        &port.to_string(),
        &addr.port().to_string(),
    ])
    .args(&paths);
    let _scp = Killed(py.spawn().expect("python could not start"));
    listening(port).await;
    let remote: RemoteAe = format!("py=PYQR@127.0.0.1:{port}").parse().unwrap();
    let mut pendings = 0;
    let outcome = pacs_dimse::move_studies(
        &remote,
        &AeTitle::new("PACSRS").unwrap(),
        &[series.study_uid()],
        &AeTitle::new("PACSRS").unwrap(),
        Duration::from_secs(5),
        Duration::from_secs(30),
        |_| {
            pendings += 1;
            std::ops::ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.status, pacs_dimse::Status::SUCCESS, "{outcome:?}");
    assert_eq!(outcome.progress.completed, Some(2));
    assert!(pendings >= 1);
    for slice in 0..2 {
        assert!(
            archive
                .index()
                .rest_instance_by_uid(&series.instance_uid(slice))
                .await
                .unwrap()
                .is_some()
        );
    }
    match move_to(&remote, &[series.study_uid()], "ELSEWHERE").await {
        Err(pacs_dimse::MoveError::Failed { status, .. }) => {
            assert_eq!(status, pacs_dimse::Status::MOVE_DESTINATION_UNKNOWN)
        }
        other => panic!("{other:?}"),
    }
}

// ---- pacs-rs as a Query SCP (PS3.4 C.4.1.3), asked by DCMTK's findscu and by pynetdicom ----

/// A configuration that answers the queries of these AE titles.
fn answering(callers: &[&str]) -> pacs_dimse::Config {
    let mut config = config();
    config.remotes = callers
        .iter()
        .map(|title| {
            format!("{}={title}@127.0.0.1:104", title.to_lowercase())
                .parse()
                .unwrap()
        })
        .collect();
    config
}

/// Doe's CT of two series and Roe's MR, in `archive`.
async fn two_studies(archive: &pacs_archive::Archive) -> [RealSeries; 3] {
    let series = [
        RealSeries::new(401, 2)
            .with_patient("Doe^Jane", None)
            .at("20260105", "100000"),
        RealSeries::new(402, 3)
            .in_study(401, 2)
            .with_patient("Doe^Jane", None)
            .at("20260105", "100000"),
        RealSeries::new(403, 1)
            .with_patient("Roe^Richard", None)
            .at("20260106", "120000")
            .of_modality("MR"),
    ];
    for one in &series {
        ingest(archive, one).await;
    }
    series
}

/// findscu on the Study Root model, as FINDSCU, with `keys` (`-k` each) and `more`
/// options: its output, and the Identifiers it extracted, in the order they came.
async fn findscu(
    port: u16,
    calling: &str,
    keys: &[&str],
    more: &[&str],
) -> (Output, Vec<dicom_object::InMemDicomObject>) {
    findscu_on("-S", port, calling, keys, more).await
}

/// findscu on the model `model` names (`-S` Study Root, `-P` Patient Root).
async fn findscu_on(
    model: &str,
    port: u16,
    calling: &str,
    keys: &[&str],
    more: &[&str],
) -> (Output, Vec<dicom_object::InMemDicomObject>) {
    let dir = tempfile::tempdir().unwrap();
    let mut findscu = dcmtk("findscu");
    findscu.args(["-v", model, "-aet", calling, "-aec", "PACSRS", "-X", "-od"]);
    findscu.arg(dir.path());
    findscu.args(more);
    for key in keys {
        findscu.args(["-k", key]);
    }
    findscu.args(["127.0.0.1", &port.to_string()]);
    let output = run(findscu).await;
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    let found = files
        .iter()
        .map(|file| {
            dicom_object::open_file(file)
                .unwrap_or_else(|e| panic!("{}: {e}", file.display()))
                .into_inner()
        })
        .collect();
    (output, found)
}

fn values(found: &[dicom_object::InMemDicomObject], tag: dicom_core::Tag) -> Vec<String> {
    found
        .iter()
        .map(|identifier| value(identifier, tag).unwrap_or_default())
        .collect()
}

/// findscu searches studies by a wild card and a range, then goes down to a study's
/// series and a series' instances.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_findscu_queries_the_three_levels() {
    use dicom_dictionary_std::tags;
    let archive = archive().await;
    let [doe, report, roe] = two_studies(&archive).await;
    let addr = start(answering(&["FINDSCU"]), archive).await;

    let (output, found) = findscu(
        addr.port(),
        "FINDSCU",
        &[
            "QueryRetrieveLevel=STUDY",
            "PatientName=DOE*",
            "StudyDate=20260101-20260131",
            "StudyInstanceUID",
            "PatientID",
            "ModalitiesInStudy",
            "NumberOfStudyRelatedInstances",
        ],
        &[],
    )
    .await;
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(found.len(), 1, "{}", text(&output));
    for (tag, expected) in [
        (tags::STUDY_INSTANCE_UID, doe.study_uid()),
        (tags::PATIENT_NAME, "Doe^Jane".into()),
        (tags::PATIENT_ID, "NEMA-401".into()),
        (tags::STUDY_DATE, "20260105".into()),
        (tags::MODALITIES_IN_STUDY, "CT".into()),
        (tags::NUMBER_OF_STUDY_RELATED_INSTANCES, "5".into()),
        (tags::QUERY_RETRIEVE_LEVEL, "STUDY".into()),
        (tags::RETRIEVE_AE_TITLE, "PACSRS".into()),
    ] {
        assert_eq!(value(&found[0], tag), Some(expected), "{tag}");
    }

    // every study, then one that is not there
    let (output, found) = findscu(
        addr.port(),
        "FINDSCU",
        &[
            "QueryRetrieveLevel=STUDY",
            "StudyInstanceUID",
            "PatientName",
        ],
        &[],
    )
    .await;
    assert!(output.status.success(), "{}", text(&output));
    let mut names = values(&found, tags::PATIENT_NAME);
    names.sort();
    assert_eq!(names, ["Doe^Jane", "Roe^Richard"]);
    let (output, found) = findscu(
        addr.port(),
        "FINDSCU",
        &["QueryRetrieveLevel=STUDY", "PatientID=NOBODY"],
        &[],
    )
    .await;
    assert!(output.status.success(), "{}", text(&output));
    assert!(found.is_empty());

    let study = format!("StudyInstanceUID={}", doe.study_uid());
    let (output, found) = findscu(
        addr.port(),
        "FINDSCU",
        &[
            "QueryRetrieveLevel=SERIES",
            &study,
            "SeriesInstanceUID",
            "Modality",
            "SeriesNumber",
            "NumberOfSeriesRelatedInstances",
        ],
        &[],
    )
    .await;
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(
        values(&found, tags::SERIES_INSTANCE_UID),
        [doe.series_uid(), report.series_uid()]
    );
    assert_eq!(
        values(&found, tags::NUMBER_OF_SERIES_RELATED_INSTANCES),
        ["2", "3"]
    );

    let series = format!("SeriesInstanceUID={}", report.series_uid());
    let (output, found) = findscu(
        addr.port(),
        "FINDSCU",
        &[
            "QueryRetrieveLevel=IMAGE",
            &study,
            &series,
            "SOPInstanceUID",
            "InstanceNumber",
            "Rows",
        ],
        &[],
    )
    .await;
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(
        values(&found, tags::SOP_INSTANCE_UID),
        [0, 1, 2].map(|slice| report.instance_uid(slice))
    );
    assert_eq!(values(&found, tags::INSTANCE_NUMBER), ["1", "2", "3"]);
    // a series of another study is not found under this one
    let other = format!("StudyInstanceUID={}", roe.study_uid());
    let (output, found) = findscu(
        addr.port(),
        "FINDSCU",
        &[
            "QueryRetrieveLevel=IMAGE",
            &other,
            &series,
            "SOPInstanceUID",
        ],
        &[],
    )
    .await;
    assert!(output.status.success(), "{}", text(&output));
    assert!(found.is_empty());
}

/// findscu is told what is wrong: an AE title pacs-rs does not answer, and an Identifier
/// without the Unique Key of the level above.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_findscu_is_refused_with_a_status_it_shows() {
    let archive = archive().await;
    two_studies(&archive).await;
    let addr = start(answering(&["FINDSCU"]), archive).await;
    let (output, found) = findscu(
        addr.port(),
        "STRANGER",
        &["QueryRetrieveLevel=STUDY", "PatientName"],
        &[],
    )
    .await;
    assert!(found.is_empty(), "{}", text(&output));
    // a status DCMTK has no name for
    assert!(text(&output).contains("Status: 0x124"), "{}", text(&output));

    let (output, found) = findscu(
        addr.port(),
        "FINDSCU",
        &["QueryRetrieveLevel=SERIES", "SeriesInstanceUID"],
        &[],
    )
    .await;
    assert!(found.is_empty(), "{}", text(&output));
    assert!(
        text(&output).contains("DataSetDoesNotMatchSOPClass"),
        "{}",
        text(&output)
    );
}

/// findscu cancels after the first match: the search ends with status Cancel, short of
/// its matches. The server is held until the cancel has left findscu ([`gate`]).
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_findscu_cancels_in_the_middle() {
    const STUDIES: u32 = 2500;
    let archive = archive().await;
    for seed in 0..STUDIES {
        index_only(&archive, &RealSeries::new(30_000 + seed, 1)).await;
    }
    let server = start(answering(&["FINDSCU"]), archive).await;
    let addr = gate(server, 2048, 2).await;
    let (output, found) = findscu(
        addr.port(),
        "FINDSCU",
        &["QueryRetrieveLevel=STUDY", "StudyInstanceUID", "PatientID"],
        &["--cancel", "1"],
    )
    .await;
    let said = text(&output);
    assert!(said.contains("Cancel"), "{said}");
    assert!(
        !found.is_empty() && found.len() < STUDIES as usize,
        "{}",
        found.len()
    );
}

/// pynetdicom queries the three levels, with a name in UTF-8, and under an AE title
/// pacs-rs does not answer it gets Refused: Not authorized.
#[tokio::test]
#[ignore = "needs pynetdicom"]
async fn pynetdicom_queries_the_three_levels() {
    let archive = archive().await;
    let [doe, report, _] = two_studies(&archive).await;
    ingest(
        &archive,
        &RealSeries::new(404, 1).with_patient("Benaïssa^Amel", Some("ISO_IR 192")),
    )
    .await;
    let addr = start(answering(&["PYNETDICOM"]), archive).await;
    let script = r#"
import sys
from pydicom.dataset import Dataset
from pynetdicom import AE
from pynetdicom.sop_class import StudyRootQueryRetrieveInformationModelFind as FIND
port, study, series, second = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]

def find(assoc, **keys):
    ds = Dataset()
    for name, value in keys.items():
        setattr(ds, name, value)
    found, last = [], None
    for status, identifier in assoc.send_c_find(ds, FIND):
        assert status, "no response"
        if status.Status in (0xFF00, 0xFF01):
            found.append((status.Status, identifier))
        else:
            last = status.Status
    return found, last

ae = AE(ae_title="PYNETDICOM")
ae.add_requested_context(FIND)
assoc = ae.associate("127.0.0.1", port, ae_title="PACSRS")
assert assoc.is_established, "no association"

found, last = find(assoc, QueryRetrieveLevel="STUDY", PatientName="doe^*", StudyInstanceUID="",
                   StudyDate="", ModalitiesInStudy="", NumberOfStudyRelatedSeries="")
assert last == 0x0000, hex(last)
assert [s for s, _ in found] == [0xFF00], found
ds = found[0][1]
assert ds.StudyInstanceUID == study and str(ds.PatientName) == "Doe^Jane", ds
assert ds.StudyDate == "20260105" and ds.NumberOfStudyRelatedSeries == 2, ds
assert ds.ModalitiesInStudy == "CT" and ds.RetrieveAETitle == "PACSRS", ds

# a name outside ASCII, asked and answered in UTF-8
ds = Dataset()
ds.SpecificCharacterSet = "ISO_IR 192"
ds.QueryRetrieveLevel = "STUDY"
ds.PatientName = "Benaïssa*"
ds.StudyInstanceUID = ""
names = [str(i.PatientName) for s, i in assoc.send_c_find(ds, FIND) if s.Status == 0xFF00]
assert names == ["Benaïssa^Amel"], names

# a key pacs-rs does not have: left out, and the status says so
found, last = find(assoc, QueryRetrieveLevel="STUDY", PatientID="NEMA-401", StudyInstanceUID="",
                   AdmittingDiagnosesDescription="")
assert last == 0x0000 and [s for s, _ in found] == [0xFF01], (found, last)
assert "AdmittingDiagnosesDescription" not in found[0][1], found[0][1]

found, last = find(assoc, QueryRetrieveLevel="SERIES", StudyInstanceUID=study,
                   SeriesInstanceUID="", Modality="", SeriesNumber="")
assert last == 0x0000, hex(last)
assert [i.SeriesInstanceUID for _, i in found] == [series, second], found
assert [i.SeriesNumber for _, i in found] == [1, 2], found

found, last = find(assoc, QueryRetrieveLevel="IMAGE", StudyInstanceUID=study,
                   SeriesInstanceUID=second, SOPInstanceUID="", InstanceNumber="", Rows=None)
assert last == 0x0000, hex(last)
assert [i.InstanceNumber for _, i in found] == [1, 2, 3], found
assert all(i.Rows == 128 and i.StudyInstanceUID == study for _, i in found), found

# a SERIES query without its study: the Identifier does not match the SOP class
found, last = find(assoc, QueryRetrieveLevel="SERIES", SeriesInstanceUID="")
assert last == 0xA900 and not found, (found, last)
assoc.release()

stranger = AE(ae_title="STRANGER")
stranger.add_requested_context(FIND)
assoc = stranger.associate("127.0.0.1", port, ae_title="PACSRS")
assert assoc.is_established, "no association"
found, last = find(assoc, QueryRetrieveLevel="STUDY", StudyInstanceUID="")
assert last == 0x0124 and not found, (found, last)
assoc.release()
"#;
    let mut py = python();
    py.args([
        "-c",
        script,
        &addr.port().to_string(),
        &doe.study_uid(),
        &doe.series_uid(),
        &report.series_uid(),
    ]);
    let output = run(py).await;
    assert!(output.status.success(), "{}", text(&output));
}

// ---- pacs-rs as a Storage SCU (PS3.4 Annex B), sending to DCMTK's storescp and to pynetdicom ----

/// A series ingested into an archive that compresses (JPEG-LS lossless), with what the
/// index says of its instances and their stored files.
async fn compressed_series(
    seed: u32,
    slices: u32,
) -> (RealSeries, Vec<(pacs_index::InstanceEntry, bytes::Bytes)>) {
    let archive = pacs_archive::Archive::new(
        pacs_store::Store::memory(),
        pacs_index::Index::in_memory().await.unwrap(),
    );
    let series = RealSeries::new(seed, slices);
    ingest(&archive, &series).await;
    let entries = archive
        .index()
        .instances(&series.study_uid(), &series.series_uid(), None)
        .await
        .unwrap();
    let mut files = Vec::new();
    for entry in entries {
        assert_eq!(entry.transfer_syntax_uid, "1.2.840.10008.1.2.4.80");
        let file = archive.file(&entry).await.unwrap();
        files.push((entry, file));
    }
    (series, files)
}

/// Send `files` to `remote` as PACSRS; every one must be stored. The transfer syntaxes
/// the peer accepted.
async fn send_series(
    remote: &RemoteAe,
    files: &[(pacs_index::InstanceEntry, bytes::Bytes)],
) -> Vec<String> {
    let stored: Vec<(String, String)> = files
        .iter()
        .map(|(entry, _)| {
            (
                entry.sop_class_uid.clone(),
                entry.transfer_syntax_uid.clone(),
            )
        })
        .collect();
    let mut sender = pacs_dimse::Sender::open(
        remote,
        &AeTitle::new("PACSRS").unwrap(),
        &stored,
        Duration::from_secs(10),
        None,
    )
    .await
    .unwrap();
    let accepted = sender
        .accepted()
        .iter()
        .map(|context| context.transfer_syntax.clone())
        .collect();
    for (entry, file) in files {
        let status = sender
            .send(
                file.clone(),
                &entry.sop_class_uid,
                &entry.sop_instance_uid,
                &entry.transfer_syntax_uid,
            )
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", entry.sop_instance_uid));
        assert_eq!(status, pacs_dimse::Status::SUCCESS);
    }
    sender.release().await;
    accepted
}

/// The files a peer wrote in `dir`, once there are `count` of them, by SOP Instance UID:
/// each one's transfer syntax and, when it is not compressed, its pixels as 16-bit values.
async fn written(dir: &std::path::Path, count: usize) -> Vec<(String, String, Vec<u16>)> {
    use dicom_dictionary_std::tags;
    let mut paths = Vec::new();
    for _ in 0..100 {
        paths = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| !path.to_string_lossy().ends_with(".tmp"))
            .collect::<Vec<_>>();
        if paths.len() >= count {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(paths.len(), count, "{paths:?}");
    let mut files: Vec<(String, String, Vec<u16>)> = paths
        .iter()
        .map(|path| {
            let object =
                dicom_object::open_file(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            // none for pixel data that is in fragments: a compressed file
            let pixels = object
                .element(tags::PIXEL_DATA)
                .unwrap()
                .to_bytes()
                .map(|bytes| {
                    bytes
                        .chunks(2)
                        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                        .collect()
                })
                .unwrap_or_default();
            (
                value(&object, tags::SOP_INSTANCE_UID).unwrap(),
                object
                    .meta()
                    .transfer_syntax()
                    .trim_end_matches('\0')
                    .to_owned(),
                pixels,
            )
        })
        .collect();
    files.sort();
    files
}

/// storescp as it comes takes uncompressed transfer syntaxes only: a series stored in
/// JPEG-LS is decoded on the way, and the files storescp writes have the original pixels.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn pacs_rs_sends_a_compressed_series_to_dcmtk_storescp() {
    let port = free_port();
    let out = tempfile::tempdir().unwrap();
    let mut storescp = dcmtk("storescp");
    storescp
        .args(["-aet", "STORESCP", "-od"])
        .arg(out.path())
        .arg(port.to_string());
    let _storescp = Killed(storescp.spawn().expect("storescp could not start"));
    listening(port).await;
    let remote: RemoteAe = format!("dcmtk=STORESCP@127.0.0.1:{port}").parse().unwrap();
    let (series, files) = compressed_series(81, 3).await;
    let accepted = send_series(&remote, &files).await;
    assert_eq!(
        accepted,
        ["1.2.840.10008.1.2.1"],
        "the stored syntax was not taken"
    );
    let written = written(out.path(), 3).await;
    for (slice, (uid, transfer_syntax, pixels)) in written.iter().enumerate() {
        assert_eq!(*uid, series.instance_uid(slice as u32));
        assert_eq!(transfer_syntax, "1.2.840.10008.1.2.1");
        assert_eq!(*pixels, series.pixels(slice as u32, 0));
    }
}

/// storescp told to prefer JPEG-LS lossless takes the stored transfer syntax: the files
/// it writes are in it, and DCMTK's own dcmdjpls decodes them to the original pixels.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn pacs_rs_sends_a_series_as_stored_to_dcmtk_storescp() {
    let port = free_port();
    let out = tempfile::tempdir().unwrap();
    let mut storescp = dcmtk("storescp");
    storescp
        .args(["-aet", "STORESCP", "+xt", "-od"])
        .arg(out.path())
        .arg(port.to_string());
    let _storescp = Killed(storescp.spawn().expect("storescp could not start"));
    listening(port).await;
    let remote: RemoteAe = format!("dcmtk=STORESCP@127.0.0.1:{port}").parse().unwrap();
    let (series, files) = compressed_series(82, 2).await;
    let accepted = send_series(&remote, &files).await;
    assert!(
        accepted.iter().any(|ts| ts == "1.2.840.10008.1.2.4.80"),
        "{accepted:?}"
    );
    let decoded = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(out.path()).unwrap() {
        let path = entry.unwrap().path();
        let object = dicom_object::open_file(&path).unwrap();
        assert_eq!(
            object.meta().transfer_syntax().trim_end_matches('\0'),
            "1.2.840.10008.1.2.4.80"
        );
        let mut dcmdjpls = dcmtk("dcmdjpls");
        dcmdjpls
            .arg(&path)
            .arg(decoded.path().join(path.file_name().unwrap()));
        let output = run(dcmdjpls).await;
        assert!(output.status.success(), "{}", text(&output));
    }
    let written = written(decoded.path(), 2).await;
    for (slice, (uid, _, pixels)) in written.iter().enumerate() {
        assert_eq!(*uid, series.instance_uid(slice as u32));
        assert_eq!(*pixels, series.pixels(slice as u32, 0));
    }
}

/// A pynetdicom Storage SCP that takes CT in Implicit VR Little Endian only: the series
/// is decoded and written again for it, and pydicom reads the original pixels.
#[tokio::test]
#[ignore = "needs pynetdicom"]
async fn pacs_rs_sends_a_compressed_series_to_pynetdicom() {
    let port = free_port();
    let out = tempfile::tempdir().unwrap();
    let script = r#"
import sys, os
from pynetdicom import AE, evt
from pynetdicom.sop_class import CTImageStorage
from pydicom.uid import ImplicitVRLittleEndian
port, out = int(sys.argv[1]), sys.argv[2]

def on_store(event):
    assert event.context.transfer_syntax == ImplicitVRLittleEndian
    ds = event.dataset
    ds.file_meta = event.file_meta
    assert ds.Rows == 128 and ds.Columns == 128, (ds.Rows, ds.Columns)
    # the pixels as pydicom reads them, 16 bits little endian, for the test to compare
    raw = ds.PixelData
    values = [int.from_bytes(raw[i:i + 2], "little") for i in range(0, len(raw), 2)]
    with open(os.path.join(out, ds.SOPInstanceUID + ".txt.tmp"), "w") as f:
        f.write(" ".join(str(v) for v in values))
    os.rename(os.path.join(out, ds.SOPInstanceUID + ".txt.tmp"),
              os.path.join(out, ds.SOPInstanceUID + ".txt"))
    return 0x0000

ae = AE(ae_title="PYSTORE")
ae.add_supported_context(CTImageStorage, ImplicitVRLittleEndian)
ae.start_server(("127.0.0.1", port), block=True, evt_handlers=[(evt.EVT_C_STORE, on_store)])
"#;
    let mut py = python();
    py.args(["-c", script, &port.to_string()]).arg(out.path());
    let _scp = Killed(py.spawn().expect("python could not start"));
    listening(port).await;
    let remote: RemoteAe = format!("py=PYSTORE@127.0.0.1:{port}").parse().unwrap();
    let (series, files) = compressed_series(83, 2).await;
    let accepted = send_series(&remote, &files).await;
    assert_eq!(accepted, ["1.2.840.10008.1.2"]);
    for slice in 0..2 {
        let path = out
            .path()
            .join(format!("{}.txt", series.instance_uid(slice)));
        let read =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let pixels: Vec<u16> = read.split(' ').map(|v| v.parse().unwrap()).collect();
        assert_eq!(pixels, series.pixels(slice, 0));
    }
}

// ---- pacs-rs as a Retrieve SCP (PS3.4 C.4.2.3), asked by DCMTK's movescu and by pynetdicom ----

/// An archive that compresses (JPEG-LS lossless) with Doe's study in it: a series of two
/// slices and one of three.
async fn archive_to_retrieve_from() -> (pacs_archive::Archive, [RealSeries; 2]) {
    let archive = pacs_archive::Archive::new(
        pacs_store::Store::memory(),
        pacs_index::Index::in_memory().await.unwrap(),
    );
    let series = [
        RealSeries::new(91, 2).with_patient("Doe^Jane", None),
        RealSeries::new(92, 3)
            .in_study(91, 2)
            .with_patient("Doe^Jane", None),
    ];
    for one in &series {
        ingest(&archive, one).await;
    }
    (archive, series)
}

/// A configuration that answers `caller` and sends to `destination` on `port` here.
fn retrieving(caller: &str, destination: &str, port: u16) -> pacs_dimse::Config {
    let mut config = answering(&[caller]);
    config
        .remotes
        .retain(|remote| !remote.ae_title.matches(destination));
    config.remotes.push(
        format!("destination={destination}@127.0.0.1:{port}")
            .parse()
            .unwrap(),
    );
    config.move_destinations = vec!["destination".into()];
    config
}

/// The pixels each instance of `series` must have, by SOP Instance UID, sorted.
fn expected_pixels(series: &[RealSeries]) -> Vec<(String, Vec<u16>)> {
    let mut expected: Vec<(String, Vec<u16>)> = series
        .iter()
        .flat_map(|one| {
            (0..one.slices).map(|slice| (one.instance_uid(slice), one.pixels(slice, 0)))
        })
        .collect();
    expected.sort();
    expected
}

/// movescu asks pacs-rs to send a study to movescu's own Storage SCP, which takes
/// uncompressed transfer syntaxes: every instance arrives, decoded, with its pixels.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_movescu_pulls_a_study_from_pacs_rs() {
    let (archive, series) = archive_to_retrieve_from().await;
    let receiving = free_port();
    let addr = start(retrieving("MOVESCU", "MOVESCU", receiving), archive).await;
    let out = tempfile::tempdir().unwrap();
    let mut movescu = dcmtk("movescu");
    movescu
        .args([
            "-v", "-S", "-aet", "MOVESCU", "-aec", "PACSRS", "-aem", "MOVESCU",
        ])
        .args(["+P", &receiving.to_string(), "-od"])
        .arg(out.path())
        .args(["-k", "QueryRetrieveLevel=STUDY", "-k"])
        .arg(format!("StudyInstanceUID={}", series[0].study_uid()))
        .args(["127.0.0.1", &addr.port().to_string()]);
    let output = run(movescu).await;
    assert!(output.status.success(), "{}", text(&output));
    let said = text(&output);
    assert!(
        said.contains("Received Final Move Response (Success)"),
        "{said}"
    );
    let written: Vec<(String, Vec<u16>)> = written(out.path(), 5)
        .await
        .into_iter()
        .map(|(uid, _, pixels)| (uid, pixels))
        .collect();
    assert_eq!(written, expected_pixels(&series));
}

/// movescu names a third AE as the destination, DCMTK's storescp: the series asked for
/// goes there, and only it.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_movescu_sends_a_series_to_dcmtk_storescp() {
    let (archive, series) = archive_to_retrieve_from().await;
    let port = free_port();
    let out = tempfile::tempdir().unwrap();
    let mut storescp = dcmtk("storescp");
    storescp
        .args(["-aet", "STORESCP", "-od"])
        .arg(out.path())
        .arg(port.to_string());
    let _storescp = Killed(storescp.spawn().expect("storescp could not start"));
    listening(port).await;
    let addr = start(retrieving("MOVESCU", "STORESCP", port), archive).await;
    let mut movescu = dcmtk("movescu");
    movescu
        .args([
            "-v", "-S", "-aet", "MOVESCU", "-aec", "PACSRS", "-aem", "STORESCP",
        ])
        .args(["-k", "QueryRetrieveLevel=SERIES", "-k"])
        .arg(format!("StudyInstanceUID={}", series[1].study_uid()))
        .arg("-k")
        .arg(format!("SeriesInstanceUID={}", series[1].series_uid()))
        .args(["127.0.0.1", &addr.port().to_string()]);
    let output = run(movescu).await;
    assert!(output.status.success(), "{}", text(&output));
    let written: Vec<(String, Vec<u16>)> = written(out.path(), 3)
        .await
        .into_iter()
        .map(|(uid, _, pixels)| (uid, pixels))
        .collect();
    assert_eq!(written, expected_pixels(&series[1..]));
}

/// movescu is told what is wrong: a destination pacs-rs does not send to (A801), and a
/// destination that does not answer (A702).
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_movescu_is_refused_with_a_status_it_shows() {
    let (archive, series) = archive_to_retrieve_from().await;
    // nothing listens where the destination is said to be
    let addr = start(retrieving("MOVESCU", "STORESCP", free_port()), archive).await;
    let ask = async |destination: &str| {
        let mut movescu = dcmtk("movescu");
        movescu
            .args([
                "-v",
                "-S",
                "-aet",
                "MOVESCU",
                "-aec",
                "PACSRS",
                "-aem",
                destination,
            ])
            .args(["-k", "QueryRetrieveLevel=STUDY", "-k"])
            .arg(format!("StudyInstanceUID={}", series[0].study_uid()))
            .args(["127.0.0.1", &addr.port().to_string()]);
        text(&run(movescu).await)
    };
    let said = ask("NOBODY").await;
    assert!(said.contains("MoveDestinationUnknown"), "{said}");
    let said = ask("STORESCP").await;
    assert!(said.contains("OutOfResourcesSubOperations"), "{said}");
}

/// pynetdicom asks pacs-rs to send a study to its own Storage SCP and follows the counts
/// of the Pending responses; then an unknown destination, and an AE title pacs-rs does
/// not answer.
#[tokio::test]
#[ignore = "needs pynetdicom"]
async fn pynetdicom_pulls_a_study_from_pacs_rs() {
    let (archive, series) = archive_to_retrieve_from().await;
    let receiving = free_port();
    let addr = start(retrieving("PYNETDICOM", "PYNETDICOM", receiving), archive).await;
    let script = r#"
import sys
from pydicom.dataset import Dataset
from pydicom.uid import ImplicitVRLittleEndian, ExplicitVRLittleEndian
from pynetdicom import AE, evt
from pynetdicom.sop_class import CTImageStorage
from pynetdicom.sop_class import StudyRootQueryRetrieveInformationModelMove as MOVE
port, receiving, study = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]

stored = {}
def on_store(event):
    ds = event.dataset
    raw = ds.PixelData
    stored[ds.SOPInstanceUID] = (
        str(ds.PatientName),
        event.request.MoveOriginatorApplicationEntityTitle,
        [int.from_bytes(raw[i:i + 2], "little") for i in range(0, len(raw), 2)],
    )
    return 0x0000

ae = AE(ae_title="PYNETDICOM")
ae.add_supported_context(CTImageStorage, [ExplicitVRLittleEndian, ImplicitVRLittleEndian])
ae.add_requested_context(MOVE)
scp = ae.start_server(("127.0.0.1", receiving), block=False,
                      evt_handlers=[(evt.EVT_C_STORE, on_store)])

def move(assoc, destination, uid):
    ds = Dataset()
    ds.QueryRetrieveLevel = "STUDY"
    ds.StudyInstanceUID = uid
    return [(s.Status, s) for s, _ in assoc.send_c_move(ds, destination, MOVE)]

assoc = ae.associate("127.0.0.1", port, ae_title="PACSRS")
assert assoc.is_established, "no association"
responses = move(assoc, "PYNETDICOM", study)
pending = [s for status, s in responses if status == 0xFF00]
assert [s.NumberOfRemainingSuboperations for s in pending] == [4, 3, 2, 1], responses
assert [s.NumberOfCompletedSuboperations for s in pending] == [1, 2, 3, 4], responses
status, last = responses[-1]
assert status == 0x0000, hex(status)
assert (last.NumberOfCompletedSuboperations, last.NumberOfFailedSuboperations,
        last.NumberOfWarningSuboperations) == (5, 0, 0), last
assert len(stored) == 5, stored.keys()
for uid, (name, originator, pixels) in sorted(stored.items()):
    assert name == "Doe^Jane" and originator.strip() == "PYNETDICOM", (name, originator)
    print(uid, " ".join(str(v) for v in pixels))

# a destination pacs-rs does not send to, and a study it does not have
responses = move(assoc, "NOBODY", study)
assert [status for status, _ in responses] == [0xA801], responses
responses = move(assoc, "PYNETDICOM", "2.25.404")
status, last = responses[-1]
assert status == 0x0000 and last.NumberOfCompletedSuboperations == 0, responses
assoc.release()

stranger = AE(ae_title="STRANGER")
stranger.add_requested_context(MOVE)
assoc = stranger.associate("127.0.0.1", port, ae_title="PACSRS")
assert assoc.is_established, "no association"
responses = move(assoc, "PYNETDICOM", study)
assert [status for status, _ in responses] == [0x0124], responses
assoc.release()
scp.shutdown()
"#;
    let mut py = python();
    py.args([
        "-c",
        script,
        &addr.port().to_string(),
        &receiving.to_string(),
        &series[0].study_uid(),
    ]);
    let output = run(py).await;
    assert!(output.status.success(), "{}", text(&output));
    let printed: Vec<(String, Vec<u16>)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| {
            let mut words = line.split(' ');
            let uid = words.next().unwrap().to_owned();
            (uid, words.map(|v| v.parse().unwrap()).collect())
        })
        .collect();
    assert_eq!(printed, expected_pixels(&series));
}

// ---- pacs-rs as a Retrieve SCP for C-GET (PS3.4 C.4.3.3), asked by DCMTK's getscu and by pynetdicom ----

/// getscu on the Study Root model as `calling`, with `keys`: its output and what it wrote.
async fn getscu(
    port: u16,
    calling: &str,
    keys: &[String],
    more: &[&str],
    expect: usize,
) -> (String, Vec<(String, String, Vec<u16>)>) {
    let out = tempfile::tempdir().unwrap();
    let mut getscu = dcmtk("getscu");
    getscu
        .args(["-v", "-S", "-aet", calling, "-aec", "PACSRS", "-od"])
        .arg(out.path())
        .args(more);
    for key in keys {
        getscu.args(["-k", key]);
    }
    getscu.args(["127.0.0.1", &port.to_string()]);
    let output = run(getscu).await;
    (text(&output), written(out.path(), expect).await)
}

/// getscu retrieves a study on its own association, taking the SCP role for the storage
/// SOP classes; stored in JPEG-LS, the instances are decoded for it, or sent as stored
/// when it prefers JPEG-LS. Then a series alone.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_getscu_retrieves_from_pacs_rs() {
    let (archive, series) = archive_to_retrieve_from().await;
    let addr = start(answering(&["GETSCU"]), archive).await;
    let study = vec![
        "QueryRetrieveLevel=STUDY".to_owned(),
        format!("StudyInstanceUID={}", series[0].study_uid()),
    ];
    let (said, files) = getscu(addr.port(), "GETSCU", &study, &[], 5).await;
    assert!(
        said.contains("Received Final C-GET Response") || said.contains("Success"),
        "{said}"
    );
    let pixels: Vec<(String, Vec<u16>)> = files
        .iter()
        .map(|(uid, _, pixels)| (uid.clone(), pixels.clone()))
        .collect();
    assert_eq!(pixels, expected_pixels(&series), "{said}");
    assert!(files.iter().all(|(_, ts, _)| ts == "1.2.840.10008.1.2.1"));

    // preferring JPEG-LS lossless, it gets the files as they are stored
    let (said, files) = getscu(addr.port(), "GETSCU", &study, &["+xt"], 5).await;
    assert!(
        files
            .iter()
            .all(|(_, ts, _)| ts == "1.2.840.10008.1.2.4.80"),
        "{said}"
    );

    let one = vec![
        "QueryRetrieveLevel=SERIES".to_owned(),
        format!("StudyInstanceUID={}", series[1].study_uid()),
        format!("SeriesInstanceUID={}", series[1].series_uid()),
    ];
    let (said, files) = getscu(addr.port(), "GETSCU", &one, &[], 3).await;
    let pixels: Vec<(String, Vec<u16>)> = files
        .iter()
        .map(|(uid, _, pixels)| (uid.clone(), pixels.clone()))
        .collect();
    assert_eq!(pixels, expected_pixels(&series[1..]), "{said}");
}

/// getscu under an AE title pacs-rs does not answer is refused, and gets nothing.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_getscu_is_refused_with_a_status_it_shows() {
    let (archive, series) = archive_to_retrieve_from().await;
    let addr = start(answering(&["GETSCU"]), archive).await;
    let study = vec![
        "QueryRetrieveLevel=STUDY".to_owned(),
        format!("StudyInstanceUID={}", series[0].study_uid()),
    ];
    let (said, files) = getscu(addr.port(), "STRANGER", &study, &[], 0).await;
    assert!(files.is_empty());
    assert!(said.contains("0x0124") || said.contains("0x124"), "{said}");
}

/// pynetdicom retrieves a study with C-GET, taking the SCP role for CT; without the role
/// nothing can be sent to it and the retrieval fails; under an AE title pacs-rs does not
/// answer it is refused.
#[tokio::test]
#[ignore = "needs pynetdicom"]
async fn pynetdicom_gets_a_study_from_pacs_rs() {
    let (archive, series) = archive_to_retrieve_from().await;
    let addr = start(answering(&["PYNETDICOM"]), archive).await;
    let script = r#"
import sys
from pydicom.dataset import Dataset
from pydicom.uid import ImplicitVRLittleEndian
from pynetdicom import AE, evt, build_role
from pynetdicom.sop_class import CTImageStorage
from pynetdicom.sop_class import StudyRootQueryRetrieveInformationModelGet as GET
port, study = int(sys.argv[1]), sys.argv[2]

stored = {}
def on_store(event):
    assert event.context.transfer_syntax == ImplicitVRLittleEndian
    ds = event.dataset
    raw = ds.PixelData
    stored[ds.SOPInstanceUID] = (
        str(ds.PatientName),
        [int.from_bytes(raw[i:i + 2], "little") for i in range(0, len(raw), 2)],
    )
    return 0x0000

def get(assoc, uid):
    ds = Dataset()
    ds.QueryRetrieveLevel = "STUDY"
    ds.StudyInstanceUID = uid
    return [(s.Status, s) for s, _ in assoc.send_c_get(ds, GET)]

def associate(title, roles):
    ae = AE(ae_title=title)
    ae.add_requested_context(GET)
    ae.add_requested_context(CTImageStorage, ImplicitVRLittleEndian)
    assoc = ae.associate("127.0.0.1", port, ae_title="PACSRS", ext_neg=roles,
                         evt_handlers=[(evt.EVT_C_STORE, on_store)])
    assert assoc.is_established, "no association"
    return assoc

assoc = associate("PYNETDICOM", [build_role(CTImageStorage, scp_role=True)])
responses = get(assoc, study)
pending = [s for status, s in responses if status == 0xFF00]
assert [s.NumberOfRemainingSuboperations for s in pending] == [4, 3, 2, 1], responses
status, last = responses[-1]
assert status == 0x0000, hex(status)
assert (last.NumberOfCompletedSuboperations, last.NumberOfFailedSuboperations,
        last.NumberOfWarningSuboperations) == (5, 0, 0), last
assert len(stored) == 5, stored.keys()
for uid, (name, pixels) in sorted(stored.items()):
    assert name == "Doe^Jane", name
    print(uid, " ".join(str(v) for v in pixels))
# a study that is not there: nothing to send, no failure
status, last = get(assoc, "2.25.404")[-1]
assert status == 0x0000 and last.NumberOfCompletedSuboperations == 0, last
assoc.release()

# without the SCP role nothing can be sent: every sub-operation fails
stored.clear()
assoc = associate("PYNETDICOM", [])
status, last = get(assoc, study)[-1]
assert status == 0xA702, hex(status)
assert last.NumberOfFailedSuboperations == 5 and not stored, last
assoc.release()

assoc = associate("STRANGER", [build_role(CTImageStorage, scp_role=True)])
responses = get(assoc, study)
assert [status for status, _ in responses] == [0x0124], responses
assoc.release()
"#;
    let mut py = python();
    py.args([
        "-c",
        script,
        &addr.port().to_string(),
        &series[0].study_uid(),
    ]);
    let output = run(py).await;
    assert!(output.status.success(), "{}", text(&output));
    let printed: Vec<(String, Vec<u16>)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| {
            let mut words = line.split(' ');
            let uid = words.next().unwrap().to_owned();
            (uid, words.map(|v| v.parse().unwrap()).collect())
        })
        .collect();
    assert_eq!(printed, expected_pixels(&series));
}

// ---- the Patient Root model (PS3.4 C.6.1), asked by DCMTK's tools with -P and by pynetdicom ----

/// findscu, movescu and getscu on the Patient Root model: patients, a patient's studies,
/// a patient moved whole to movescu's receiver, and a study of the patient got.
#[tokio::test]
#[ignore = "needs DCMTK"]
async fn dcmtk_tools_query_and_retrieve_on_the_patient_root_model() {
    use dicom_dictionary_std::tags;
    let (archive, series) = archive_to_retrieve_from().await;
    let other = RealSeries::new(93, 1).with_patient("Roe^Richard", None);
    ingest(&archive, &other).await;
    let receiving = free_port();
    let mut config = retrieving("DCMTK", "DCMTK", receiving);
    config.remotes.extend(answering(&["DCMTK"]).remotes);
    let addr = start(config, archive).await;
    let patient = series[0].patient_id();

    let (output, found) = findscu_on(
        "-P",
        addr.port(),
        "DCMTK",
        &[
            "QueryRetrieveLevel=PATIENT",
            "PatientID",
            "PatientName=DOE*",
            "NumberOfPatientRelatedInstances",
        ],
        &[],
    )
    .await;
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(values(&found, tags::PATIENT_ID), [patient.as_str()]);
    assert_eq!(
        values(&found, tags::NUMBER_OF_PATIENT_RELATED_INSTANCES),
        ["5"]
    );
    let of_patient = format!("PatientID={patient}");
    let (output, found) = findscu_on(
        "-P",
        addr.port(),
        "DCMTK",
        &[
            "QueryRetrieveLevel=STUDY",
            &of_patient,
            "StudyInstanceUID",
            "StudyDate",
        ],
        &[],
    )
    .await;
    assert!(output.status.success(), "{}", text(&output));
    assert_eq!(
        values(&found, tags::STUDY_INSTANCE_UID),
        [series[0].study_uid()]
    );
    // a study without its patient is not a query of this model
    let (output, found) = findscu_on(
        "-P",
        addr.port(),
        "DCMTK",
        &["QueryRetrieveLevel=STUDY", "StudyInstanceUID"],
        &[],
    )
    .await;
    assert!(found.is_empty());
    assert!(
        text(&output).contains("DataSetDoesNotMatchSOPClass"),
        "{}",
        text(&output)
    );

    let out = tempfile::tempdir().unwrap();
    let mut movescu = dcmtk("movescu");
    movescu
        .args([
            "-v", "-P", "-aet", "DCMTK", "-aec", "PACSRS", "-aem", "DCMTK",
        ])
        .args(["+P", &receiving.to_string(), "-od"])
        .arg(out.path())
        .args(["-k", "QueryRetrieveLevel=PATIENT", "-k", &of_patient])
        .args(["127.0.0.1", &addr.port().to_string()]);
    let output = run(movescu).await;
    assert!(output.status.success(), "{}", text(&output));
    let moved: Vec<(String, Vec<u16>)> = written(out.path(), 5)
        .await
        .into_iter()
        .map(|(uid, _, pixels)| (uid, pixels))
        .collect();
    assert_eq!(moved, expected_pixels(&series));

    let out = tempfile::tempdir().unwrap();
    let mut getscu = dcmtk("getscu");
    getscu
        .args(["-v", "-P", "-aet", "DCMTK", "-aec", "PACSRS", "-od"])
        .arg(out.path())
        .args(["-k", "QueryRetrieveLevel=STUDY", "-k", &of_patient, "-k"])
        .arg(format!("StudyInstanceUID={}", series[0].study_uid()))
        .args(["127.0.0.1", &addr.port().to_string()]);
    let output = run(getscu).await;
    assert!(output.status.success(), "{}", text(&output));
    let got: Vec<(String, Vec<u16>)> = written(out.path(), 5)
        .await
        .into_iter()
        .map(|(uid, _, pixels)| (uid, pixels))
        .collect();
    assert_eq!(got, expected_pixels(&series));
}

/// pynetdicom on the Patient Root model: patients, a patient's studies, and a patient
/// got whole.
#[tokio::test]
#[ignore = "needs pynetdicom"]
async fn pynetdicom_queries_and_retrieves_on_the_patient_root_model() {
    let (archive, series) = archive_to_retrieve_from().await;
    let other = RealSeries::new(94, 1).with_patient("Roe^Richard", None);
    ingest(&archive, &other).await;
    let addr = start(answering(&["PYNETDICOM"]), archive).await;
    let script = r#"
import sys
from pydicom.dataset import Dataset
from pydicom.uid import ImplicitVRLittleEndian
from pynetdicom import AE, evt, build_role
from pynetdicom.sop_class import CTImageStorage
from pynetdicom.sop_class import PatientRootQueryRetrieveInformationModelFind as FIND
from pynetdicom.sop_class import PatientRootQueryRetrieveInformationModelGet as GET
port, patient, study = int(sys.argv[1]), sys.argv[2], sys.argv[3]

stored = []
def on_store(event):
    stored.append(event.dataset.SOPInstanceUID)
    return 0x0000

ae = AE(ae_title="PYNETDICOM")
ae.add_requested_context(FIND)
ae.add_requested_context(GET)
ae.add_requested_context(CTImageStorage, ImplicitVRLittleEndian)
assoc = ae.associate("127.0.0.1", port, ae_title="PACSRS",
                     ext_neg=[build_role(CTImageStorage, scp_role=True)],
                     evt_handlers=[(evt.EVT_C_STORE, on_store)])
assert assoc.is_established, "no association"

def find(**keys):
    ds = Dataset()
    for name, value in keys.items():
        setattr(ds, name, value)
    found, last = [], None
    for status, identifier in assoc.send_c_find(ds, FIND):
        if status.Status in (0xFF00, 0xFF01):
            found.append(identifier)
        else:
            last = status.Status
    return found, last

found, last = find(QueryRetrieveLevel="PATIENT", PatientID="", PatientName="",
                   NumberOfPatientRelatedStudies="")
assert last == 0x0000, hex(last)
assert sorted(str(i.PatientName) for i in found) == ["Doe^Jane", "Roe^Richard"], found
doe = [i for i in found if i.PatientID == patient][0]
assert doe.NumberOfPatientRelatedStudies == 1, doe

found, last = find(QueryRetrieveLevel="STUDY", PatientID=patient, StudyInstanceUID="",
                   NumberOfStudyRelatedInstances="")
assert last == 0x0000 and [i.StudyInstanceUID for i in found] == [study], (found, last)
assert found[0].PatientID == patient and found[0].NumberOfStudyRelatedInstances == 5, found[0]

# below the PATIENT level a query names its patient
found, last = find(QueryRetrieveLevel="STUDY", StudyInstanceUID="")
assert last == 0xA900 and not found, (found, last)

ds = Dataset()
ds.QueryRetrieveLevel = "PATIENT"
ds.PatientID = patient
responses = [s for s, _ in assoc.send_c_get(ds, GET)]
last = responses[-1]
assert last.Status == 0x0000 and last.NumberOfCompletedSuboperations == 5, last
assert len(stored) == 5, stored
assoc.release()
"#;
    let mut py = python();
    py.args([
        "-c",
        script,
        &addr.port().to_string(),
        &series[0].patient_id(),
        &series[0].study_uid(),
    ]);
    let output = run(py).await;
    assert!(output.status.success(), "{}", text(&output));
}
