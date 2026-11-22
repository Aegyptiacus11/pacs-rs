//! Deletion that can be undone (`/trash`, `PACS_TRASH_DAYS`), through the REST API on a real
//! series: deleted, listed, restored with its project and annotations, emptied now or when
//! old, and with no trash at all.

use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt;
use pacs_archive::Archive;
use pacs_core::real::RealSeries;
use pacs_server::auth::{Right, right_for};
use pacs_server::{Settings, app, open_archive};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Server {
    app: Router,
    archive: Archive,
    files: PathBuf,
    series: RealSeries,
    _dir: tempfile::TempDir,
}

async fn server(extra: &[&str], slices: u32) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index.sqlite").display().to_string();
    let files = dir.path().join("files");
    let storage = format!("file:{}", files.display());
    let mut args = vec!["pacs-server", "--index", &index, "--storage", &storage];
    args.extend(extra);
    let settings = Settings::try_parse_from(args).unwrap();
    let archive = open_archive(&settings).await.unwrap();
    let series = RealSeries::new(97, slices);
    for bytes in series.all_bytes() {
        archive.ingest(bytes.into()).await.unwrap();
    }
    Server {
        app: app(archive.clone(), &settings).unwrap(),
        archive,
        files,
        series,
        _dir: dir,
    }
}

impl Server {
    async fn call(&self, method: Method, uri: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn study_id(&self) -> Option<String> {
        let (_, studies) = self.call(Method::GET, "/studies").await;
        studies
            .as_array()
            .unwrap()
            .first()
            .map(|s| s.as_str().unwrap().to_owned())
    }

    async fn instances(&self) -> usize {
        let Some(study) = self.study_id().await else {
            return 0;
        };
        let (_, instances) = self
            .call(Method::GET, &format!("/studies/{study}/instances"))
            .await;
        instances.as_array().unwrap().len()
    }

    async fn trash(&self) -> Vec<Value> {
        let (status, entries) = self.call(Method::GET, "/trash").await;
        assert_eq!(status, StatusCode::OK);
        entries.as_array().unwrap().clone()
    }

    /// The files under the store's directory, `trash/` or not.
    fn files_in(&self, trash: bool) -> usize {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.push(path);
                }
            }
        }
        let mut all = Vec::new();
        walk(&self.files, &mut all);
        let trash_dir = self.files.join("trash");
        all.iter()
            .filter(|p| p.starts_with(&trash_dir) == trash)
            .count()
    }
}

