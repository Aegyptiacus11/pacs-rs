//! The web interface's measurements: one JSON list per study, by Study Instance UID. Like the
//! hanging protocols, what the list says is the interface's business; the index only keeps it,
//! and drops it with the study.

use sqlx::Row;

use crate::{Error, Index, now_ms};

impl Index {
    /// The measurements kept for a study, as JSON text; `None` when it has none.
    pub async fn annotations(&self, study_instance_uid: &str) -> Result<Option<String>, Error> {
        let row = sqlx::query("SELECT body FROM ui_annotations WHERE study_instance_uid = $1")
            .bind(study_instance_uid)
            .fetch_optional(&self.pool)
            .await?;
        Ok(match row {
            Some(row) => Some(row.try_get("body")?),
            None => None,
        })
    }

    /// Keep `body` as the study's measurements, replacing what was there; `None` removes
    /// them. Whether the study is in the index: measurements of an unknown study are refused.
    pub async fn put_annotations(
        &self,
        study_instance_uid: &str,
        body: Option<&str>,
    ) -> Result<bool, Error> {
        let known: i64 =
            sqlx::query_scalar("SELECT count(*) FROM studies WHERE study_instance_uid = $1")
                .bind(study_instance_uid)
                .fetch_one(&self.pool)
                .await?;
        if known == 0 {
            return Ok(false);
        }
        match body {
            Some(body) => {
                sqlx::query(
                    "INSERT INTO ui_annotations (study_instance_uid, body, updated_ms)
                     VALUES ($1, $2, $3)
                     ON CONFLICT (study_instance_uid)
                     DO UPDATE SET body = excluded.body, updated_ms = excluded.updated_ms",
                )
                .bind(study_instance_uid)
                .bind(body)
                .bind(now_ms())
                .execute(&self.pool)
                .await?;
            }
            None => {
                sqlx::query("DELETE FROM ui_annotations WHERE study_instance_uid = $1")
                    .bind(study_instance_uid)
                    .execute(&self.pool)
                    .await?;
            }
        }
        Ok(true)
    }
}
