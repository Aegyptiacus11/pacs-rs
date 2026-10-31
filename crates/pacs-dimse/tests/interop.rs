//! pacs-rs against other implementations: DCMTK's echoscu, storescu, storescp and
//! dcmqrscp, and pynetdicom. Ignored by default; with the tools installed:
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
/// dcmqridx.
async fn dcmqrscp(extra: u32) -> (Killed, RemoteAe, tempfile::TempDir, Vec<String>) {
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
             HostTable BEGIN\nHostTable END\nVendorTable BEGIN\nVendorTable END\n\
             AETable BEGIN\nQRSCP {} RW (10, 64mb) ANY\nAETable END\n",
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
    let (_scp, remote, _dir, uids) = dcmqrscp(0).await;
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
    let (_scp, remote, _dir, _) = dcmqrscp(100).await;
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
