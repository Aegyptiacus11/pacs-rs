//! What the index knows of each stored file's integrity: its SHA-256, taken when it was
//! stored, and its last check against the store. The checking itself is the archive's.

use sqlx::Row;
use sqlx::any::AnyRow;

use crate::{Error, Index, now_ms};

/// A stored file, as a check needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    pub sop_instance_uid: String,
    pub object_key: String,
    pub size_bytes: i64,
    /// `None` for a file stored before hashes were kept, until its first check.
    pub sha256: Option<String>,
}

/// A file whose last check failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedFile {
    pub study_instance_uid: String,
    pub series_instance_uid: String,
    pub sop_instance_uid: String,
    pub object_key: String,
    pub failure: String,
    pub checked_ms: i64,
}

/// How many files there are, and where their checks stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CheckCounts {
    pub files: i64,
    /// Stored before hashes were kept and not checked since.
    pub never_checked: i64,
    pub failed: i64,
}

fn stored_file(row: &AnyRow) -> Result<StoredFile, Error> {
    Ok(StoredFile {
        sop_instance_uid: row.try_get("sop_instance_uid")?,
        object_key: row.try_get("object_key")?,
        size_bytes: row.try_get("size_bytes")?,
        sha256: row.try_get("sha256")?,
    })
}

impl Index {
    /// Up to `limit` files not checked since `before_ms`, the longest unchecked first (those
    /// never checked before all others).
    pub async fn files_due(&self, before_ms: i64, limit: i64) -> Result<Vec<StoredFile>, Error> {
        let rows = sqlx::query(
            "SELECT sop_instance_uid, object_key, size_bytes, sha256 FROM instances
             WHERE checked_ms IS NULL OR checked_ms < $1
             ORDER BY COALESCE(checked_ms, 0), sop_instance_uid LIMIT $2",
        )
        .bind(before_ms)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(stored_file).collect()
    }

    /// Up to `limit` files after `after` (a SOP Instance UID), in that order: a whole sweep,
    /// page after page.
    pub async fn files_after(&self, after: &str, limit: i64) -> Result<Vec<StoredFile>, Error> {
        let rows = sqlx::query(
            "SELECT sop_instance_uid, object_key, size_bytes, sha256 FROM instances
             WHERE sop_instance_uid > $1 ORDER BY sop_instance_uid LIMIT $2",
        )
        .bind(after)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(stored_file).collect()
    }

    /// The files of one study.
    pub async fn files_of_study(&self, study_instance_uid: &str) -> Result<Vec<StoredFile>, Error> {
        let rows = sqlx::query(
            "SELECT i.sop_instance_uid, i.object_key, i.size_bytes, i.sha256
             FROM instances i JOIN series se ON se.series_instance_uid = i.series_instance_uid
             WHERE se.study_instance_uid = $1 ORDER BY i.sop_instance_uid",
        )
        .bind(study_instance_uid)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(stored_file).collect()
    }

    /// What a check of `sop_instance_uid` found: `failure` is `None` when the file matched.
    /// `sha256` is the hash taken by a first check of a file stored without one; a file that
    /// has one keeps it. Nothing happens to an instance deleted meanwhile.
    pub async fn record_check(
        &self,
        sop_instance_uid: &str,
        sha256: Option<&str>,
        failure: Option<&str>,
    ) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "UPDATE instances SET checked_ms = $1, check_failure = $2,
                    sha256 = COALESCE(sha256, $3)
             WHERE sop_instance_uid = $4",
        )
        .bind(now_ms())
        .bind(failure)
        .bind(sha256)
        .bind(sop_instance_uid)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Whether the database answers: one round trip.
    pub async fn ping(&self) -> Result<(), Error> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    /// The public ID and storage key of an instance, by SOP Instance UID.
    pub async fn instance_file(
        &self,
        sop_instance_uid: &str,
    ) -> Result<Option<(String, String)>, Error> {
        let row =
            sqlx::query("SELECT public_id, object_key FROM instances WHERE sop_instance_uid = $1")
                .bind(sop_instance_uid)
                .fetch_optional(&self.pool)
                .await?;
        row.map(|row| Ok((row.try_get("public_id")?, row.try_get("object_key")?)))
            .transpose()
    }

    /// The files whose last check failed, by study.
    pub async fn failed_files(&self) -> Result<Vec<FailedFile>, Error> {
        let rows = sqlx::query(
            "SELECT se.study_instance_uid, i.series_instance_uid, i.sop_instance_uid,
                    i.object_key, i.check_failure, i.checked_ms
             FROM instances i JOIN series se ON se.series_instance_uid = i.series_instance_uid
             WHERE i.check_failure IS NOT NULL
             ORDER BY se.study_instance_uid, i.series_instance_uid, i.sop_instance_uid",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(FailedFile {
                    study_instance_uid: row.try_get("study_instance_uid")?,
                    series_instance_uid: row.try_get("series_instance_uid")?,
                    sop_instance_uid: row.try_get("sop_instance_uid")?,
                    object_key: row.try_get("object_key")?,
                    failure: row.try_get("check_failure")?,
                    checked_ms: row.try_get("checked_ms")?,
                })
            })
            .collect()
    }

    pub async fn check_counts(&self) -> Result<CheckCounts, Error> {
        let row = sqlx::query(
            "SELECT count(*) AS files,
                    count(*) - count(checked_ms) AS never_checked,
                    count(check_failure) AS failed
             FROM instances",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(CheckCounts {
            files: row.try_get("files")?,
            never_checked: row.try_get("never_checked")?,
            failed: row.try_get("failed")?,
        })
    }
}