#[tokio::test]
async fn a_deleted_study_waits_in_the_trash_and_comes_back_whole() {
    let s = server(&[], 3).await;
    let study_uid = s.series.study_uid();
    s.archive
        .index()
        .put_annotations(&study_uid, Some(r#"[{"tool":"Length"}]"#))
        .await
        .unwrap();
    let study = s.study_id().await.unwrap();
    let (status, _) = s.call(Method::DELETE, &format!("/studies/{study}")).await;
    assert_eq!(status, StatusCode::OK);
    // gone from what is served, waiting in the store
    assert_eq!(s.study_id().await, None);
    assert_eq!((s.files_in(false), s.files_in(true)), (0, 3));
    let trash = s.trash().await;
    assert_eq!(trash.len(), 1);
    let entry = &trash[0];
    assert_eq!(
        (entry["level"].as_str(), entry["files"].as_i64()),
        (Some("Study"), Some(3))
    );
    assert_eq!(entry["studies"][0]["StudyInstanceUID"], study_uid);
    assert_eq!(entry["studies"][0]["PatientID"], s.series.patient_id());
    assert!(entry["studies"][0].get("annotations").is_none(), "{entry}");
    let days = entry["emptied_ms"].as_i64().unwrap() - entry["deleted_ms"].as_i64().unwrap();
    assert_eq!(days, 30 * 86_400_000);
    // the store and the index agree: the trash is the trash's
    let found = s.archive.inconsistencies(Duration::ZERO).await.unwrap();
    assert!(
        found.orphans.is_empty() && found.lost.is_empty(),
        "{found:?}"
    );

    let id = entry["ID"].as_str().unwrap();
    let (status, done) = s.call(Method::POST, &format!("/trash/{id}/restore")).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done, json!({"restored": 3, "kept": []}));
    assert_eq!(s.instances().await, 3);
    assert_eq!((s.files_in(false), s.files_in(true)), (3, 0));
    assert_eq!(
        s.archive
            .index()
            .annotations(&study_uid)
            .await
            .unwrap()
            .as_deref(),
        Some(r#"[{"tool":"Length"}]"#)
    );
    assert!(s.trash().await.is_empty());
    let (status, _) = s.call(Method::POST, &format!("/trash/{id}/restore")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_part_of_a_study_comes_back_into_it() {
    let s = server(&[], 3).await;
    let study = s.study_id().await.unwrap();
    let (_, instances) = s
        .call(Method::GET, &format!("/studies/{study}/instances"))
        .await;
    let one = instances[0]["ID"].as_str().unwrap().to_owned();
    assert_eq!(
        s.call(Method::DELETE, &format!("/instances/{one}")).await.0,
        StatusCode::OK
    );
    assert_eq!(s.instances().await, 2);
    let trash = s.trash().await;
    assert_eq!(
        (trash[0]["level"].as_str(), trash[0]["files"].as_i64()),
        (Some("Instance"), Some(1))
    );
    let id = trash[0]["ID"].as_str().unwrap();
    assert_eq!(
        s.call(Method::POST, &format!("/trash/{id}/restore"))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(s.instances().await, 3);
}

#[tokio::test]
async fn an_instance_stored_again_since_keeps_the_newer_copy() {
    let s = server(&[], 2).await;
    let study = s.study_id().await.unwrap();
    s.call(Method::DELETE, &format!("/studies/{study}")).await;
    // the first slice is sent again before the restore
    s.archive
        .ingest(s.series.all_bytes()[0].clone().into())
        .await
        .unwrap();
    let id = s.trash().await[0]["ID"].as_str().unwrap().to_owned();
    let (status, done) = s.call(Method::POST, &format!("/trash/{id}/restore")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(done["restored"], 1);
    assert_eq!(done["kept"].as_array().unwrap().len(), 1);
    assert!(
        done["kept"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("stored again")
    );
    assert_eq!(s.instances().await, 2);
    // the entry stays with the file it kept, to be emptied
    let trash = s.trash().await;
    assert_eq!(trash[0]["files"], 1);
    assert_eq!(
        s.call(Method::DELETE, &format!("/trash/{id}")).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(s.files_in(true), 0);
    assert!(s.trash().await.is_empty());
}

#[tokio::test]
async fn an_entry_is_emptied_on_request_or_once_old() {
    let s = server(&["--trash-days", "7"], 1).await;
    let study = s.study_id().await.unwrap();
    s.call(Method::DELETE, &format!("/studies/{study}")).await;
    // not old yet
    assert_eq!(s.archive.empty_old_trash().await.unwrap(), 0);
    sqlx::query("UPDATE trash SET deleted_ms = deleted_ms - $1")
        .bind(8 * 86_400_000i64)
        .execute(s.archive.index().pool())
        .await
        .unwrap();
    assert_eq!(s.archive.empty_old_trash().await.unwrap(), 1);
    assert_eq!(s.files_in(true), 0);
    assert!(s.trash().await.is_empty());
    assert_eq!(
        s.call(Method::DELETE, "/trash/1").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        s.call(Method::POST, "/trash/x/restore").await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_study_of_a_project_comes_back_into_its_project() {
    let s = server(&[], 1).await;
    let series = RealSeries::new(98, 2);
    let project = s.archive.clone().for_project("liver");
    for bytes in series.all_bytes() {
        project.ingest(bytes.into()).await.unwrap();
    }
    let index = s.archive.index();
    let (_, studies) = s.call(Method::GET, "/studies").await;
    assert_eq!(studies.as_array().unwrap().len(), 2);
    for study in studies.as_array().unwrap() {
        let (_, about) = s
            .call(
                Method::GET,
                &format!("/studies/{}", study.as_str().unwrap()),
            )
            .await;
        if about["MainDicomTags"]["StudyInstanceUID"] == series.study_uid() {
            s.call(
                Method::DELETE,
                &format!("/studies/{}", study.as_str().unwrap()),
            )
            .await;
        }
    }
    let entry = s.trash().await[0].clone();
    assert_eq!(entry["studies"][0]["project"], "liver");
    let id = entry["ID"].as_str().unwrap();
    s.call(Method::POST, &format!("/trash/{id}/restore")).await;
    assert_eq!(
        index.project_of_study(&series.study_uid()).await.unwrap(),
        Some(Some("liver".to_owned()))
    );
}

#[tokio::test]
async fn with_no_trash_a_deletion_removes_the_files_at_once() {
    let s = server(&["--trash-days", "0"], 2).await;
    let study = s.study_id().await.unwrap();
    s.call(Method::DELETE, &format!("/studies/{study}")).await;
    assert_eq!((s.files_in(false), s.files_in(true)), (0, 0));
    assert!(s.trash().await.is_empty());
}

#[test]
fn only_who_may_delete_reaches_the_trash() {
    for (method, path) in [
        (Method::GET, "/trash"),
        (Method::POST, "/trash/1/restore"),
        (Method::DELETE, "/trash/1"),
    ] {
        assert_eq!(right_for(&method, path), Right::Delete, "{method} {path}");
    }
}
