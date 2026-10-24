//! The pacs-rs index: which studies, series and instances exist, the attributes QIDO-RS
//! and WADO-RS metadata answer with, and the storage key of each instance's file.
//!
//! SQLite for now. The schema is in `migrations/` and applied on open.

use std::path::Path;
use std::str::FromStr;

use dicom_dictionary_std::tags;
use pacs_core::{Instance, levels};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The UIDs disagree with what is already indexed, e.g. a series UID already seen in
    /// another study. The index refuses rather than move things between studies.
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// What the index keeps of one received instance.
#[derive(Debug, Clone)]
pub struct NewInstance {
    pub study_instance_uid: String,
    pub series_instance_uid: String,
    pub sop_instance_uid: String,
    pub sop_class_uid: String,
    pub transfer_syntax_uid: String,
    pub instance_number: Option<i64>,
    pub object_key: String,
    pub size_bytes: i64,
    patient_id: Option<String>,
    patient_name: Option<String>,
    study_date: Option<String>,
    accession_number: Option<String>,
    modality: Option<String>,
    series_number: Option<i64>,
    study_attributes: Value,
    series_attributes: Value,
    instance_attributes: Value,
    metadata: Value,
}

impl NewInstance {
    /// `object_key` is where the store put the file, `size_bytes` the file's length.
    pub fn new(instance: &Instance, object_key: String, size_bytes: usize) -> Result<Self, Error> {
        let key = &instance.key;
        Ok(NewInstance {
            study_instance_uid: key.study_instance_uid.clone(),
            series_instance_uid: key.series_instance_uid.clone(),
            sop_instance_uid: key.sop_instance_uid.clone(),
            sop_class_uid: key.sop_class_uid.clone(),
            transfer_syntax_uid: key.transfer_syntax_uid.clone(),
            instance_number: instance.int(tags::INSTANCE_NUMBER),
            object_key,
            size_bytes: size_bytes as i64,
            patient_id: instance.string(tags::PATIENT_ID),
            patient_name: instance.string(tags::PATIENT_NAME),
            study_date: instance.string(tags::STUDY_DATE),
            accession_number: instance.string(tags::ACCESSION_NUMBER),
            modality: instance.string(tags::MODALITY),
            series_number: instance.int(tags::SERIES_NUMBER),
            study_attributes: instance.attributes_json(levels::STUDY)?,
            series_attributes: instance.attributes_json(levels::SERIES)?,
            instance_attributes: instance.attributes_json(levels::INSTANCE)?,
            metadata: instance.metadata_json()?,
        })
    }
}

