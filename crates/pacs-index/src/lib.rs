//! The pacs-rs index: which studies, series and instances exist, the attributes QIDO-RS
//! and WADO-RS metadata answer with, and the storage key of each instance's file.
//!
//! SQLite (a file, or memory for tests) or Postgres, through sqlx's `Any` driver: one code
//! path, and SQL written in the subset both understand - `$1` parameters, `CAST($1 AS
//! TEXT) IS NULL` for optional ones - with a dialect-specific statement only where there is
//! no common one. Each dialect's schema is in `migrations/<dialect>/`, applied on open.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use dicom_dictionary_std::tags;
use pacs_core::{Instance, levels, main_tags, public_ids};
use serde_json::Value;
use sqlx::AnyPool;
use sqlx::Row;
use sqlx::any::AnyPoolOptions;

mod annotations;
mod delete;
mod protocols;
mod rest;
mod users;
mod worklist;
pub use delete::{Deleted, Level};
pub use protocols::StoredProtocol;
pub use rest::{RestInstance, RestPatient, RestSeries, RestStudy, STABLE_AGE_SECONDS, Statistics};
pub use users::{StoredUser, UserChange};
pub use worklist::{
    PatientQuery, PatientSort, WorklistFacets, WorklistPatient, WorklistQuery, WorklistSort,
    WorklistStudy, WorklistSummary,
};

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
    study_public_id: String,
    patient_public_id: String,
    series_public_id: String,
    instance_public_id: String,
    patient_main_tags: Value,
    study_main_tags: Value,
    series_main_tags: Value,
    instance_main_tags: Value,
}

