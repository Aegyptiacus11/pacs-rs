//! The studies to forward by a rule (`PACS_FORWARD`): noted with each instance recorded for
//! it ([`crate::NewInstance::forwarded`]), taken out once quiet. Forwarding them is the
//! server's business.

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
}
