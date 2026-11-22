//! Checking stored files (`/tools/check-files`, `pacs_server::checks`): files changed,
//! cut short, gone, unreadable, and stored before hashes were kept, on a real series.

use std::path::PathBuf;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt;
use pacs_archive::Archive;
use pacs_core::real::RealSeries;
use pacs_server::auth::{Right, right_for};
use pacs_server::checks::{Summary, check_all, check_due};
use pacs_server::{Settings, app, open_archive};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Stored {
    app: Router,
    archive: Archive,
    files: PathBuf,
    series: RealSeries,
    _dir: tempfile::TempDir,
}

/// A server with a real series of `slices` stored.
async fn stored(slices: u32) -> Stored {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index.sqlite").display().to_string();
    let files = dir.path().join("files");
    let storage = format!("file:{}", files.display());
    let settings =
        Settings::try_parse_from(["pacs-server", "--index", &index, "--storage", &storage])
            .unwrap();
    let archive = open_archive(&settings).await.unwrap();
    let series = RealSeries::new(91, slices);
    for bytes in series.all_bytes() {
        archive.ingest(bytes.into()).await.unwrap();
    }
    Stored {
        app: app(archive.clone(), &settings).unwrap(),
        archive,
        files,
        series,
        _dir: dir,
    }
}

impl Stored {
    /// Where the store keeps a slice's file.
    async fn file_of(&self, slice: u32) -> PathBuf {
        let uid = self.series.instance_uid(slice);
        let files = self
            .archive
            .index()
            .files_of_study(&self.series.study_uid())
            .await
            .unwrap();
        let file = files.iter().find(|f| f.sop_instance_uid == uid).unwrap();
        self.files.join(&file.object_key)
    }

    async fn ask(&self, method: Method, body: Option<Value>) -> (StatusCode, Value) {
        let request = Request::builder().method(method).uri("/tools/check-files");
        let request = match body {
            Some(body) => request
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn check_study(&self) -> (StatusCode, Value) {
        self.ask(
            Method::POST,
            Some(json!({"StudyInstanceUID": self.series.study_uid()})),
        )
        .await
    }

    async fn status(&self) -> Value {
        let (status, body) = self.ask(Method::GET, None).await;
        assert_eq!(status, StatusCode::OK);
        body
    }
}

fn failure_of<'a>(answer: &'a Value, uid: &str) -> Option<&'a str> {
    answer["failures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["SOPInstanceUID"] == uid)
        .and_then(|f| f["failure"].as_str())
}

#[tokio::test]
async fn a_changed_short_or_missing_file_is_found_and_a_restored_one_passes_again() {
    let s = stored(4).await;
    let (status, answer) = s.check_study().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(answer, json!({"checked": 4, "failures": []}));

    // one byte changed, one file cut short, one gone
    let changed = s.file_of(0).await;
    let original = std::fs::read(&changed).unwrap();
    let mut flipped = original.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 0xff;
    std::fs::write(&changed, &flipped).unwrap();
    let short = s.file_of(1).await;
    let bytes = std::fs::read(&short).unwrap();
    std::fs::write(&short, &bytes[..bytes.len() / 2]).unwrap();
    std::fs::remove_file(s.file_of(2).await).unwrap();

    let (status, answer) = s.check_study().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(answer["checked"], 4);
    let uid = |slice| s.series.instance_uid(slice);
    assert!(
        failure_of(&answer, &uid(0))
            .unwrap()
            .starts_with("changed: SHA-256 ")
    );
    assert!(
        failure_of(&answer, &uid(1))
            .unwrap()
            .contains("bytes in the store")
    );
    assert_eq!(failure_of(&answer, &uid(2)), Some("missing from the store"));
    assert_eq!(failure_of(&answer, &uid(3)), None);

    // they stay listed until a check passes
    let status = s.status().await;
    assert_eq!(
        (status["files"].as_i64(), status["failed"].as_i64()),
        (Some(4), Some(3))
    );
    let listed = status["failures"].as_array().unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0]["StudyInstanceUID"], s.series.study_uid());
    std::fs::write(&changed, &original).unwrap();
    s.check_study().await;
    let status = s.status().await;
    assert_eq!(status["failed"], 2);
    assert_eq!(failure_of(&status, &uid(0)), None);
}

#[tokio::test]
async fn a_file_stored_before_hashes_were_kept_gets_one_at_its_first_check() {
    let s = stored(2).await;
    sqlx::query("UPDATE instances SET sha256 = NULL, checked_ms = NULL")
        .execute(s.archive.index().pool())
        .await
        .unwrap();
    assert_eq!(s.status().await["never_checked"], 2);
    // never checked: due, whatever the days
    let summary = check_due(&s.archive, 30).await.unwrap();
    assert_eq!((summary.checked, summary.failures.len()), (2, 0));
    assert_eq!(s.status().await["never_checked"], 0);
    // checked just now: nothing due
    assert_eq!(check_due(&s.archive, 30).await.unwrap().checked, 0);
    // the hash taken is the file's: a change after it is found
    let file = s.file_of(1).await;
    let mut bytes = std::fs::read(&file).unwrap();
    bytes[0] ^= 1;
    std::fs::write(&file, bytes).unwrap();
    let (_, answer) = s.check_study().await;
    assert!(failure_of(&answer, &s.series.instance_uid(1)).is_some());
}

#[cfg(unix)]
#[tokio::test]
async fn a_store_that_cannot_be_read_marks_no_file_bad() {
    use std::os::unix::fs::PermissionsExt;
    let s = stored(1).await;
    let file = s.file_of(0).await;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&file).is_ok() {
        eprintln!("running as root: a file without permissions is still readable; skipping");
        return;
    }
    let (status, answer) = s.check_study().await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
    assert_eq!(s.status().await["failed"], 0);
    // from the command line too: an error, not a list of bad files
    let mut summary = Summary::default();
    assert!(check_all(&s.archive, &mut summary).await.is_err());
    assert!(summary.failures.is_empty());
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
}

#[tokio::test]
async fn the_whole_archive_is_checked_in_the_background_once_at_a_time() {
    let s = stored(3).await;
    std::fs::remove_file(s.file_of(1).await).unwrap();
    let (status, _) = s.ask(Method::POST, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let mut sweep = Value::Null;
    for _ in 0..100 {
        let status = s.status().await;
        if status["running"] == false {
            sweep = status["last_sweep"].clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(sweep["checked"], 3, "{sweep}");
    assert_eq!(sweep["failed"], 1);
    assert!(sweep["finished_ms"].as_i64().unwrap() >= sweep["started_ms"].as_i64().unwrap());
    assert_eq!(sweep["error"], Value::Null);

    // the same, as `pacs-server --check-files` runs it
    let mut summary = Summary::default();
    check_all(&s.archive, &mut summary).await.unwrap();
    assert_eq!((summary.checked, summary.failures.len()), (3, 1));
}

#[tokio::test]
async fn a_study_that_is_not_there_or_a_bad_request_is_refused() {
    let s = stored(1).await;
    let (status, _) = s
        .ask(Method::POST, Some(json!({"StudyInstanceUID": "1.2.3"})))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = s
        .ask(Method::POST, Some(json!({"StudyInstanceUID": 7})))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[test]
fn only_who_manages_users_checks_files() {
    for method in [Method::GET, Method::POST] {
        assert_eq!(right_for(&method, "/tools/check-files"), Right::Admin);
    }
    // the other tools are unchanged
    assert_eq!(right_for(&Method::POST, "/tools/find"), Right::Read);
}
