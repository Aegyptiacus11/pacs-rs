//! A patient's instances rewritten with new patient attributes (a correction, or a merge into
//! another patient): every entry replaced by its new file in one transaction, the labels and
//! metadata moved to the public IDs that follow from the new Patient ID, and the files
//! replaced kept in a trash entry, from which the change can be undone, in one transaction
//! too.

use serde_json::{Value, json};

use crate::{Error, Index, IntoTrash, NewInstance, Recorded, record_in, trash};

/// A resource whose public ID changed: the old ID and the new one.
pub type Rekey = (String, String);

/// The labels and metadata moved from a public ID that changed to its new one, each with
/// whether the new one lacked it (`added`): an undo gives the old ID all of them back and
/// takes from the new one only those it did not have. A patient merged into another keeps
/// the other's labels through an undo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub from: String,
    pub to: String,
    /// Level and label, and whether `to` lacked it.
    pub labels: Vec<(String, String, bool)>,
    /// Level, name and value, and whether `to` lacked that name.
    pub metadata: Vec<(String, String, String, bool)>,
}

impl Moved {
    pub fn to_json(&self) -> Value {
        json!({
            "from": self.from,
            "to": self.to,
            "labels": self.labels.iter().map(|(level, label, added)| {
                json!({"level": level, "label": label, "added": added})
            }).collect::<Vec<_>>(),
            "metadata": self.metadata.iter().map(|(level, name, value, added)| {
                json!({"level": level, "name": name, "value": value, "added": added})
            }).collect::<Vec<_>>(),
        })
    }

    /// The moves a trash entry keeps (`to_json` of each). A list that is not one is an error:
    /// an undo that guessed would put labels on the wrong patient.
    pub fn list_from_json(value: &Value) -> Result<Vec<Moved>, Error> {
        let bad = || {
            Error::Conflict(format!(
                "the moves the entry keeps are not readable: {value}"
            ))
        };
        let text = |v: &Value, key: &str| v[key].as_str().map(str::to_owned).ok_or_else(bad);
        let added = |v: &Value| v["added"].as_bool().ok_or_else(bad);
        let mut moves = Vec::new();
        for m in value.as_array().ok_or_else(bad)? {
            let mut labels = Vec::new();
            for l in m["labels"].as_array().ok_or_else(bad)? {
                labels.push((text(l, "level")?, text(l, "label")?, added(l)?));
            }
            let mut metadata = Vec::new();
            for d in m["metadata"].as_array().ok_or_else(bad)? {
                metadata.push((
                    text(d, "level")?,
                    text(d, "name")?,
                    text(d, "value")?,
                    added(d)?,
                ));
            }
            moves.push(Moved {
                from: text(m, "from")?,
                to: text(m, "to")?,
                labels,
                metadata,
            });
        }
        Ok(moves)
    }
}

