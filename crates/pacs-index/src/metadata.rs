//! User metadata on patients, studies, series and instances: a text value under a name a
//! user or a tool chooses (the REST API's `/metadata`). Kept by the resource's public ID;
//! a deletion takes the metadata of what it removed ([`Index::delete`]).

use std::collections::BTreeMap;

use sqlx::AnyConnection;

use crate::{Error, Index, Level};

impl Index {
    /// A resource's user metadata, by name.
    pub async fn metadata_values(
        &self,
        public_id: &str,
    ) -> Result<BTreeMap<String, String>, Error> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT name, value FROM metadata WHERE public_id = $1")
                .bind(public_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().collect())
    }

    /// Set `name` of a resource of `level` (already known to be there) to `value`.
    pub async fn set_metadata(
        &self,
        level: Level,
        public_id: &str,
        name: &str,
        value: &str,
    ) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "INSERT INTO metadata (public_id, level, name, value) VALUES ($1, $2, $3, $4)
             ON CONFLICT (public_id, name) DO UPDATE SET value = excluded.value",
        )
        .bind(public_id)
        .bind(level.name())
        .bind(name)
        .bind(value)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Remove `name` of a resource; nothing when it has none.
    pub async fn remove_metadata(&self, public_id: &str, name: &str) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        sqlx::query("DELETE FROM metadata WHERE public_id = $1 AND name = $2")
            .bind(public_id)
            .bind(name)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Remove the metadata of resources a deletion took out. Run in the deletion's transaction.
pub(crate) async fn drop_orphans(conn: &mut AnyConnection) -> Result<(), Error> {
    sqlx::query(
        "DELETE FROM metadata WHERE
            (level = 'Instance' AND NOT EXISTS
                (SELECT 1 FROM instances i WHERE i.public_id = metadata.public_id))
         OR (level = 'Series' AND NOT EXISTS
                (SELECT 1 FROM series se WHERE se.public_id = metadata.public_id))
         OR (level = 'Study' AND NOT EXISTS
                (SELECT 1 FROM studies st WHERE st.public_id = metadata.public_id))
         OR (level = 'Patient' AND NOT EXISTS
                (SELECT 1 FROM studies st WHERE st.patient_public_id = metadata.public_id))",
    )
    .execute(conn)
    .await?;
    Ok(())
}
