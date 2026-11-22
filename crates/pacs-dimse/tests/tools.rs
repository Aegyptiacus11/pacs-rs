//! The command-line tools, as a site runs them: storescp receiving what storescu sends,
//! echoscu, and findscu, movescu and getscu against pacs-rs's own SCP, on real images; and
//! the exit status of what fails.

mod common;

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use common::*;
use pacs_core::real::{RealSeries, path};
use pacs_dimse::AeTitle;
use pacs_index::Index;

fn tool(name: &str) -> Command {
    let bin = match name {
        "echoscu" => env!("CARGO_BIN_EXE_echoscu"),
        "storescu" => env!("CARGO_BIN_EXE_storescu"),
        "storescp" => env!("CARGO_BIN_EXE_storescp"),
        "findscu" => env!("CARGO_BIN_EXE_findscu"),
        "movescu" => env!("CARGO_BIN_EXE_movescu"),
        "getscu" => env!("CARGO_BIN_EXE_getscu"),
        _ => unreachable!(),
    };
    Command::new(bin)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// storescp on a free port, into `output`, once it answers C-ECHO.
struct Receiver(Child, u16);

impl Receiver {
    async fn start(output: &Path) -> Receiver {
        let port = free_port();
        // owned at once, so that it is stopped whatever happens next
        let receiver = Receiver(
            tool("storescp")
                .args(["--bind", "127.0.0.1", "--aet", "STORESCP", "--output"])
                .arg(output)
                .arg(port.to_string())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
            port,
        );
        for _ in 0..100 {
            let echo = tool("echoscu")
                .arg(format!("STORESCP@127.0.0.1:{port}"))
                .output()
                .unwrap();
            if echo.status.success() {
                return receiver;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("storescp never answered");
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn received(output: &Path) -> usize {
    let index = Index::open(output.join("index.sqlite")).await.unwrap();
    let mut count = 0;
    for study in index.studies(&Default::default()).await.unwrap() {
        count += study.instance_count as usize;
    }
    count
}

#[tokio::test]
async fn storescu_sends_what_storescp_keeps() {
    let dir = tempfile::tempdir().unwrap();
    let receiver = Receiver::start(dir.path()).await;
    let to = format!("STORESCP@127.0.0.1:{}", receiver.1);
    let sent = tool("storescu")
        .arg(&to)
        .arg(path("CT_small.dcm"))
        .arg(path("MR_small_jpeg_ls_lossless.dcm"))
        .output()
        .unwrap();
    assert!(
        sent.status.success(),
        "{}",
        String::from_utf8_lossy(&sent.stderr)
    );
    assert_eq!(received(dir.path()).await, 2);
    // what is not DICOM is said, and the exit status says so
    let not_dicom = dir.path().join("notes.txt");
    std::fs::write(&not_dicom, "not DICOM").unwrap();
    let sent = tool("storescu")
        .arg(&to)
        .arg(path("CT_small.dcm"))
        .arg(&not_dicom)
        .output()
        .unwrap();
    assert!(!sent.status.success());
    assert!(String::from_utf8_lossy(&sent.stderr).contains("not DICOM"));
}

// the tools run as processes while the SCP serves on this runtime: it needs threads of its own
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn find_move_and_get_from_pacs_rs() {
    let archive = archive().await;
    let series = RealSeries::new(1201, 2).with_patient("DOE^JANE", None);
    ingest(&archive, &series).await;
    let dir = tempfile::tempdir().unwrap();
    let receiver = Receiver::start(&dir.path().join("moved")).await;
    let mut config = config();
    config.remotes = vec![
        "tools=PACSRS@127.0.0.1:11112".parse().unwrap(),
        format!("receiver=STORESCP@127.0.0.1:{}", receiver.1)
            .parse()
            .unwrap(),
    ];
    config.move_destinations = vec!["receiver".into()];
    config.ae_title = AeTitle::new("ARCHIVE").unwrap();
    let addr = start(config, archive).await;
    let at = format!("ARCHIVE@127.0.0.1:{}", addr.port());

    let found = tool("findscu")
        .args([&at, "--patient-name", "doe^*"])
        .output()
        .unwrap();
    assert!(
        found.status.success(),
        "{}",
        String::from_utf8_lossy(&found.stderr)
    );
    let lines: Vec<serde_json::Value> = String::from_utf8_lossy(&found.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["0020000D"]["Value"][0], series.study_uid());

    let moved = tool("movescu")
        .args([&at, &series.study_uid(), "--destination", "STORESCP"])
        .output()
        .unwrap();
    assert!(
        moved.status.success(),
        "{}",
        String::from_utf8_lossy(&moved.stderr)
    );
    assert_eq!(received(&dir.path().join("moved")).await, 2);

    let got = tool("getscu")
        .args([&at, &series.study_uid(), "--output"])
        .arg(dir.path().join("got"))
        .output()
        .unwrap();
    assert!(
        got.status.success(),
        "{}",
        String::from_utf8_lossy(&got.stderr)
    );
    assert_eq!(received(&dir.path().join("got")).await, 2);
}

#[test]
fn what_fails_exits_with_failure() {
    let nobody = format!("NOBODY@127.0.0.1:{}", free_port());
    for (name, args) in [
        ("echoscu", vec![nobody.as_str()]),
        ("echoscu", vec!["no title here"]),
        ("storescu", vec![nobody.as_str(), "/nonexistent.dcm"]),
        (
            "findscu",
            vec![nobody.as_str(), "--study-date", "2026-01-01"],
        ),
        ("getscu", vec![nobody.as_str(), "not-a-uid"]),
    ] {
        // run in a folder of its own: nothing it may leave lands in the checkout
        let dir = tempfile::tempdir().unwrap();
        let run = tool(name)
            .current_dir(dir.path())
            .args(&args)
            .arg("--timeout")
            .arg("2")
            .output()
            .unwrap();
        assert!(!run.status.success(), "{name} {args:?}");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "{name} {args:?} left files"
        );
    }
}
