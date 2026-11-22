//! The jobs kept in the index, on every backend.

mod common;

use pacs_index::KEPT_FINISHED;
use serde_json::json;

#[tokio::test]
async fn a_job_is_kept_from_its_start_to_its_end() {
    for index in common::indexes().await {
        let studies = vec!["1.2.3".to_owned(), "1.2.4".to_owned()];
        let id = index
            .create_job("C-MOVE", "qr", "PACSRS", Some("NEMA-1"), &studies)
            .await
            .unwrap();
        let job = index.job(id).await.unwrap().unwrap();
        assert_eq!(
            (job.state.as_str(), job.studies.clone()),
            ("Running", studies.clone())
        );
        assert_eq!(job.patient_id.as_deref(), Some("NEMA-1"));
        // a move that gave way to a get says so
        index.job_kind(id, "C-GET").await.unwrap();
        assert_eq!(index.job(id).await.unwrap().unwrap().kind, "C-GET");
        assert_eq!(
            (job.restarts, job.cancel_asked, job.finished_ms),
            (0, false, None)
        );
        // just started: alive, not stale
        assert_eq!(index.stale_jobs(job.started_ms - 1).await.unwrap(), []);
        assert_eq!(index.stale_jobs(i64::MAX).await.unwrap().len(), 1);

        assert!(
            !index
                .job_heartbeat(id, Some(&json!({"Completed": 2})))
                .await
                .unwrap()
        );
        assert_eq!(
            index.job(id).await.unwrap().unwrap().progress["Completed"],
            2
        );
        let asked = index.ask_cancel(id).await.unwrap().unwrap();
        assert!(asked.cancel_asked);
        assert!(index.job_heartbeat(id, None).await.unwrap());

        // claimed once its heartbeat is older than asked; claimed, it is fresh again
        assert!(index.claim_job(id, i64::MAX).await.unwrap());
        assert!(!index.claim_job(id, 0).await.unwrap());
        let claimed = index.job(id).await.unwrap().unwrap();
        assert_eq!((claimed.restarts, claimed.progress.clone()), (1, json!({})));

        index
            .finish_job(
                id,
                "Cancelled",
                &json!({"Completed": 3}),
                &["9.9".into()],
                None,
            )
            .await
            .unwrap();
        let over = index.job(id).await.unwrap().unwrap();
        assert_eq!(over.state, "Cancelled");
        assert_eq!(over.failed_instances, ["9.9"]);
        assert!(over.finished_ms.is_some());
        assert_eq!(index.stale_jobs(i64::MAX).await.unwrap(), []);
        // over: a cancel changes nothing, a claim neither
        index.ask_cancel(id).await.unwrap();
        assert!(!index.claim_job(id, i64::MAX).await.unwrap());
        assert_eq!(index.job(id).await.unwrap().unwrap().restarts, 1);
        assert_eq!(index.ask_cancel(999_999).await.unwrap(), None);
        assert_eq!(index.job(999_999).await.unwrap(), None);
    }
}

#[tokio::test]
async fn only_the_latest_finished_jobs_are_kept_and_running_ones_always() {
    for index in common::indexes().await {
        let running = index
            .create_job("C-MOVE", "qr", "PACSRS", None, &[])
            .await
            .unwrap();
        let mut finished = Vec::new();
        for _ in 0..KEPT_FINISHED + 5 {
            let id = index
                .create_job("C-MOVE", "qr", "PACSRS", None, &[])
                .await
                .unwrap();
            index
                .finish_job(id, "Success", &json!({}), &[], None)
                .await
                .unwrap();
            finished.push(id);
        }
        let kept: Vec<i64> = index.jobs().await.unwrap().iter().map(|j| j.id).collect();
        assert_eq!(kept.len() as i64, KEPT_FINISHED + 1);
        assert_eq!(kept[0], running);
        assert!(!kept.contains(&finished[4]) && kept.contains(&finished[5]));
    }
}

/// A study noted for forwarding with its instance is taken out once quiet, by one caller,
/// and noted again by its next instance.
#[tokio::test]
async fn a_quiet_study_is_taken_out_for_forwarding_once() {
    use pacs_core::real::RealSeries;
    for index in common::indexes().await {
        let series = RealSeries::new(81, 2);
        let instance = |slice| {
            let bytes = series.slice_bytes(slice);
            let instance = pacs_core::Instance::from_bytes(&bytes).unwrap();
            pacs_index::NewInstance::new(&instance, format!("k{slice}"), bytes.len())
                .unwrap()
                .forwarded(true)
        };
        let other = RealSeries::new(82, 1);
        let unforwarded = {
            let bytes = other.slice_bytes(0);
            let instance = pacs_core::Instance::from_bytes(&bytes).unwrap();
            pacs_index::NewInstance::new(&instance, "o".into(), bytes.len()).unwrap()
        };
        index.record(&instance(0)).await.unwrap();
        index.record(&unforwarded).await.unwrap();
        // not quiet yet
        assert!(index.claim_forwards(0).await.unwrap().is_empty());
        let (a, b) = tokio::join!(
            index.claim_forwards(i64::MAX),
            index.claim_forwards(i64::MAX)
        );
        let mut taken = a.unwrap();
        taken.extend(b.unwrap());
        assert_eq!(taken, [series.study_uid()]);
        // the next instance notes it again
        index.record(&instance(1)).await.unwrap();
        assert_eq!(
            index.claim_forwards(i64::MAX).await.unwrap(),
            [series.study_uid()]
        );
    }
}
