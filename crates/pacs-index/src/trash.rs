//! Deleted resources waiting to be emptied or restored (see the server's `/trash`): what the
//! index keeps of each deletion, and where its files wait in the store.

use serde_json::Value;
use sqlx::Row;
use sqlx::any::AnyRow;

use crate::{Error, Index, IntoTrash, now_ms};

/// A deletion in the trash.
#[derive(Debug, Clone, PartialEq)]
pub struct TrashEntry {
    pub id: i64,
    pub deleted_ms: i64,
    pub level: String,
    pub public_id: String,
    /// A study each: `StudyInstanceUID`, `PatientID`, `PatientName`, `project`, and
    /// `annotations` when the whole study went.
    pub studies: Value,
    /// Files waiting.
    pub files: i64,
}

/// A file in the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashFile {
    pub trash_key: String,
    pub object_key: String,
}

fn entry(row: &AnyRow) -> Result<TrashEntry, Error> {
    let studies: String = row.try_get("studies")?;
    Ok(TrashEntry {
        id: row.try_get("id")?,
        deleted_ms: row.try_get("deleted_ms")?,
        level: row.try_get("level")?,
        public_id: row.try_get("public_id")?,
        studies: serde_json::from_str(&studies)?,
        files: row.try_get("files")?,
    })
}

/// Within a deletion's transaction: its trash entry, made unless an earlier transaction of
/// the same deletion made it, and the files it took, each with where it is to wait.
pub(crate) async fn keep(
    conn: &mut sqlx::AnyConnection,
    trash: &IntoTrash<'_>,
    object_keys: &[String],
) -> Result<i64, Error> {
    let id = match trash.id {
        Some(id) => id,
        None => {
            sqlx::query_scalar(
                "INSERT INTO trash (deleted_ms, level, public_id, studies)
                 VALUES ($1, $2, $3, $4) RETURNING id",
            )
            .bind(now_ms())
            .bind(trash.level)
            .bind(trash.public_id)
            .bind(trash.studies.to_string())
            .fetch_one(&mut *conn)
            .await?
        }
    };
    for object_key in object_keys {
        sqlx::query(
            "INSERT INTO trash_files (trash_key, trash_id, object_key) VALUES ($1, $2, $3)",
        )
        .bind((trash.trash_key)(id, object_key))
        .bind(id)
        .bind(object_key)
        .execute(&mut *conn)
        .await?;
    }
    Ok(id)
}

const ENTRY: &str = "SELECT t.id, t.deleted_ms, t.level, t.public_id, t.studies,
                            (SELECT count(*) FROM trash_files f WHERE f.trash_id = t.id) AS files
                     FROM trash t";

impl Index {
    /// What entry `id` keeps of its studies, replaced.
    pub async fn set_trash_studies(&self, id: i64, studies: &Value) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        sqlx::query("UPDATE trash SET studies = $1 WHERE id = $2")
            .bind(studies.to_string())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Where every file of the trash was before its deletion. A file still there is one a
    /// deletion did not get to move: the trash's all the same, not an orphan.
    pub async fn trash_object_keys(&self) -> Result<Vec<String>, Error> {
        Ok(sqlx::query_scalar("SELECT object_key FROM trash_files")
            .fetch_all(&self.pool)
            .await?)
    }

    /// Every entry, the latest first.
    pub async fn trash(&self) -> Result<Vec<TrashEntry>, Error> {
        let sql = sqlx::AssertSqlSafe(format!("{ENTRY} ORDER BY t.id DESC"));
        let rows = sqlx::query(sql).fetch_all(&self.pool).await?;
        rows.iter().map(entry).collect()
    }

    pub async fn trash_entry(&self, id: i64) -> Result<Option<TrashEntry>, Error> {
        let sql = sqlx::AssertSqlSafe(format!("{ENTRY} WHERE t.id = $1"));
        let row = sqlx::query(sql).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(entry).transpose()
    }

    pub async fn trash_files(&self, id: i64) -> Result<Vec<TrashFile>, Error> {
        let rows = sqlx::query(
            "SELECT trash_key, object_key FROM trash_files WHERE trash_id = $1 ORDER BY trash_key",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(TrashFile {
                    trash_key: row.try_get("trash_key")?,
                    object_key: row.try_get("object_key")?,
                })
            })
            .collect()
    }

    /// The entries deleted before `before_ms`: those to empty.
    pub async fn trash_older_than(&self, before_ms: i64) -> Result<Vec<i64>, Error> {
        Ok(
            sqlx::query_scalar("SELECT id FROM trash WHERE deleted_ms < $1 ORDER BY id")
                .bind(before_ms)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// A file gone from the trash: restored, or emptied.
    pub async fn remove_trash_file(&self, trash_key: &str) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        sqlx::query("DELETE FROM trash_files WHERE trash_key = $1")
            .bind(trash_key)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// An entry gone from the trash, with what is left of its files.
    pub async fn remove_trash(&self, id: i64) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        // SQLite does not hold to the foreign key unless asked to: both by hand
        sqlx::query("DELETE FROM trash_files WHERE trash_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM trash WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The project of a study as stored, whoever asks: `None` for no such study.
    pub async fn project_of_study(
        &self,
        study_instance_uid: &str,
    ) -> Result<Option<Option<String>>, Error> {
        Ok(
            sqlx::query_scalar("SELECT project FROM studies WHERE study_instance_uid = $1")
                .bind(study_instance_uid)
                .fetch_optional(&self.pool)
                .await?,
        )
    }
}
