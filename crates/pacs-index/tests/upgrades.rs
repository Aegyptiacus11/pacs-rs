//! Every release's index, opened by today's code: the copies in `tests/upgrades/` were
//! written by the released images themselves (`scripts/upgrade-fixtures.sh`), each holding
//! the sample study of docs/sample. Opening one runs the migrations it lacks; the study must
//! then read as it was written, and take what today's code writes.
//!
//! On SQLite always; on Postgres when `PACS_TEST_POSTGRES_URL` names a server.

use std::path::{Path, PathBuf};

use pacs_core::Instance;
use pacs_index::{Index, StudyFilter};

/// Every release published to ghcr.io, oldest first. After a release, its copy is made with
/// `scripts/upgrade-fixtures.sh <version>` and the version added here.
const RELEASES: [&str; 9] = [
    "0.5.2", "0.6.0", "0.7.0", "0.8.0", "0.8.1", "0.9.0", "0.10.0", "0.11.0", "0.12.0",
];

fn copies() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/upgrades")
}

/// The sample study, as the releases were sent it.
fn sample() -> (String, Vec<String>) {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/sample");
    let mut study = String::new();
    let mut instances = Vec::new();
    for name in ["00001.dcm", "00002.dcm", "00003.dcm"] {
        let bytes = std::fs::read(sample.join(name)).unwrap();
        let instance = Instance::from_bytes(&bytes).unwrap();
        study = instance.key.study_instance_uid.clone();
        instances.push(instance.key.sop_instance_uid.clone());
    }
    instances.sort();
    (study, instances)
}

/// What a release wrote, read by today's code, and today's writes taken.
async fn reads_as_written(index: &Index, release: &str, first: bool) {
    let (study_uid, instance_uids) = sample();
    let studies = index.studies(&StudyFilter::default()).await.unwrap();
    assert_eq!(studies.len(), 1, "{release}");
    let study = index.study(&study_uid).await.unwrap().unwrap();
    assert_eq!(study.instance_count, 3, "{release}");
    assert_eq!(study.series_count, 1, "{release}");
    assert!(
        study.attributes["00100020"]["Value"][0].is_string(),
        "{release}"
    );
    let files = index.files_of_study(&study_uid).await.unwrap();
    let uids: Vec<_> = files.iter().map(|f| f.sop_instance_uid.clone()).collect();
    assert_eq!(uids, instance_uids, "{release}");
    for file in &files {
        assert!(
            file.size_bytes > 0 && !file.object_key.is_empty(),
            "{release}"
        );
    }
    // stored before hashes were kept (until 0.12): due for a first check, which takes one
    if first && release_before(release, "0.13.0") {
        let counts = index.check_counts().await.unwrap();
        assert_eq!((counts.files, counts.never_checked), (3, 3), "{release}");
        assert_eq!(index.files_due(0, 10).await.unwrap().len(), 3, "{release}");
    }
    index
        .record_check(&instance_uids[0], Some("abc"), None)
        .await
        .unwrap();
    // the tables later releases added are there and work
    assert_eq!(index.user_count().await.unwrap(), 0, "{release}");
    assert!(index.role("admin").await.unwrap().is_some(), "{release}");
}

fn release_before(release: &str, than: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> { v.split('.').map(|p| p.parse().unwrap()).collect() };
    parse(release) < parse(than)
}

#[test]
fn every_release_has_its_copies() {
    for release in RELEASES {
        for kind in ["sqlite", "postgres.sql"] {
            let copy = copies().join(format!("{release}.{kind}"));
            assert!(copy.is_file(), "no copy {}", copy.display());
        }
    }
}

#[tokio::test]
async fn every_releases_sqlite_index_opens_and_reads_as_written() {
    for release in RELEASES {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.sqlite");
        std::fs::copy(copies().join(format!("{release}.sqlite")), &path).unwrap();
        let index = Index::open(&path)
            .await
            .unwrap_or_else(|e| panic!("the index of {release} does not open: {e}"));
        reads_as_written(&index, release, true).await;
        // and opens again, nothing left to migrate
        drop(index);
        let again = Index::open(&path).await.unwrap();
        reads_as_written(&again, release, false).await;
    }
}

#[tokio::test]
async fn every_releases_postgres_index_opens_and_reads_as_written() {
    let Ok(server) = std::env::var("PACS_TEST_POSTGRES_URL") else {
        eprintln!("PACS_TEST_POSTGRES_URL not set; skipping");
        return;
    };
    let server = server.trim_end_matches('/');
    sqlx::any::install_default_drivers();
    let admin = sqlx::AnyPool::connect(&format!("{server}/postgres"))
        .await
        .unwrap();
    for release in RELEASES {
        let name = format!(
            "pacs_upgrade_{}_{}",
            release.replace('.', "_"),
            std::process::id()
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .unwrap();
        let url = format!("{server}/{name}");
        let dump =
            std::fs::read_to_string(copies().join(format!("{release}.postgres.sql"))).unwrap();
        let restore = sqlx::AnyPool::connect(&url).await.unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(dump))
            .execute(&restore)
            .await
            .unwrap_or_else(|e| panic!("the dump of {release} does not load: {e}"));
        restore.close().await;
        let index = Index::connect(&url)
            .await
            .unwrap_or_else(|e| panic!("the index of {release} does not open: {e}"));
        reads_as_written(&index, release, true).await;
    }
}
