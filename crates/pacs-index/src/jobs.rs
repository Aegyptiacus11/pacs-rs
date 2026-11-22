//! The moves running in the background, kept in the index: what the server's `/jobs` show,
//! and what a restart takes up again. Running them is the server's business.

use serde_json::Value;
use sqlx::Row;
use sqlx::any::AnyRow;

use crate::{Error, Index, now_ms};

/// Finished jobs kept; older ones are removed as others finish.
pub const KEPT_FINISHED: i64 = 100;

/// A job as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredJob {
    pub id: i64,
    pub kind: String,
    pub remote: String,
    pub destination: String,
    /// The Patient ID the studies are named under, for a remote AE in Patient Root.
    pub patient_id: Option<String>,
    pub studies: Vec<String>,
    /// `Running`, `Success`, `Warning`, `Cancelled` or `Failure`.
    pub state: String,
    /// The counts of the remote AE's last response, as the job shows them.
    pub progress: Value,
    pub failed_instances: Vec<String>,
    pub error: Option<String>,
    pub cancel_asked: bool,
    pub restarts: i64,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
}

fn job(row: &AnyRow) -> Result<StoredJob, Error> {
    let json = |column: &str| -> Result<Value, Error> {
        let text: String = row.try_get(column)?;
        Ok(serde_json::from_str(&text)?)
    };
    let list =
        |column: &str| -> Result<Vec<String>, Error> { Ok(serde_json::from_value(json(column)?)?) };
    Ok(StoredJob {
        id: row.try_get("id")?,
        kind: row.try_get("kind")?,
        remote: row.try_get("remote")?,
        destination: row.try_get("destination")?,
        patient_id: row.try_get("patient_id")?,
        studies: list("studies")?,
        state: row.try_get("state")?,
        progress: json("progress")?,
        failed_instances: list("failed_instances")?,
        error: row.try_get("error")?,
        cancel_asked: row.try_get::<i64, _>("cancel_asked")? != 0,
        restarts: row.try_get("restarts")?,
        started_ms: row.try_get("started_ms")?,
        finished_ms: row.try_get("finished_ms")?,
    })
}

const COLUMNS: &str = "id, kind, remote, destination, patient_id, studies, state, progress, \
                       failed_instances, error, cancel_asked, restarts, started_ms, finished_ms";

fn select(rest: &'static str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM jobs {rest}"))
}

impl Index {
    /// A new running job; its id.
    pub async fn create_job(
        &self,
        kind: &str,
        remote: &str,
        destination: &str,
        patient_id: Option<&str>,
        studies: &[String],
    ) -> Result<i64, Error> {
        let mut tx = self.begin_write().await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO jobs (kind, remote, destination, patient_id, studies, state, progress,
                               failed_instances, error, cancel_asked, restarts, started_ms,
                               heartbeat_ms)
             VALUES ($1, $2, $3, $4, $5, 'Running', '{}', '[]', NULL, 0, 0, $6, $6) RETURNING id",
        )
        .bind(kind)
        .bind(remote)
        .bind(destination)
        .bind(patient_id)
        .bind(Value::from(studies).to_string())
        .bind(now_ms())
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn job(&self, id: i64) -> Result<Option<StoredJob>, Error> {
        let row = sqlx::query(select("WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(job).transpose()
    }

    /// Every job kept, oldest first.
    pub async fn jobs(&self) -> Result<Vec<StoredJob>, Error> {
        let rows = sqlx::query(select("ORDER BY id"))
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(job).collect()
    }

    /// The jobs running whose heartbeat stopped before `before_ms`: those of a server that
    /// stopped.
    pub async fn stale_jobs(&self, before_ms: i64) -> Result<Vec<StoredJob>, Error> {
        let rows = sqlx::query(select(
            "WHERE state = 'Running' AND (heartbeat_ms IS NULL OR heartbeat_ms < $1) ORDER BY id",
        ))
        .bind(before_ms)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(job).collect()
    }

    /// The server running job `id` is alive: its heartbeat, with its counts when there are
    /// new ones. Whether a cancel was asked of it.
    pub async fn job_heartbeat(&self, id: i64, progress: Option<&Value>) -> Result<bool, Error> {
        let mut tx = self.begin_write().await?;
        match progress {
            Some(progress) => {
                sqlx::query(
                    "UPDATE jobs SET progress = $1, heartbeat_ms = $2
                     WHERE id = $3 AND state = 'Running'",
                )
                .bind(progress.to_string())
                .bind(now_ms())
                .bind(id)
                .execute(&mut *tx)
                .await?
            }
            None => {
                sqlx::query("UPDATE jobs SET heartbeat_ms = $1 WHERE id = $2 AND state = 'Running'")
                    .bind(now_ms())
                    .bind(id)
                    .execute(&mut *tx)
                    .await?
            }
        };
        let asked: Option<i64> = sqlx::query_scalar("SELECT cancel_asked FROM jobs WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(asked.unwrap_or(0) != 0)
    }

    /// Ask a running job to stop: the job as it is, or `None` when there is no such job. A
    /// job over already is returned unchanged.
    pub async fn ask_cancel(&self, id: i64) -> Result<Option<StoredJob>, Error> {
        let mut tx = self.begin_write().await?;
        sqlx::query("UPDATE jobs SET cancel_asked = 1 WHERE id = $1 AND state = 'Running'")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query(select("WHERE id = $1"))
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        tx.commit().await?;
        row.as_ref().map(job).transpose()
    }

    /// Claim a job a stopped server left running, to take it up or end it: `true` for the
    /// one server that gets it, its heartbeat now this server's, its counts reset, one
    /// restart more. `false`: alive again, over, or claimed by another server first.
    pub async fn claim_job(&self, id: i64, stale_before_ms: i64) -> Result<bool, Error> {
        let mut tx = self.begin_write().await?;
        let claimed = sqlx::query(
            "UPDATE jobs SET heartbeat_ms = $1, restarts = restarts + 1, progress = '{}',
                    failed_instances = '[]'
             WHERE id = $2 AND state = 'Running'
               AND (heartbeat_ms IS NULL OR heartbeat_ms < $3)",
        )
        .bind(now_ms())
        .bind(id)
        .bind(stale_before_ms)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(claimed == 1)
    }

    /// The end of a job; the finished jobs past [`KEPT_FINISHED`] go.
    pub async fn finish_job(
        &self,
        id: i64,
        state: &str,
        progress: &Value,
        failed_instances: &[String],
        error: Option<&str>,
    ) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "UPDATE jobs SET state = $1, progress = $2, failed_instances = $3, error = $4,
                    finished_ms = $5
             WHERE id = $6",
        )
        .bind(state)
        .bind(progress.to_string())
        .bind(Value::from(failed_instances).to_string())
        .bind(error)
        .bind(now_ms())
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "DELETE FROM jobs WHERE state <> 'Running' AND id NOT IN
               (SELECT id FROM jobs WHERE state <> 'Running' ORDER BY id DESC LIMIT $1)",
        )
        .bind(KEPT_FINISHED)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
