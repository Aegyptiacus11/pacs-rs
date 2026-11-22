//! The studies to forward by a rule (`PACS_FORWARD`), and those with results to write back
//! to the hospital: noted with each instance recorded for them
//! ([`crate::NewInstance::forwarded`], [`crate::NewInstance::a_result`]), taken out once
//! quiet. Sending them is the server's business.

use sqlx::Row;

use crate::{Error, Index};

impl Index {
    /// Take out the studies whose latest instance was recorded at `before_ms` or earlier:
    /// those quiet long enough to be forwarded. Each is taken by one caller only, of the
    /// servers sharing the index; one that takes in an instance afterwards is noted again.
    pub async fn claim_forwards(&self, before_ms: i64) -> Result<Vec<String>, Error> {
        let mut tx = self.begin_write().await?;
        let rows = sqlx::query(
            "DELETE FROM forward_due WHERE touched_ms <= $1 RETURNING study_instance_uid",
        )
        .bind(before_ms)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut studies = rows
            .iter()
            .map(|row| row.try_get::<String, _>("study_instance_uid"))
            .collect::<Result<Vec<_>, _>>()?;
        studies.sort();
        Ok(studies)
    }

    /// Take out the studies with results to write back whose latest result was recorded at
    /// `before_ms` or earlier, as [`Index::claim_forwards`] does for forwarding.
    pub async fn claim_write_backs(&self, before_ms: i64) -> Result<Vec<String>, Error> {
        let mut tx = self.begin_write().await?;
        let rows = sqlx::query(
            "DELETE FROM write_back_due WHERE touched_ms <= $1 RETURNING study_instance_uid",
        )
        .bind(before_ms)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut studies = rows
            .iter()
            .map(|row| row.try_get::<String, _>("study_instance_uid"))
            .collect::<Result<Vec<_>, _>>()?;
        studies.sort();
        Ok(studies)
    }

    /// The results of `project`'s de-identified study: its instances that are not part of
    /// the copy (whose SOP Instance UID the project did not make by replacing one).
    pub async fn results_of(
        &self,
        project: &str,
        study_instance_uid: &str,
    ) -> Result<Vec<crate::InstanceEntry>, Error> {
        let mut results = Vec::new();
        for series in self.series(study_instance_uid, None).await? {
            for instance in self
                .instances(study_instance_uid, &series.series_instance_uid, None)
                .await?
            {
                let made: Option<String> = sqlx::query_scalar(
                    "SELECT original FROM project_uids WHERE project = $1 AND replaced = $2",
                )
                .bind(project)
                .bind(&instance.sop_instance_uid)
                .fetch_optional(&self.pool)
                .await?;
                if made.is_none() {
                    results.push(instance);
                }
            }
        }
        Ok(results)
    }
}