/// Whether [`Index::record`] added an instance or replaced one with the same UIDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    Added,
    Replaced,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Study {
    pub study_instance_uid: String,
    /// DICOM JSON of the study-level attributes.
    pub attributes: Value,
    pub modalities: Vec<String>,
    pub series_count: i64,
    pub instance_count: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub study_instance_uid: String,
    pub series_instance_uid: String,
    pub attributes: Value,
    pub instance_count: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstanceEntry {
    pub study_instance_uid: String,
    pub series_instance_uid: String,
    pub sop_instance_uid: String,
    pub sop_class_uid: String,
    pub transfer_syntax_uid: String,
    pub object_key: String,
    pub size_bytes: i64,
    pub attributes: Value,
}

/// Exact-match study search. Fields left `None` match anything.
#[derive(Debug, Clone, Default)]
pub struct StudyFilter {
    pub study_instance_uid: Option<String>,
    pub patient_id: Option<String>,
    pub accession_number: Option<String>,
    pub study_date: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Index {
    pool: SqlitePool,
}

impl Index {
    /// Open (creating if needed) the index in a SQLite file.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        Self::with_pool(SqlitePoolOptions::new().connect_with(options).await?).await
    }

    /// A fresh index in memory, for tests. One connection, since each in-memory
    /// connection would otherwise be its own database.
    pub async fn in_memory() -> Result<Self, Error> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        Self::with_pool(pool).await
    }

    async fn with_pool(pool: SqlitePool) -> Result<Self, Error> {
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Index { pool })
    }

    /// Record a stored instance. Re-sending an instance replaces its entry; study and
    /// series attributes follow the latest instance received.
    pub async fn record(&self, new: &NewInstance) -> Result<Recorded, Error> {
        let mut tx = self.pool.begin().await?;

        let series_study: Option<String> = sqlx::query_scalar(
            "SELECT study_instance_uid FROM series WHERE series_instance_uid = ?",
        )
        .bind(&new.series_instance_uid)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(study) = series_study.filter(|s| *s != new.study_instance_uid) {
            return Err(Error::Conflict(format!(
                "series {} is already in study {study}, not {}",
                new.series_instance_uid, new.study_instance_uid
            )));
        }
        let instance_series: Option<String> = sqlx::query_scalar(
            "SELECT series_instance_uid FROM instances WHERE sop_instance_uid = ?",
        )
        .bind(&new.sop_instance_uid)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(series) = instance_series
            .as_ref()
            .filter(|s| **s != new.series_instance_uid)
        {
            return Err(Error::Conflict(format!(
                "instance {} is already in series {series}, not {}",
                new.sop_instance_uid, new.series_instance_uid
            )));
        }

        sqlx::query(
            "INSERT INTO studies (study_instance_uid, patient_id, patient_name, study_date,
                                  accession_number, attributes)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (study_instance_uid) DO UPDATE SET
                patient_id = excluded.patient_id, patient_name = excluded.patient_name,
                study_date = excluded.study_date, accession_number = excluded.accession_number,
                attributes = excluded.attributes",
        )
        .bind(&new.study_instance_uid)
        .bind(&new.patient_id)
        .bind(&new.patient_name)
        .bind(&new.study_date)
        .bind(&new.accession_number)
        .bind(new.study_attributes.to_string())
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO series (series_instance_uid, study_instance_uid, modality, series_number,
                                 attributes)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (series_instance_uid) DO UPDATE SET
                modality = excluded.modality, series_number = excluded.series_number,
                attributes = excluded.attributes",
        )
        .bind(&new.series_instance_uid)
        .bind(&new.study_instance_uid)
        .bind(&new.modality)
        .bind(new.series_number)
        .bind(new.series_attributes.to_string())
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO instances (sop_instance_uid, series_instance_uid, sop_class_uid,
                                    instance_number, transfer_syntax_uid, object_key, size_bytes,
                                    attributes, metadata)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (sop_instance_uid) DO UPDATE SET
                sop_class_uid = excluded.sop_class_uid,
                instance_number = excluded.instance_number,
                transfer_syntax_uid = excluded.transfer_syntax_uid,
                object_key = excluded.object_key, size_bytes = excluded.size_bytes,
                attributes = excluded.attributes, metadata = excluded.metadata,
                received_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(&new.sop_instance_uid)
        .bind(&new.series_instance_uid)
        .bind(&new.sop_class_uid)
        .bind(new.instance_number)
        .bind(&new.transfer_syntax_uid)
        .bind(&new.object_key)
        .bind(new.size_bytes)
        .bind(new.instance_attributes.to_string())
        .bind(new.metadata.to_string())
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(if instance_series.is_some() {
            Recorded::Replaced
        } else {
            Recorded::Added
        })
    }

    pub async fn studies(&self, filter: &StudyFilter) -> Result<Vec<Study>, Error> {
        let rows = sqlx::query(
            "SELECT st.study_instance_uid, st.attributes,
                    (SELECT count(*) FROM series se
                      WHERE se.study_instance_uid = st.study_instance_uid) AS series_count,
                    (SELECT count(*) FROM instances i JOIN series se USING (series_instance_uid)
                      WHERE se.study_instance_uid = st.study_instance_uid) AS instance_count,
                    (SELECT group_concat(modality, char(92)) FROM
                       (SELECT DISTINCT modality FROM series se
                         WHERE se.study_instance_uid = st.study_instance_uid
                           AND modality IS NOT NULL ORDER BY modality)) AS modalities
               FROM studies st
              WHERE (?1 IS NULL OR st.study_instance_uid = ?1)
                AND (?2 IS NULL OR st.patient_id = ?2)
                AND (?3 IS NULL OR st.accession_number = ?3)
                AND (?4 IS NULL OR st.study_date = ?4)
              ORDER BY st.study_date DESC, st.study_instance_uid
              LIMIT ?5 OFFSET ?6",
        )
        .bind(&filter.study_instance_uid)
        .bind(&filter.patient_id)
        .bind(&filter.accession_number)
        .bind(&filter.study_date)
        .bind(filter.limit.unwrap_or(-1))
        .bind(filter.offset.unwrap_or(0))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let modalities: Option<String> = row.try_get("modalities")?;
                Ok(Study {
                    study_instance_uid: row.try_get("study_instance_uid")?,
                    attributes: parse(row.try_get("attributes")?)?,
                    modalities: modalities
                        .map(|m| m.split('\\').map(str::to_owned).collect())
                        .unwrap_or_default(),
                    series_count: row.try_get("series_count")?,
                    instance_count: row.try_get("instance_count")?,
                })
            })
            .collect()
    }

    pub async fn study(&self, study_instance_uid: &str) -> Result<Option<Study>, Error> {
        let filter = StudyFilter {
            study_instance_uid: Some(study_instance_uid.to_owned()),
            ..StudyFilter::default()
        };
        Ok(self.studies(&filter).await?.into_iter().next())
    }

    /// A study's series, by series number. `series_instance_uid` narrows to one.
    pub async fn series(
        &self,
        study_instance_uid: &str,
        series_instance_uid: Option<&str>,
    ) -> Result<Vec<Series>, Error> {
        let rows = sqlx::query(
            "SELECT se.study_instance_uid, se.series_instance_uid, se.attributes,
                    (SELECT count(*) FROM instances i
                      WHERE i.series_instance_uid = se.series_instance_uid) AS instance_count
               FROM series se
              WHERE se.study_instance_uid = ?1
                AND (?2 IS NULL OR se.series_instance_uid = ?2)
              ORDER BY se.series_number IS NULL, se.series_number, se.series_instance_uid",
        )
        .bind(study_instance_uid)
        .bind(series_instance_uid)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(Series {
                    study_instance_uid: row.try_get("study_instance_uid")?,
                    series_instance_uid: row.try_get("series_instance_uid")?,
                    attributes: parse(row.try_get("attributes")?)?,
                    instance_count: row.try_get("instance_count")?,
                })
            })
            .collect()
    }

    /// A series' instances, by instance number. `sop_instance_uid` narrows to one.
    pub async fn instances(
        &self,
        study_instance_uid: &str,
        series_instance_uid: &str,
        sop_instance_uid: Option<&str>,
    ) -> Result<Vec<InstanceEntry>, Error> {
        let rows = sqlx::query(
            "SELECT se.study_instance_uid, i.series_instance_uid, i.sop_instance_uid,
                    i.sop_class_uid, i.transfer_syntax_uid, i.object_key, i.size_bytes,
                    i.attributes
               FROM instances i JOIN series se USING (series_instance_uid)
              WHERE se.study_instance_uid = ?1 AND i.series_instance_uid = ?2
                AND (?3 IS NULL OR i.sop_instance_uid = ?3)
              ORDER BY i.instance_number IS NULL, i.instance_number, i.sop_instance_uid",
        )
        .bind(study_instance_uid)
        .bind(series_instance_uid)
        .bind(sop_instance_uid)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(InstanceEntry {
                    study_instance_uid: row.try_get("study_instance_uid")?,
                    series_instance_uid: row.try_get("series_instance_uid")?,
                    sop_instance_uid: row.try_get("sop_instance_uid")?,
                    sop_class_uid: row.try_get("sop_class_uid")?,
                    transfer_syntax_uid: row.try_get("transfer_syntax_uid")?,
                    object_key: row.try_get("object_key")?,
                    size_bytes: row.try_get("size_bytes")?,
                    attributes: parse(row.try_get("attributes")?)?,
                })
            })
            .collect()
    }

    pub async fn instance(
        &self,
        study_instance_uid: &str,
        series_instance_uid: &str,
        sop_instance_uid: &str,
    ) -> Result<Option<InstanceEntry>, Error> {
        Ok(self
            .instances(
                study_instance_uid,
                series_instance_uid,
                Some(sop_instance_uid),
            )
            .await?
            .into_iter()
            .next())
    }

    /// The metadata (data set without pixel data) of every instance in a series, by
    /// instance number: the body of WADO-RS series metadata.
    pub async fn series_metadata(
        &self,
        study_instance_uid: &str,
        series_instance_uid: &str,
    ) -> Result<Vec<Value>, Error> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT i.metadata
               FROM instances i JOIN series se USING (series_instance_uid)
              WHERE se.study_instance_uid = ?1 AND i.series_instance_uid = ?2
              ORDER BY i.instance_number IS NULL, i.instance_number, i.sop_instance_uid",
        )
        .bind(study_instance_uid)
        .bind(series_instance_uid)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(parse).collect()
    }
}

fn parse(text: String) -> Result<Value, Error> {
    Ok(serde_json::from_str(&text)?)
}
