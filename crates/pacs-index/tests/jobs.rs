//! The jobs kept in the index, on every backend.

mod common;

use pacs_index::KEPT_FINISHED;
use serde_json::json;

#[tokio::test]
async fn a_job_is_kept_from_its_start_to_its_end() {
    for index in common::indexes().await {
        let studies = vec!["1.2.3".to_owned(), "1.2.4".to_owned()];
        let id = index
            .create_job("C-MOVE", "qr", "PACSRS", &studies)
            .await
            .unwrap();
        let job = index.job(id).await.unwrap().unwrap();
        assert_eq!(
            (job.state.as_str(), job.studies.clone()),
            ("Running", studies.clone())
        );
        assert_eq!(
            (job.restarts, job.cancel_asked, job.finished_ms),
            (0, false, None)
        );
        assert_eq!(index.running_jobs().await.unwrap().len(), 1);

        assert!(
            !index
                .job_progress(id, &json!({"Completed": 2}))
                .await
                .unwrap()
        );
        assert_eq!(
            index.job(id).await.unwrap().unwrap().progress["Completed"],
            2
        );
        let asked = index.ask_cancel(id).await.unwrap().unwrap();
        assert!(asked.cancel_asked);
        assert!(
            index
                .job_progress(id, &json!({"Completed": 3}))
                .await
                .unwrap()
        );

        index.restart_job(id).await.unwrap();
        let restarted = index.job(id).await.unwrap().unwrap();
        assert_eq!(
            (restarted.restarts, restarted.progress.clone()),
            (1, json!({}))
        );

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
        assert_eq!(index.running_jobs().await.unwrap(), []);
        // over: a cancel changes nothing, a restart neither
        index.ask_cancel(id).await.unwrap();
        index.restart_job(id).await.unwrap();
        assert_eq!(index.job(id).await.unwrap().unwrap().restarts, 1);
        assert_eq!(index.ask_cancel(999_999).await.unwrap(), None);
        assert_eq!(index.job(999_999).await.unwrap(), None);
    }
}

#[tokio::test]
async fn only_the_latest_finished_jobs_are_kept_and_running_ones_always() {
    for index in common::indexes().await {
        let running = index
            .create_job("C-MOVE", "qr", "PACSRS", &[])
            .await
            .unwrap();
        let mut finished = Vec::new();
        for _ in 0..KEPT_FINISHED + 5 {
            let id = index
                .create_job("C-MOVE", "qr", "PACSRS", &[])
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