impl NewInstance {
    /// `object_key` is where the store put the file, `size_bytes` the file's length.
    pub fn new(instance: &Instance, object_key: String, size_bytes: usize) -> Result<Self, Error> {
        let key = &instance.key;
        let patient = instance.string(tags::PATIENT_ID).unwrap_or_default();
        let (study, series, sop) = (
            key.study_instance_uid.as_str(),
            key.series_instance_uid.as_str(),
            key.sop_instance_uid.as_str(),
        );
        let main = |tags| Value::Object(instance.main_tags(tags));
        Ok(NewInstance {
            patient_public_id: public_ids::patient(&patient),
            study_public_id: public_ids::study(&patient, study),
            series_public_id: public_ids::series(&patient, study, series),
            instance_public_id: public_ids::instance(&patient, study, series, sop),
            patient_main_tags: main(main_tags::PATIENT),
            study_main_tags: main(main_tags::STUDY),
            series_main_tags: main(main_tags::SERIES),
            instance_main_tags: main(main_tags::INSTANCE),
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
    /// The REST API's ID; `None` for a study indexed before public IDs existed.
    pub public_id: Option<String>,
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
    /// When the stored copy was received, in milliseconds since the epoch: it changes
    /// whenever the instance's file does.
    pub received_ms: i64,
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

/// Which database is behind the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone)]
pub struct Index {
    pool: AnyPool,
    dialect: Dialect,
}

/// The study search, with `$modalities` the dialect's way to list a study's modalities
/// separated by `\\`.
macro_rules! studies_query {
    ($modalities:literal) => {
        concat!(
            "SELECT st.study_instance_uid, st.public_id, st.attributes,
                    (SELECT count(*) FROM series se
                      WHERE se.study_instance_uid = st.study_instance_uid) AS series_count,
                    (SELECT count(*) FROM instances i JOIN series se USING (series_instance_uid)
                      WHERE se.study_instance_uid = st.study_instance_uid) AS instance_count, ",
            $modalities,
            " AS modalities
               FROM studies st
              WHERE (CAST($1 AS TEXT) IS NULL OR st.study_instance_uid = $1)
                AND (CAST($2 AS TEXT) IS NULL OR st.patient_id = $2)
                AND (CAST($3 AS TEXT) IS NULL OR st.accession_number = $3)
                AND (CAST($4 AS TEXT) IS NULL OR st.study_date = $4)
              ORDER BY st.study_date DESC NULLS LAST, st.study_instance_uid
              LIMIT $5 OFFSET $6"
        )
    };
}

/// `sqlite:<path, percent-encoded>?mode=rwc`. The Any driver parses the URL before sqlite
/// sees it, and a path pasted in as is does not survive that: `sqlite://C:\Users\...` reads
/// as host `C` with a port, and `?` or `#` in a name would start the query or fragment.
/// Encoded, the path is one opaque segment the parser keeps verbatim; sqlx decodes it back.
fn sqlite_url(path: &Path) -> Result<String, Error> {
    let Some(text) = path.to_str() else {
        return Err(Error::Database(sqlx::Error::Configuration(
            format!("index path is not UTF-8: {}", path.display()).into(),
        )));
    };
    let mut url = String::from("sqlite:");
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            url.push(byte as char);
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url.push_str("?mode=rwc");
    Ok(url)
}

/// Milliseconds since the Unix epoch, now.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl Index {
    /// Open (creating if needed) the index in a SQLite file.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::connect(&sqlite_url(path.as_ref())?).await
    }

    /// Open the index at `url`: `sqlite://<path>?mode=rwc`, `sqlite::memory:` or
    /// `postgres://user:password@host/database`.
    pub async fn connect(url: &str) -> Result<Self, Error> {
        sqlx::any::install_default_drivers();
        let dialect = if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            Dialect::Postgres
        } else if url.starts_with("sqlite:") {
            Dialect::Sqlite
        } else {
            return Err(Error::Database(sqlx::Error::Configuration(
                format!("not a sqlite: or postgres:// URL: {url}").into(),
            )));
        };
        let mut options = AnyPoolOptions::new();
        if url.starts_with("sqlite::memory:") {
            // each in-memory connection would be a database of its own
            options = options.max_connections(1);
        }
        let pool = options.connect(url).await?;
        if dialect == Dialect::Sqlite && !url.starts_with("sqlite::memory:") {
            // write-ahead logging: readers never wait for the writer; it sticks to the file
            sqlx::query("PRAGMA journal_mode = WAL")
                .execute(&pool)
                .await?;
        }
        match dialect {
            Dialect::Sqlite => sqlx::migrate!("./migrations/sqlite").run(&pool).await?,
            Dialect::Postgres => sqlx::migrate!("./migrations/postgres").run(&pool).await?,
        }
        Ok(Index { pool, dialect })
    }

    /// A fresh index in memory, for tests.
    pub async fn in_memory() -> Result<Self, Error> {
        Self::connect("sqlite::memory:").await
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// A transaction that holds the index's one write lock from its start.
    pub(crate) async fn begin_write(&self) -> Result<sqlx::Transaction<'static, sqlx::Any>, Error> {
        // One writer at a time, taken before the first read. In SQLite, IMMEDIATE takes the
        // write lock: a deferred transaction would read, then fail at once with "database
        // is locked" upgrading while another upload writes. In Postgres, a transaction-wide
        // advisory lock does the same, so a writer's checks see every earlier write.
        let tx = match self.dialect {
            Dialect::Sqlite => self.pool.begin_with("BEGIN IMMEDIATE").await?,
            Dialect::Postgres => {
                let mut tx = self.pool.begin().await?;
                sqlx::query("SELECT CAST(pg_advisory_xact_lock(7461636) AS TEXT)")
                    .execute(&mut *tx)
                    .await?;
                tx
            }
        };
        Ok(tx)
    }

    /// Record a stored instance. Re-sending an instance replaces its entry; study and
    /// series attributes follow the latest instance received.
    pub async fn record(&self, new: &NewInstance) -> Result<Recorded, Error> {
        let mut tx = self.begin_write().await?;

        let series_study: Option<String> = sqlx::query_scalar(
            "SELECT study_instance_uid FROM series WHERE series_instance_uid = $1",
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
            "SELECT series_instance_uid FROM instances WHERE sop_instance_uid = $1",
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
                                  accession_number, attributes, public_id, patient_public_id,
                                  main_tags, patient_main_tags)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (study_instance_uid) DO UPDATE SET
                patient_id = excluded.patient_id, patient_name = excluded.patient_name,
                study_date = excluded.study_date, accession_number = excluded.accession_number,
                attributes = excluded.attributes, public_id = excluded.public_id,
                patient_public_id = excluded.patient_public_id,
                main_tags = excluded.main_tags, patient_main_tags = excluded.patient_main_tags",
        )
        .bind(&new.study_instance_uid)
        .bind(&new.patient_id)
        .bind(&new.patient_name)
        .bind(&new.study_date)
        .bind(&new.accession_number)
        .bind(new.study_attributes.to_string())
        .bind(&new.study_public_id)
        .bind(&new.patient_public_id)
        .bind(new.study_main_tags.to_string())
        .bind(new.patient_main_tags.to_string())
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO series (series_instance_uid, study_instance_uid, modality, series_number,
                                 attributes, public_id, main_tags)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (series_instance_uid) DO UPDATE SET
                modality = excluded.modality, series_number = excluded.series_number,
                attributes = excluded.attributes, public_id = excluded.public_id,
                main_tags = excluded.main_tags",
        )
        .bind(&new.series_instance_uid)
        .bind(&new.study_instance_uid)
        .bind(&new.modality)
        .bind(new.series_number)
        .bind(new.series_attributes.to_string())
        .bind(&new.series_public_id)
        .bind(new.series_main_tags.to_string())
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO instances (sop_instance_uid, series_instance_uid, sop_class_uid,
                                    instance_number, transfer_syntax_uid, object_key, size_bytes,
                                    attributes, metadata, public_id, main_tags, received_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (sop_instance_uid) DO UPDATE SET
                sop_class_uid = excluded.sop_class_uid,
                instance_number = excluded.instance_number,
                transfer_syntax_uid = excluded.transfer_syntax_uid,
                object_key = excluded.object_key, size_bytes = excluded.size_bytes,
                attributes = excluded.attributes, metadata = excluded.metadata,
                public_id = excluded.public_id, main_tags = excluded.main_tags,
                received_ms = excluded.received_ms",
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
        .bind(&new.instance_public_id)
        .bind(new.instance_main_tags.to_string())
        .bind(now_ms())
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
        let sql = match self.dialect {
            Dialect::Sqlite => studies_query!(
                "(SELECT group_concat(modality, char(92)) FROM
                   (SELECT DISTINCT modality FROM series se
                     WHERE se.study_instance_uid = st.study_instance_uid
                       AND modality IS NOT NULL ORDER BY modality))"
            ),
            Dialect::Postgres => studies_query!(
                "(SELECT string_agg(DISTINCT modality, chr(92) ORDER BY modality) FROM series se
                   WHERE se.study_instance_uid = st.study_instance_uid)"
            ),
        };
        let rows = sqlx::query(sql)
            .bind(&filter.study_instance_uid)
            .bind(&filter.patient_id)
            .bind(&filter.accession_number)
            .bind(&filter.study_date)
            .bind(filter.limit.unwrap_or(i64::MAX))
            .bind(filter.offset.unwrap_or(0))
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                let modalities: Option<String> = row.try_get("modalities")?;
                Ok(Study {
                    study_instance_uid: row.try_get("study_instance_uid")?,
                    public_id: row.try_get("public_id")?,
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
              WHERE se.study_instance_uid = $1
                AND (CAST($2 AS TEXT) IS NULL OR se.series_instance_uid = $2)
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
                    i.received_ms, i.attributes
               FROM instances i JOIN series se USING (series_instance_uid)
              WHERE se.study_instance_uid = $1 AND i.series_instance_uid = $2
                AND (CAST($3 AS TEXT) IS NULL OR i.sop_instance_uid = $3)
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
                    received_ms: row
                        .try_get::<Option<i64>, _>("received_ms")?
                        .unwrap_or_default(),
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
    /// instance number: the body of WADO-RS series metadata. `sop_instance_uid` narrows
    /// it to one instance.
    pub async fn metadata(
        &self,
        study_instance_uid: &str,
        series_instance_uid: &str,
        sop_instance_uid: Option<&str>,
    ) -> Result<Vec<Value>, Error> {
        self.metadata_text(study_instance_uid, series_instance_uid, sop_instance_uid)
            .await?
            .into_iter()
            .map(parse)
            .collect()
    }

    /// [`Index::metadata`] as the stored JSON text, one string per instance, for a server
    /// to send without parsing and writing it again.
    pub async fn metadata_text(
        &self,
        study_instance_uid: &str,
        series_instance_uid: &str,
        sop_instance_uid: Option<&str>,
    ) -> Result<Vec<String>, Error> {
        Ok(sqlx::query_scalar(
            "SELECT i.metadata
               FROM instances i JOIN series se USING (series_instance_uid)
              WHERE se.study_instance_uid = $1 AND i.series_instance_uid = $2
                AND (CAST($3 AS TEXT) IS NULL OR i.sop_instance_uid = $3)
              ORDER BY i.instance_number IS NULL, i.instance_number, i.sop_instance_uid",
        )
        .bind(study_instance_uid)
        .bind(series_instance_uid)
        .bind(sop_instance_uid)
        .fetch_all(&self.pool)
        .await?)
    }
}

fn parse(text: String) -> Result<Value, Error> {
    Ok(serde_json::from_str(&text)?)
}
