//! Labels on patients, studies, series and instances: free words put on a resource to group
//! and find it (the REST API's `/labels`). The index keeps them by the resource's public ID;
//! a deletion takes the labels of what it removed ([`Index::delete`]).

use std::collections::HashMap;

use sqlx::AnyConnection;

use crate::{Error, Index, Level};

/// The longest label.
pub const LABEL_MAX: usize = 64;

/// Whether `label` may be a label: 1 to [`LABEL_MAX`] characters, each an ASCII letter or
/// digit, `_` or `-`.
pub fn is_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= LABEL_MAX
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl Index {
    /// The labels of a resource, sorted.
    pub async fn labels(&self, public_id: &str) -> Result<Vec<String>, Error> {
        Ok(
            sqlx::query_scalar("SELECT label FROM labels WHERE public_id = $1 ORDER BY label")
                .bind(public_id)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// The labels of each of `public_ids` that has some, each list sorted.
    pub async fn labels_of(
        &self,
        public_ids: &[String],
    ) -> Result<HashMap<String, Vec<String>>, Error> {
        let mut found: HashMap<String, Vec<String>> = HashMap::new();
        for chunk in public_ids.chunks(500) {
            let list = (1..=chunk.len())
                .map(|n| format!("${n}"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT public_id, label FROM labels WHERE public_id IN ({list})
                  ORDER BY public_id, label"
            );
            let mut query = sqlx::query_as::<_, (String, String)>(sqlx::AssertSqlSafe(sql));
            for id in chunk {
                query = query.bind(id);
            }
            for (id, label) in query.fetch_all(&self.pool).await? {
                found.entry(id).or_default().push(label);
            }
        }
        Ok(found)
    }

    /// Every label a resource this value sees has, sorted, each once: for a value limited to
    /// an account, the labels on the others are not told.
    pub async fn all_labels(&self) -> Result<Vec<String>, Error> {
        Ok(sqlx::query_scalar(concat!(
            "SELECT DISTINCT l.label FROM labels l WHERE CAST($1 AS TEXT) IS NULL
                OR (l.level = 'Patient' AND EXISTS (SELECT 1 FROM studies st
                      WHERE st.patient_public_id = l.public_id AND",
            seen!("st", "1"),
            "))
                OR (l.level = 'Study' AND EXISTS (SELECT 1 FROM studies st
                      WHERE st.public_id = l.public_id AND",
            seen!("st", "1"),
            "))
                OR (l.level = 'Series' AND EXISTS (SELECT 1 FROM series se
                      JOIN studies st ON st.study_instance_uid = se.study_instance_uid
                      WHERE se.public_id = l.public_id AND",
            seen!("st", "1"),
            "))
                OR (l.level = 'Instance' AND EXISTS (SELECT 1 FROM instances i
                      JOIN series se USING (series_instance_uid)
                      JOIN studies st ON st.study_instance_uid = se.study_instance_uid
                      WHERE i.public_id = l.public_id AND",
            seen!("st", "1"),
            "))
              ORDER BY l.label"
        ))
        .bind(self.account())
        .fetch_all(&self.pool)
        .await?)
    }

    /// The public IDs of `level` that have the labels `labels` ask, by `constraint`: all of
    /// them, any of them. Not bounded by an account: the caller reads each resource found
    /// as its account sees it.
    pub async fn labelled(
        &self,
        level: Level,
        labels: &[String],
        all: bool,
    ) -> Result<Vec<String>, Error> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for label in labels {
            let ids: Vec<String> =
                sqlx::query_scalar("SELECT public_id FROM labels WHERE level = $1 AND label = $2")
                    .bind(level.name())
                    .bind(label)
                    .fetch_all(&self.pool)
                    .await?;
            for id in ids {
                *counts.entry(id).or_default() += 1;
            }
        }
        let mut ids: Vec<String> = counts
            .into_iter()
            .filter(|(_, n)| !all || *n == labels.len())
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        Ok(ids)
    }

    /// Put `label` on the resource of `level` (already known to be there); nothing when it
    /// has it.
    pub async fn add_label(&self, level: Level, public_id: &str, label: &str) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "INSERT INTO labels (public_id, level, label) VALUES ($1, $2, $3)
             ON CONFLICT (public_id, label) DO NOTHING",
        )
        .bind(public_id)
        .bind(level.name())
        .bind(label)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Take `label` off a resource; nothing when it does not have it.
    pub async fn remove_label(&self, public_id: &str, label: &str) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        sqlx::query("DELETE FROM labels WHERE public_id = $1 AND label = $2")
            .bind(public_id)
            .bind(label)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Remove the labels of resources a deletion took out: those whose public ID no resource of
/// their level has any more. Run in the deletion's transaction.
pub(crate) async fn drop_orphans(conn: &mut AnyConnection) -> Result<(), Error> {
    sqlx::query(
        "DELETE FROM labels WHERE
            (level = 'Instance' AND NOT EXISTS
                (SELECT 1 FROM instances i WHERE i.public_id = labels.public_id))
         OR (level = 'Series' AND NOT EXISTS
                (SELECT 1 FROM series se WHERE se.public_id = labels.public_id))
         OR (level = 'Study' AND NOT EXISTS
                (SELECT 1 FROM studies st WHERE st.public_id = labels.public_id))
         OR (level = 'Patient' AND NOT EXISTS
                (SELECT 1 FROM studies st WHERE st.patient_public_id = labels.public_id))",
    )
    .execute(conn)
    .await?;
    Ok(())
}
