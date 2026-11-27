//! A patient's instances rewritten with new patient attributes (a correction, or a merge into
//! another patient): every entry replaced by its new file in one transaction, the labels and
//! metadata moved to the public IDs that follow from the new Patient ID, and the files
//! replaced kept in a trash entry, from which the change can be undone.

use crate::{Error, Index, IntoTrash, NewInstance, Recorded, record_in, trash};

/// A resource whose public ID changed: the old ID and the new one.
pub type Rekey = (String, String);

impl Index {
    /// Replace the entries of `news`, each an instance already stored, by these, in one
    /// transaction; move the labels and metadata of every public ID that changed to its new
    /// one, and keep the files replaced in a trash entry (`trash.id` is set). Answers the IDs
    /// that changed.
    pub async fn replace_instances(
        &self,
        news: &[NewInstance],
        trash: &mut IntoTrash<'_>,
    ) -> Result<Vec<Rekey>, Error> {
        let mut tx = self.begin_write().await?;
        let mut rekeys: Vec<Rekey> = Vec::new();
        let mut replaced = Vec::with_capacity(news.len());
        for new in news {
            let old: Option<(String, String, String, String)> = sqlx::query_as(
                "SELECT st.patient_public_id, st.public_id, se.public_id, i.public_id
                 FROM instances i
                 JOIN series se ON se.series_instance_uid = i.series_instance_uid
                 JOIN studies st ON st.study_instance_uid = se.study_instance_uid
                 WHERE i.sop_instance_uid = $1",
            )
            .bind(&new.sop_instance_uid)
            .fetch_optional(&mut *tx)
            .await?;
            let Some((patient, study, series, instance)) = old else {
                return Err(Error::Conflict(format!(
                    "instance {} is no longer stored",
                    new.sop_instance_uid
                )));
            };
            match record_in(&mut tx, new).await? {
                Recorded::Replaced { previous_key } => replaced.push(previous_key),
                Recorded::Added | Recorded::Kept { .. } => {
                    return Err(Error::Conflict(format!(
                        "instance {} was not replaced",
                        new.sop_instance_uid
                    )));
                }
            }
            for pair in [
                (patient, new.patient_public_id.clone()),
                (study, new.study_public_id.clone()),
                (series, new.series_public_id.clone()),
                (instance, new.instance_public_id.clone()),
            ] {
                if pair.0 != pair.1 && !rekeys.contains(&pair) {
                    rekeys.push(pair);
                }
            }
        }
        rekey_in(&mut tx, &rekeys).await?;
        trash.id = Some(trash::keep(&mut tx, trash, &replaced).await?);
        tx.commit().await?;
        Ok(rekeys)
    }

    /// Move the labels and metadata of each old public ID to its new one, as undoing a
    /// modification needs.
    pub async fn rekey(&self, rekeys: &[Rekey]) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        rekey_in(&mut tx, rekeys).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Labels and metadata from old public IDs to new ones. What the new ID has already (a
/// patient merged into another one keeps the other's) stays: the old one's is dropped.
async fn rekey_in(conn: &mut sqlx::AnyConnection, rekeys: &[Rekey]) -> Result<(), Error> {
    for (old, new) in rekeys {
        sqlx::query(
            "INSERT INTO labels (public_id, level, label)
             SELECT $1, level, label FROM labels WHERE public_id = $2
             ON CONFLICT DO NOTHING",
        )
        .bind(new)
        .bind(old)
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM labels WHERE public_id = $1")
            .bind(old)
            .execute(&mut *conn)
            .await?;
        sqlx::query(
            "INSERT INTO metadata (public_id, level, name, value)
             SELECT $1, level, name, value FROM metadata WHERE public_id = $2
             ON CONFLICT DO NOTHING",
        )
        .bind(new)
        .bind(old)
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM metadata WHERE public_id = $1")
            .bind(old)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

impl Index {
    /// A patient's main tags (name, ID, birth date, sex...), as their latest study keeps them.
    /// `None`: no such patient.
    pub async fn patient_main_tags(
        &self,
        patient_public_id: &str,
    ) -> Result<Option<serde_json::Map<String, serde_json::Value>>, Error> {
        let tags: Option<String> = sqlx::query_scalar(
            "SELECT patient_main_tags FROM studies WHERE patient_public_id = $1
             ORDER BY study_date DESC LIMIT 1",
        )
        .bind(patient_public_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match tags {
            Some(tags) => serde_json::from_str::<serde_json::Value>(&tags)?
                .as_object()
                .cloned(),
            None => None,
        })
    }
}