/// Within a transaction: the entries of `news` replace those of the same SOP Instance UIDs,
/// each of which must be stored. Answers the object keys replaced and the public IDs that
/// changed.
async fn replace_in(
    tx: &mut sqlx::AnyConnection,
    news: &[NewInstance],
) -> Result<(Vec<String>, Vec<Rekey>), Error> {
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
        match record_in(&mut *tx, new).await? {
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
    Ok((replaced, rekeys))
}

impl Index {
    /// Replace the entries of `news`, each an instance already stored, by these, in one
    /// transaction; move the labels and metadata of every public ID that changed to its new
    /// one, and keep the files replaced in a trash entry (`trash.id` is set) whose first
    /// study holds the moves as `rekeyed`, written in the same transaction: a server that
    /// stops after it can still undo the change whole. Answers the moves.
    pub async fn replace_instances(
        &self,
        news: &[NewInstance],
        trash: &mut IntoTrash<'_>,
    ) -> Result<Vec<Moved>, Error> {
        let mut tx = self.begin_write().await?;
        let (replaced, rekeys) = replace_in(&mut tx, news).await?;
        let moves = move_in(&mut tx, &rekeys).await?;
        let mut studies = trash.studies.clone();
        match studies.get_mut(0).and_then(Value::as_object_mut) {
            Some(first) => {
                first.insert("rekeyed".into(), moves.iter().map(Moved::to_json).collect());
            }
            None => {
                return Err(Error::Conflict(
                    "a modification's trash entry names no study".into(),
                ));
            }
        }
        let with_moves = IntoTrash {
            studies: &studies,
            ..*trash
        };
        trash.id = Some(trash::keep(&mut tx, &with_moves, &replaced).await?);
        tx.commit().await?;
        Ok(moves)
    }

    /// Undo a modification, in one transaction: the entries of `olds` (the files of trash
    /// entry `trash_id`, stored again) replace the modified ones, the labels and metadata of
    /// `moves` go back, and the entry goes. Every modified instance must still be stored: one
    /// deleted or modified again since refuses the whole undo, and nothing changes. Answers the
    /// object keys of the modified files, which no entry names any more.
    pub async fn undo_replacement(
        &self,
        olds: &[NewInstance],
        moves: &[Moved],
        trash_id: i64,
    ) -> Result<Vec<String>, Error> {
        let mut tx = self.begin_write().await?;
        let (replaced, _) = replace_in(&mut tx, olds).await?;
        unmove_in(&mut tx, moves).await?;
        // SQLite does not hold to the foreign key unless asked to: both by hand
        sqlx::query("DELETE FROM trash_files WHERE trash_id = $1")
            .bind(trash_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM trash WHERE id = $1")
            .bind(trash_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(replaced)
    }
}

/// Labels and metadata from old public IDs to new ones, noting which the new one lacked. What
/// the new ID has already (a patient merged into another one keeps the other's) stays.
async fn move_in(conn: &mut sqlx::AnyConnection, rekeys: &[Rekey]) -> Result<Vec<Moved>, Error> {
    let mut moves = Vec::new();
    for (old, new) in rekeys {
        let labels: Vec<(String, String)> =
            sqlx::query_as("SELECT level, label FROM labels WHERE public_id = $1 ORDER BY label")
                .bind(old)
                .fetch_all(&mut *conn)
                .await?;
        let metadata: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT level, name, value FROM metadata WHERE public_id = $1 ORDER BY name",
        )
        .bind(old)
        .fetch_all(&mut *conn)
        .await?;
        if labels.is_empty() && metadata.is_empty() {
            continue;
        }
        let mut moved = Moved {
            from: old.clone(),
            to: new.clone(),
            labels: Vec::new(),
            metadata: Vec::new(),
        };
        for (level, label) in labels {
            let added = sqlx::query(
                "INSERT INTO labels (public_id, level, label) VALUES ($1, $2, $3)
                 ON CONFLICT DO NOTHING",
            )
            .bind(new)
            .bind(&level)
            .bind(&label)
            .execute(&mut *conn)
            .await?
            .rows_affected()
                == 1;
            moved.labels.push((level, label, added));
        }
        for (level, name, value) in metadata {
            let added = sqlx::query(
                "INSERT INTO metadata (public_id, level, name, value) VALUES ($1, $2, $3, $4)
                 ON CONFLICT DO NOTHING",
            )
            .bind(new)
            .bind(&level)
            .bind(&name)
            .bind(&value)
            .execute(&mut *conn)
            .await?
            .rows_affected()
                == 1;
            moved.metadata.push((level, name, value, added));
        }
        for delete in [
            "DELETE FROM labels WHERE public_id = $1",
            "DELETE FROM metadata WHERE public_id = $1",
        ] {
            sqlx::query(delete).bind(old).execute(&mut *conn).await?;
        }
        moves.push(moved);
    }
    Ok(moves)
}

/// The moves of a modification undone: each old ID gets its labels and metadata back, and the
/// new one loses those it got from it (a metadata value changed since stays).
async fn unmove_in(conn: &mut sqlx::AnyConnection, moves: &[Moved]) -> Result<(), Error> {
    for m in moves.iter().rev() {
        for (level, label, added) in &m.labels {
            sqlx::query(
                "INSERT INTO labels (public_id, level, label) VALUES ($1, $2, $3)
                 ON CONFLICT DO NOTHING",
            )
            .bind(&m.from)
            .bind(level)
            .bind(label)
            .execute(&mut *conn)
            .await?;
            if *added {
                sqlx::query("DELETE FROM labels WHERE public_id = $1 AND label = $2")
                    .bind(&m.to)
                    .bind(label)
                    .execute(&mut *conn)
                    .await?;
            }
        }
        for (level, name, value, added) in &m.metadata {
            sqlx::query(
                "INSERT INTO metadata (public_id, level, name, value) VALUES ($1, $2, $3, $4)
                 ON CONFLICT DO NOTHING",
            )
            .bind(&m.from)
            .bind(level)
            .bind(name)
            .bind(value)
            .execute(&mut *conn)
            .await?;
            if *added {
                sqlx::query(
                    "DELETE FROM metadata WHERE public_id = $1 AND name = $2 AND value = $3",
                )
                .bind(&m.to)
                .bind(name)
                .bind(value)
                .execute(&mut *conn)
                .await?;
            }
        }
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
             ORDER BY CASE WHEN study_date IS NULL OR study_date = '' THEN 1 ELSE 0 END,
                      study_date DESC, study_instance_uid LIMIT 1",
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
