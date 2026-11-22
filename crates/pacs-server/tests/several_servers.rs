//! Two servers on one Postgres index and one store: each its own `Archive`, with its own
//! connections and its own in-process locks, as two processes would have. Uploads and
//! deletions of the same instances race; afterwards the store and the index agree: no entry
//! names a file that is gone, and no file is left that no entry names.
//!
//! Runs only when `PACS_TEST_POSTGRES_URL` names a Postgres server.

use std::time::Duration;

use clap::Parser;
use pacs_archive::Archive;
use pacs_core::real::RealSeries;
use pacs_index::Level;
use pacs_server::{Settings, open_archive};

async fn two_servers(extra: &[&str]) -> Option<(Archive, Archive, tempfile::TempDir)> {
    let Ok(server) = std::env::var("PACS_TEST_POSTGRES_URL") else {
        eprintln!("PACS_TEST_POSTGRES_URL not set; skipping");
        return None;
    };
    let server = server.trim_end_matches('/');
    sqlx::any::install_default_drivers();
    let admin = sqlx::AnyPool::connect(&format!("{server}/postgres"))
        .await
        .unwrap();
    static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let name = format!(
        "pacs_two_{}_{}",
        std::process::id(),
        COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&admin)
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let index = format!("{server}/{name}");
    let storage = format!("file:{}", dir.path().join("files").display());
    let settings = || {
        let mut args = vec!["pacs-server", "--index", &index, "--storage", &storage];
        args.extend(extra);
        Settings::try_parse_from(args).unwrap()
    };
    let a = open_archive(&settings()).await.unwrap();
    let b = open_archive(&settings()).await.unwrap();
    Some((a, b, dir))
}

async fn store_all(archive: &Archive, series: &RealSeries) {
    for bytes in series.all_bytes() {
        archive.ingest(bytes.into()).await.unwrap();
    }
}

async fn agree(archive: &Archive) {
    let found = archive.inconsistencies(Duration::ZERO).await.unwrap();
    assert!(
        found.orphans.is_empty() && found.lost.is_empty(),
        "orphans {:?}, lost {:?}",
        found.orphans,
        found.lost
    );
}

/// One server deletes a study while the other stores its instances again, many times.
#[tokio::test]
async fn a_deletion_and_an_upload_on_two_servers_leave_no_lost_or_stray_file() {
    let Some((a, b, _dir)) = two_servers(&["--trash-days", "0"]).await else {
        return;
    };
    let series = RealSeries::new(99, 4);
    store_all(&a, &series).await;
    for _ in 0..15 {
        let study = a
            .index()
            .study(&series.study_uid())
            .await
            .unwrap()
            .and_then(|s| s.public_id);
        let deleting = async {
            if let Some(id) = &study {
                a.delete(Level::Study, id).await.unwrap();
            }
        };
        tokio::join!(deleting, store_all(&b, &series));
        agree(&a).await;
    }
}

/// Both servers store the same instances at once, with and without overwriting: each instance
/// ends with one file, the one its entry names.
#[tokio::test]
async fn the_same_instances_stored_on_two_servers_at_once_leave_one_file_each() {
    for overwrite in [&[][..], &["--overwrite-instances"][..]] {
        let Some((a, b, _dir)) = two_servers(overwrite).await else {
            return;
        };
        let series = RealSeries::new(100, 4);
        for _ in 0..10 {
            tokio::join!(store_all(&a, &series), store_all(&b, &series));
            agree(&a).await;
        }
        let study = a.index().study(&series.study_uid()).await.unwrap().unwrap();
        assert_eq!(study.instance_count, 4);
    }
}

/// A deletion with the trash on, against an upload on the other server: the trash keeps the
/// deleted files, and the store and the index still agree.
#[tokio::test]
async fn a_deletion_into_the_trash_and_an_upload_on_two_servers_agree() {
    let Some((a, b, _dir)) = two_servers(&[]).await else {
        return;
    };
    let series = RealSeries::new(101, 3);
    store_all(&a, &series).await;
    for _ in 0..10 {
        let study = a
            .index()
            .study(&series.study_uid())
            .await
            .unwrap()
            .and_then(|s| s.public_id);
        let deleting = async {
            if let Some(id) = &study {
                a.delete(Level::Study, id).await.unwrap();
            }
        };
        tokio::join!(deleting, store_all(&b, &series));
        agree(&a).await;
    }
    // every deletion is in the trash, whatever the race
    assert!(!a.index().trash().await.unwrap().is_empty());
}
