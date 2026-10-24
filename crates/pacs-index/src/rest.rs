//! Lookups by public ID, for the REST API.

use serde_json::Value;
use sqlx::Row;
use sqlx::any::AnyRow;

use crate::{Error, Index, now_ms, parse};

/// A study or series is stable once nothing new has arrived for this long, the reference
/// server's default `StableAge`.
pub const STABLE_AGE_SECONDS: i64 = 60;

/// The latest arrival among `$instances`, in epoch milliseconds, as `last_ms`.
macro_rules! last_arrival {
    ($instances:literal) => {
        concat!("(SELECT max(i.received_ms) ", $instances, ") AS last_ms")
    };
}

/// `YYYYMMDDTHHMMSS` in UTC, the reference server's LastUpdate format, from epoch ms.
pub(crate) fn last_update_text(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil date from days since 1970-01-01 (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// LastUpdate and IsStable from the latest arrival.
fn freshness(last_ms: Option<i64>) -> (String, bool) {
    match last_ms {
        Some(ms) => (
            last_update_text(ms),
            now_ms() - ms > STABLE_AGE_SECONDS * 1000,
        ),
        None => (String::new(), true),
    }
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

fn instance(row: AnyRow) -> Result<RestInstance, Error> {
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
            last_arrival!(
                "FROM instances i JOIN series se USING (series_instance_uid)
                      WHERE se.study_instance_uid = st.study_instance_uid"
            ),
            " FROM studies st WHERE st.public_id = $1"
        ))
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let study_instance_uid: String = row.try_get("study_instance_uid")?;
        let series = sqlx::query_scalar(
            "SELECT public_id FROM series WHERE study_instance_uid = $1
              ORDER BY series_number IS NULL, series_number, series_instance_uid",
        )
        .bind(&study_instance_uid)
        .fetch_all(&self.pool)
        .await?;
        let (last_update, is_stable) = freshness(row.try_get("last_ms")?);
        Ok(Some(RestStudy {
            public_id: row.try_get("public_id")?,
            study_instance_uid,
            patient_public_id: row.try_get("patient_public_id")?,
            main_tags: parse(row.try_get("main_tags")?)?,
            patient_main_tags: parse(row.try_get("patient_main_tags")?)?,
            series,
            last_update,
            is_stable,
        }))
    }

    pub async fn rest_series(&self, public_id: &str) -> Result<Option<RestSeries>, Error> {
        let Some(row) = sqlx::query(concat!(
            "SELECT se.public_id, se.series_instance_uid, se.study_instance_uid,
                        se.main_tags, st.public_id AS parent_study, ",
            last_arrival!("FROM instances i WHERE i.series_instance_uid = se.series_instance_uid"),
            " FROM series se JOIN studies st USING (study_instance_uid)
                  WHERE se.public_id = $1"
        ))
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let series_instance_uid: String = row.try_get("series_instance_uid")?;
        let instances = sqlx::query_scalar(
            "SELECT public_id FROM instances WHERE series_instance_uid = $1
              ORDER BY instance_number IS NULL, instance_number, sop_instance_uid",
        )
        .bind(&series_instance_uid)
        .fetch_all(&self.pool)
        .await?;
        let (last_update, is_stable) = freshness(row.try_get("last_ms")?);
        Ok(Some(RestSeries {
            public_id: row.try_get("public_id")?,
            series_instance_uid,
            study_instance_uid: row.try_get("study_instance_uid")?,
            parent_study: row.try_get("parent_study")?,
            main_tags: parse(row.try_get("main_tags")?)?,
            instances,
            last_update,
            is_stable,
        }))
    }

    pub async fn rest_instance(&self, public_id: &str) -> Result<Option<RestInstance>, Error> {
        sqlx::query(instance_query!("WHERE i.public_id = $1"))
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
            sqlx::query_scalar("SELECT public_id FROM series WHERE series_instance_uid = $1")
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
        sqlx::query(instance_query!("WHERE i.sop_instance_uid = $1"))
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
            "WHERE st.public_id = $1
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
                    CAST(coalesce(sum(i.size_bytes), 0) AS BIGINT) AS size_bytes,
                    count(st.study_instance_uid) AS found
               FROM studies st
               LEFT JOIN series se ON se.study_instance_uid = st.study_instance_uid
               LEFT JOIN instances i ON i.series_instance_uid = se.series_instance_uid
              WHERE st.public_id = $1",
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

#[cfg(test)]
mod tests {
    use super::last_update_text;

    #[test]
    fn last_update_is_utc_civil_time() {
        assert_eq!(last_update_text(0), "19700101T000000");
        // 2026-10-24T17:53:00.123Z
        assert_eq!(last_update_text(1_792_864_380_123), "20261024T175300");
        // a leap day
        assert_eq!(last_update_text(951_782_400_000), "20000229T000000");
    }
}
