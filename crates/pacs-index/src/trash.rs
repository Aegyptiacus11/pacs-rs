//! Deleted resources waiting to be emptied or restored (see the server's `/trash`): what the
//! index keeps of each deletion, and where its files wait in the store.

use serde_json::Value;
use sqlx::Row;
use sqlx::any::AnyRow;

use crate::{Error, Index, now_ms};

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

const ENTRY: &str = "SELECT t.id, t.deleted_ms, t.level, t.public_id, t.studies,
                            (SELECT count(*) FROM trash_files f WHERE f.trash_id = t.id) AS files
                     FROM trash t";

impl Index {
    /// A deletion goes in the trash; its id. Its files are added as they are moved.
    pub async fn create_trash(
        &self,
        level: &str,
        public_id: &str,
        studies: &Value,
    ) -> Result<i64, Error> {
        let mut tx = self.begin_write().await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO trash (deleted_ms, level, public_id, studies) VALUES ($1, $2, $3, $4)
             RETURNING id",
        )
        .bind(now_ms())
        .bind(level)
        .bind(public_id)
        .bind(studies.to_string())
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// A file moved into the trash entry `trash_id`.
    pub async fn add_trash_file(&self, trash_id: i64, file: &TrashFile) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "INSERT INTO trash_files (trash_key, trash_id, object_key) VALUES ($1, $2, $3)",
        )
        .bind(&file.trash_key)
        .bind(trash_id)
        .bind(&file.object_key)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
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
