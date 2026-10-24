//! Lookups by public ID, for the REST API.

use serde_json::Value;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::{Error, Index, parse};

/// A study or series is stable once nothing new has arrived for this long, the reference
/// server's default `StableAge`.
pub const STABLE_AGE_SECONDS: i64 = 60;

/// `last_update` and `is_stable` from the latest `received_at` of `$instances`. The 60 is
/// [`STABLE_AGE_SECONDS`]; SQL text has to be a literal, so it is written out.
macro_rules! freshness {
    ($instances:literal) => {
        concat!(
            "(SELECT strftime('%Y%m%dT%H%M%S', max(i.received_at)) ",
            $instances,
            ") AS last_update,
             (SELECT (julianday('now') - julianday(max(i.received_at))) * 86400 > 60 ",
            $instances,
            ") AS is_stable"
        )
    };
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestStudy {
    pub public_id: String,
    pub study_instance_uid: String,
    pub patient_public_id: String,
    pub main_tags: Value,
    pub patient_main_tags: Value,
    /// Public IDs of its series, by series number.
    pub series: Vec<String>,
    /// When its latest instance arrived, UTC, `YYYYMMDDTHHMMSS`.
    pub last_update: String,
    /// Whether nothing has arrived for [`STABLE_AGE_SECONDS`].
    pub is_stable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestSeries {
    pub public_id: String,
    pub series_instance_uid: String,
    pub study_instance_uid: String,
    pub parent_study: String,
    pub main_tags: Value,
    /// Public IDs of its instances, by instance number.
    pub instances: Vec<String>,
    pub last_update: String,
    pub is_stable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestInstance {
    pub public_id: String,
    pub sop_instance_uid: String,
    pub series_instance_uid: String,
    pub study_instance_uid: String,
    pub parent_series: String,
    pub parent_study: String,
    pub parent_patient: String,
    pub main_tags: Value,
    pub instance_number: Option<i64>,
    pub transfer_syntax_uid: String,
    pub object_key: String,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Statistics {
    pub series: i64,
    pub instances: i64,
    pub size_bytes: i64,
}

/// An instance query: the columns `instance` reads, then `$tail` (a literal), all fixed at
/// compile time as sqlx wants.
macro_rules! instance_query {
    ($tail:literal) => {
        concat!(
            "SELECT i.public_id, i.sop_instance_uid, i.series_instance_uid,
                    se.study_instance_uid, se.public_id AS parent_series,
                    st.public_id AS parent_study, st.patient_public_id AS parent_patient,
                    i.main_tags, i.instance_number, i.transfer_syntax_uid, i.object_key,
                    i.size_bytes
               FROM instances i
               JOIN series se USING (series_instance_uid)
               JOIN studies st ON st.study_instance_uid = se.study_instance_uid ",
            $tail
        )
    };
}

fn instance(row: SqliteRow) -> Result<RestInstance, Error> {
    Ok(RestInstance {
        public_id: row.try_get("public_id")?,
        sop_instance_uid: row.try_get("sop_instance_uid")?,
        series_instance_uid: row.try_get("series_instance_uid")?,
        study_instance_uid: row.try_get("study_instance_uid")?,
        parent_series: row.try_get("parent_series")?,
        parent_study: row.try_get("parent_study")?,
        parent_patient: row.try_get("parent_patient")?,
        main_tags: parse(row.try_get("main_tags")?)?,
        instance_number: row.try_get("instance_number")?,
        transfer_syntax_uid: row.try_get("transfer_syntax_uid")?,
        object_key: row.try_get("object_key")?,
        size_bytes: row.try_get("size_bytes")?,
    })
}

impl Index {
    /// Public IDs of every study.
    pub async fn study_public_ids(&self) -> Result<Vec<String>, Error> {
        Ok(sqlx::query_scalar(
            "SELECT public_id FROM studies WHERE public_id IS NOT NULL ORDER BY public_id",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn rest_study(&self, public_id: &str) -> Result<Option<RestStudy>, Error> {
        let Some(row) = sqlx::query(concat!(
            "SELECT st.public_id, st.study_instance_uid, st.patient_public_id, st.main_tags,
                        st.patient_main_tags, ",
            freshness!(
                "FROM instances i JOIN series se USING (series_instance_uid)
                      WHERE se.study_instance_uid = st.study_instance_uid"
            ),
            " FROM studies st WHERE st.public_id = ?"
        ))
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let study_instance_uid: String = row.try_get("study_instance_uid")?;
        let series = sqlx::query_scalar(
            "SELECT public_id FROM series WHERE study_instance_uid = ?
              ORDER BY series_number IS NULL, series_number, series_instance_uid",
        )
        .bind(&study_instance_uid)
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(RestStudy {
            public_id: row.try_get("public_id")?,
            study_instance_uid,
            patient_public_id: row.try_get("patient_public_id")?,
            main_tags: parse(row.try_get("main_tags")?)?,
            patient_main_tags: parse(row.try_get("patient_main_tags")?)?,
            series,
            last_update: row.try_get("last_update")?,
            is_stable: row.try_get("is_stable")?,
        }))
    }

    pub async fn rest_series(&self, public_id: &str) -> Result<Option<RestSeries>, Error> {
        let Some(row) = sqlx::query(concat!(
            "SELECT se.public_id, se.series_instance_uid, se.study_instance_uid,
                        se.main_tags, st.public_id AS parent_study, ",
            freshness!("FROM instances i WHERE i.series_instance_uid = se.series_instance_uid"),
            " FROM series se JOIN studies st USING (study_instance_uid)
                  WHERE se.public_id = ?"
        ))
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let series_instance_uid: String = row.try_get("series_instance_uid")?;
        let instances = sqlx::query_scalar(
            "SELECT public_id FROM instances WHERE series_instance_uid = ?
              ORDER BY instance_number IS NULL, instance_number, sop_instance_uid",
        )
        .bind(&series_instance_uid)
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(RestSeries {
            public_id: row.try_get("public_id")?,
            series_instance_uid,
            study_instance_uid: row.try_get("study_instance_uid")?,
            parent_study: row.try_get("parent_study")?,
            main_tags: parse(row.try_get("main_tags")?)?,
            instances,
            last_update: row.try_get("last_update")?,
            is_stable: row.try_get("is_stable")?,
        }))
    }

    pub async fn rest_instance(&self, public_id: &str) -> Result<Option<RestInstance>, Error> {
        sqlx::query(instance_query!("WHERE i.public_id = ?"))
            .bind(public_id)
            .fetch_optional(&self.pool)
            .await?
            .map(instance)
            .transpose()
    }

    /// The public ID of the series with this UID.
    pub async fn series_public_id(
        &self,
        series_instance_uid: &str,
    ) -> Result<Option<String>, Error> {
        Ok(
            sqlx::query_scalar("SELECT public_id FROM series WHERE series_instance_uid = ?")
                .bind(series_instance_uid)
                .fetch_optional(&self.pool)
                .await?
                .flatten(),
        )
    }

    pub async fn rest_instance_by_uid(
        &self,
        sop_instance_uid: &str,
    ) -> Result<Option<RestInstance>, Error> {
        sqlx::query(instance_query!("WHERE i.sop_instance_uid = ?"))
            .bind(sop_instance_uid)
            .fetch_optional(&self.pool)
            .await?
            .map(instance)
            .transpose()
    }

    /// A study's instances, by series number then instance number.
    pub async fn rest_study_instances(
        &self,
        study_public_id: &str,
    ) -> Result<Vec<RestInstance>, Error> {
        sqlx::query(instance_query!(
            "WHERE st.public_id = ?
              ORDER BY se.series_number IS NULL, se.series_number, se.series_instance_uid,
                       i.instance_number IS NULL, i.instance_number, i.sop_instance_uid"
        ))
        .bind(study_public_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(instance)
        .collect()
    }

    pub async fn study_statistics(
        &self,
        study_public_id: &str,
    ) -> Result<Option<Statistics>, Error> {
        let row = sqlx::query(
            "SELECT count(DISTINCT se.series_instance_uid) AS series,
                    count(i.sop_instance_uid) AS instances,
                    coalesce(sum(i.size_bytes), 0) AS size_bytes,
                    count(st.study_instance_uid) AS found
               FROM studies st
               LEFT JOIN series se ON se.study_instance_uid = st.study_instance_uid
               LEFT JOIN instances i ON i.series_instance_uid = se.series_instance_uid
              WHERE st.public_id = ?",
        )
        .bind(study_public_id)
        .fetch_one(&self.pool)
        .await?;
        let found: i64 = row.try_get("found")?;
        if found == 0 {
            return Ok(None);
        }
        Ok(Some(Statistics {
            series: row.try_get("series")?,
            instances: row.try_get("instances")?,
            size_bytes: row.try_get("size_bytes")?,
        }))
    }
}
