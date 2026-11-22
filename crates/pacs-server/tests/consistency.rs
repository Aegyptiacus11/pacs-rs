//! `pacs-server --consistency [--repair]` on a real series: orphans (indexed again, or left
//! when they are not DICOM or a copy), lost entries (removed, in the audit trail), recent
//! files left out, and a quarantine inside the store refused.

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use pacs_archive::Archive;
use pacs_core::real::RealSeries;
use pacs_server::audit::{Audit, open_audit};
use pacs_server::consistency::{check_quarantine, run};
use pacs_server::{Settings, open_archive};

struct Stored {
    archive: Archive,
    audit: Audit,
    files: PathBuf,
    audit_file: PathBuf,
    series: RealSeries,
    _dir: tempfile::TempDir,
}

async fn stored(slices: u32) -> Stored {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index.sqlite").display().to_string();
    let files = dir.path().join("files");
    let audit_file = dir.path().join("audit.log");
    let storage = format!("file:{}", files.display());
    let audit_path = audit_file.display().to_string();
    let settings = Settings::try_parse_from([
        "pacs-server",
        "--index",
        &index,
        "--storage",
        &storage,
        "--audit-file",
        &audit_path,
    ])
    .unwrap();
    let archive = open_archive(&settings).await.unwrap();
    let series = RealSeries::new(95, slices);
    for bytes in series.all_bytes() {
        archive.ingest(bytes.into()).await.unwrap();
    }
    Stored {
        audit: open_audit(&settings).unwrap(),
        archive,
        files,
        audit_file,
        series,
        _dir: dir,
    }
}

impl Stored {
    async fn key_of(&self, slice: u32) -> String {
        self.archive
            .index()
            .instance_file(&self.series.instance_uid(slice))
            .await
            .unwrap()
            .unwrap()
            .1
    }

    /// The comparison, as the command runs it, with no grace: what it says, and whether the
    /// two agree.
    async fn run(&self, repair: bool) -> (bool, String) {
        let mut out = Vec::new();
        let agree = run(&self.archive, &self.audit, repair, Duration::ZERO, &mut out)
            .await
            .unwrap();
        (agree, String::from_utf8(out).unwrap())
    }

    async fn indexed(&self, slice: u32) -> bool {
        self.archive
            .index()
            .instance_file(&self.series.instance_uid(slice))
            .await
            .unwrap()
            .is_some()
    }
}

#[tokio::test]
async fn a_store_and_an_index_that_agree() {
    let s = stored(2).await;
    let (agree, said) = s.run(false).await;
    assert!(agree, "{said}");
    assert!(
        said.contains("2 objects in the store, 2 index entries"),
        "{said}"
    );
    assert!(said.contains("0 orphans, 0 lost"), "{said}");
}

#[tokio::test]
async fn an_orphan_that_reads_as_dicom_is_indexed_again() {
    let s = stored(3).await;
    // the entry lost, as by a crash between storing the file and indexing it
    let key = s.key_of(1).await;
    sqlx::query("DELETE FROM instances WHERE sop_instance_uid = $1")
        .bind(s.series.instance_uid(1))
        .execute(s.archive.index().pool())
        .await
        .unwrap();
    let (agree, said) = s.run(false).await;
    assert!(!agree);
    assert!(said.contains(&format!("orphan: {key}")), "{said}");
    // listing changes nothing
    assert!(!s.indexed(1).await);

    let (agree, said) = s.run(true).await;
    assert!(agree, "{said}");
    assert!(
        said.contains(&format!("orphan indexed again: {key}")),
        "{said}"
    );
    assert!(s.indexed(1).await);
    assert_eq!(s.key_of(1).await, key);
    assert!(s.run(false).await.0);
}

#[tokio::test]
async fn an_orphan_that_is_not_dicom_or_is_a_copy_is_left_where_it_is() {
    let s = stored(1).await;
    let junk = s.files.join("notes.txt");
    std::fs::write(&junk, b"not an instance").unwrap();
    let original = s.files.join(s.key_of(0).await);
    let copy = s.files.join("elsewhere/copy.dcm");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::copy(&original, &copy).unwrap();

    let (agree, said) = s.run(true).await;
    assert!(!agree, "{said}");
    assert!(said.contains("orphan left: notes.txt: not DICOM"), "{said}");
    assert!(
        said.contains("orphan left: elsewhere/copy.dcm: a copy of an instance indexed at"),
        "{said}"
    );
    assert!(
        said.contains("2 orphans, 0 lost; 2 left as they are"),
        "{said}"
    );
    // repair never deletes a file
    assert!(junk.is_file() && copy.is_file());
}

#[tokio::test]
async fn a_lost_entry_is_removed_and_the_audit_trail_says_so() {
    let s = stored(2).await;
    let key = s.key_of(0).await;
    std::fs::remove_file(s.files.join(&key)).unwrap();
    let (agree, said) = s.run(false).await;
    assert!(!agree);
    assert!(
        said.contains(&format!("lost: {}", s.series.instance_uid(0))),
        "{said}"
    );
    assert!(s.indexed(0).await);

    let (agree, said) = s.run(true).await;
    assert!(agree, "{said}");
    assert!(said.contains("lost entry removed"), "{said}");
    assert!(!s.indexed(0).await);
    assert!(s.indexed(1).await);
    // the study is still there, a part less: Instances Accessed, action Delete
    let trail = std::fs::read_to_string(&s.audit_file).unwrap();
    assert!(trail.contains("110103"), "{trail}");
    assert!(trail.contains(&s.series.study_uid()), "{trail}");

    // the last file gone too: the study goes, a Study Deleted
    std::fs::remove_file(s.files.join(s.key_of(1).await)).unwrap();
    let (agree, _) = s.run(true).await;
    assert!(agree);
    let trail = std::fs::read_to_string(&s.audit_file).unwrap();
    assert!(trail.contains("110105"), "{trail}");
    assert!(
        s.archive
            .index()
            .audited_study(&s.series.study_uid())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_file_written_within_the_grace_time_is_not_judged() {
    let s = stored(1).await;
    std::fs::write(s.files.join("being-stored.dcm"), b"x").unwrap();
    let mut out = Vec::new();
    let agree = run(
        &s.archive,
        &s.audit,
        true,
        Duration::from_secs(600),
        &mut out,
    )
    .await
    .unwrap();
    let said = String::from_utf8(out).unwrap();
    assert!(agree, "{said}");
    assert!(said.contains("1 written too recently to judge"), "{said}");
}

#[test]
fn a_quarantine_inside_the_store_is_refused_and_repair_needs_the_comparison() {
    let with = |quarantine: &str| {
        Settings::try_parse_from([
            "pacs-server",
            "--storage",
            "file:/data/files",
            "--quarantine",
            quarantine,
        ])
        .unwrap()
    };
    assert!(check_quarantine(&with("file:/data/files")).is_err());
    assert!(check_quarantine(&with("file:/data/files/quarantine")).is_err());
    assert!(check_quarantine(&with("file:/data/quarantine")).is_ok());
    assert!(check_quarantine(&with("file:/data/files-quarantine")).is_ok());
    assert!(Settings::try_parse_from(["pacs-server", "--repair"]).is_err());
    assert!(Settings::try_parse_from(["pacs-server", "--consistency", "--repair"]).is_ok());
}
