//! What the index keeps of each file's checks, on every backend.

mod common;

use pacs_core::Instance;
use pacs_core::real::RealSeries;
use pacs_index::{CheckCounts, Index, NewInstance};

/// The series recorded: the first slice without a hash (stored before hashes were kept),
/// the others with one.
async fn record(index: &Index, series: &RealSeries) -> Vec<String> {
    let mut uids = Vec::new();
    for (slice, bytes) in series.all_bytes().iter().enumerate() {
        let instance = Instance::from_bytes(bytes).unwrap();
        let mut new = NewInstance::new(&instance, format!("k/{slice}"), bytes.len()).unwrap();
        if slice > 0 {
            new = new.with_sha256(format!("hash-{slice}"));
        }
        uids.push(new.sop_instance_uid.clone());
        index.record(&new).await.unwrap();
    }
    uids
}

#[tokio::test]
async fn a_file_is_due_when_its_last_check_is_old_and_one_never_checked_first() {
    for index in common::indexes().await {
        let series = RealSeries::new(31, 3);
        let uids = record(&index, &series).await;
        // hashed at storage: checked then; the one stored without a hash never was
        assert_eq!(
            index.check_counts().await.unwrap(),
            CheckCounts {
                files: 3,
                never_checked: 1,
                failed: 0
            }
        );
        let far_future = i64::MAX;
        let due = index.files_due(far_future, 10).await.unwrap();
        assert_eq!(due.len(), 3);
        // never checked: due at once, and first
        assert_eq!(due[0].sop_instance_uid, uids[0]);
        let now_due = index.files_due(0, 10).await.unwrap();
        assert_eq!(now_due.len(), 1);
        assert_eq!(now_due[0].sop_instance_uid, uids[0]);
        assert_eq!(index.files_due(far_future, 2).await.unwrap().len(), 2);
        let files = index.files_of_study(&series.study_uid()).await.unwrap();
        assert_eq!(files.len(), 3);
        let first = files
            .iter()
            .find(|f| f.sop_instance_uid == uids[0])
            .unwrap();
        assert_eq!(first.sha256, None);
        assert_eq!(first.object_key, "k/0");

        // a page at a time, in UID order, to the end
        let mut swept = Vec::new();
        let mut after = String::new();
        loop {
            let page = index.files_after(&after, 2).await.unwrap();
            let Some(last) = page.last() else { break };
            after = last.sop_instance_uid.clone();
            swept.extend(page.into_iter().map(|f| f.sop_instance_uid));
        }
        let mut sorted = uids.clone();
        sorted.sort();
        assert_eq!(swept, sorted);
        assert_eq!(index.files_of_study("1.2.3").await.unwrap(), []);
    }
}

#[tokio::test]
async fn a_check_records_its_failure_and_takes_a_missing_hash_only() {
    for index in common::indexes().await {
        let series = RealSeries::new(32, 2);
        let uids = record(&index, &series).await;
        // the file stored without a hash gets one at its first check
        index
            .record_check(&uids[0], Some("taken"), None)
            .await
            .unwrap();
        // one with a hash keeps it, whatever a check sends
        index
            .record_check(&uids[1], Some("other"), Some("changed"))
            .await
            .unwrap();
        let files = index.files_of_study(&series.study_uid()).await.unwrap();
        let hash = |uid: &str| {
            files
                .iter()
                .find(|f| f.sop_instance_uid == uid)
                .unwrap()
                .sha256
                .clone()
        };
        assert_eq!(hash(&uids[0]).as_deref(), Some("taken"));
        assert_eq!(hash(&uids[1]).as_deref(), Some("hash-1"));

        let failed = index.failed_files().await.unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].sop_instance_uid, uids[1]);
        assert_eq!(failed[0].study_instance_uid, series.study_uid());
        assert_eq!(failed[0].failure, "changed");
        assert!(failed[0].checked_ms > 0);
        assert_eq!(index.check_counts().await.unwrap().failed, 1);

        // a check that passes clears the failure
        index.record_check(&uids[1], None, None).await.unwrap();
        assert_eq!(index.failed_files().await.unwrap(), []);
        // an instance gone meanwhile: nothing to record, no error
        index
            .record_check("1.2.3.4", None, Some("x"))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn storing_again_takes_the_new_files_hash_and_clears_its_failure() {
    for index in common::indexes().await {
        let series = RealSeries::new(33, 1);
        let uids = record(&index, &series).await;
        index
            .record_check(&uids[0], None, Some("missing"))
            .await
            .unwrap();
        let bytes = &series.all_bytes()[0];
        let instance = Instance::from_bytes(bytes).unwrap();
        let again = NewInstance::new(&instance, "k/new".into(), bytes.len())
            .unwrap()
            .with_sha256("new-hash".into());
        index.record(&again).await.unwrap();
        let files = index.files_of_study(&series.study_uid()).await.unwrap();
        assert_eq!(files[0].sha256.as_deref(), Some("new-hash"));
        assert_eq!(index.failed_files().await.unwrap(), []);
    }
}
