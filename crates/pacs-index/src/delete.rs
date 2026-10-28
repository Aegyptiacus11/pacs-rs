//! Deleting patients, studies, series and instances by public ID.
//!
//! A deletion takes its resource and everything under it, then every ancestor it left
//! empty: deleting a series' last instance deletes the series, and its study if that was
//! the study's last series. A patient is the studies with its patient ID, so it goes with
//! its last study. The index says which files the deleted instances had; removing them is
//! the caller's part (`pacs_archive`).

use sqlx::{AnyConnection, Row};

use crate::{Error, Index};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Patient,
    Study,
    Series,
    Instance,
}

impl Level {
    /// The REST API's name for the level: `Patient`, `Study`, ...
    pub fn name(self) -> &'static str {
        match self {
            Level::Patient => "Patient",
            Level::Study => "Study",
            Level::Series => "Series",
            Level::Instance => "Instance",
        }
    }

    /// The level's path segment in the REST API: `patients`, `studies`, ...
    pub fn path(self) -> &'static str {
        match self {
            Level::Patient => "patients",
            Level::Study => "studies",
            Level::Series => "series",
            Level::Instance => "instances",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deleted {
    /// The storage keys of the deleted instances' files.
    pub object_keys: Vec<String>,
    /// The closest ancestor still indexed, as (level, public ID); `None` when the deletion
    /// took the whole patient.
    pub remaining_ancestor: Option<(Level, String)>,
}

/// Where a deletion starts its walk up: the series, study and patient it was under.
struct Parents {
    series_instance_uid: Option<String>,
    study_instance_uid: Option<String>,
    patient_public_id: Option<String>,
}

impl Index {
    /// Delete a resource, everything under it and every ancestor it leaves empty. `None`
    /// when no resource has that ID at that level.
    pub async fn delete(&self, level: Level, public_id: &str) -> Result<Option<Deleted>, Error> {
        let mut tx = self.begin_write().await?;
        let conn = &mut *tx;
        let (keys_sql, parents) = match level {
            Level::Instance => {
                let Some(row) = sqlx::query(
                    "SELECT i.sop_instance_uid, i.series_instance_uid, se.study_instance_uid,
                            st.patient_public_id
                       FROM instances i
                       JOIN series se USING (series_instance_uid)
                       JOIN studies st ON st.study_instance_uid = se.study_instance_uid
                      WHERE i.public_id = $1",
                )
                .bind(public_id)
                .fetch_optional(&mut *conn)
                .await?
                else {
                    return Ok(None);
                };
                let sop: String = row.try_get("sop_instance_uid")?;
                let keys = keys(
                    conn,
                    "SELECT object_key FROM instances WHERE sop_instance_uid = $1",
                    &sop,
                )
                .await?;
                sqlx::query("DELETE FROM instances WHERE sop_instance_uid = $1")
                    .bind(&sop)
                    .execute(&mut *conn)
                    .await?;
                (
                    keys,
                    Parents {
                        series_instance_uid: row.try_get("series_instance_uid")?,
                        study_instance_uid: row.try_get("study_instance_uid")?,
                        patient_public_id: row.try_get("patient_public_id")?,
                    },
                )
            }
            Level::Series => {
                let Some(row) = sqlx::query(
                    "SELECT se.series_instance_uid, se.study_instance_uid, st.patient_public_id
                       FROM series se JOIN studies st USING (study_instance_uid)
                      WHERE se.public_id = $1",
                )
                .bind(public_id)
                .fetch_optional(&mut *conn)
                .await?
                else {
                    return Ok(None);
                };
                let series: String = row.try_get("series_instance_uid")?;
                let keys = keys(
                    conn,
                    "SELECT object_key FROM instances WHERE series_instance_uid = $1",
                    &series,
                )
                .await?;
                delete_series(conn, &series).await?;
                (
                    keys,
                    Parents {
                        series_instance_uid: None,
                        study_instance_uid: row.try_get("study_instance_uid")?,
                        patient_public_id: row.try_get("patient_public_id")?,
                    },
                )
            }
            Level::Study => {
                let Some(row) = sqlx::query(
                    "SELECT study_instance_uid, patient_public_id FROM studies WHERE public_id = $1",
                )
                .bind(public_id)
                .fetch_optional(&mut *conn)
                .await?
                else {
                    return Ok(None);
                };
                let study: String = row.try_get("study_instance_uid")?;
                let keys = keys(
                    conn,
                    "SELECT i.object_key FROM instances i JOIN series se USING (series_instance_uid)
                      WHERE se.study_instance_uid = $1",
                    &study,
                )
                .await?;
                delete_study(conn, &study).await?;
                (
                    keys,
                    Parents {
                        series_instance_uid: None,
                        study_instance_uid: None,
                        patient_public_id: row.try_get("patient_public_id")?,
                    },
                )
            }
            Level::Patient => {
                let studies: Vec<String> = sqlx::query_scalar(
                    "SELECT study_instance_uid FROM studies WHERE patient_public_id = $1",
                )
                .bind(public_id)
                .fetch_all(&mut *conn)
                .await?;
                if studies.is_empty() {
                    return Ok(None);
                }
                let keys = keys(
                    conn,
                    "SELECT i.object_key FROM instances i
                       JOIN series se USING (series_instance_uid)
                       JOIN studies st ON st.study_instance_uid = se.study_instance_uid
                      WHERE st.patient_public_id = $1",
                    public_id,
                )
                .await?;
                for study in &studies {
                    delete_study(conn, study).await?;
                }
                (
                    keys,
                    Parents {
                        series_instance_uid: None,
                        study_instance_uid: None,
                        patient_public_id: None,
                    },
                )
            }
        };
        let remaining_ancestor = prune(conn, parents).await?;
        tx.commit().await?;
        Ok(Some(Deleted {
            object_keys: keys_sql,
            remaining_ancestor,
        }))
    }
}

async fn keys(conn: &mut AnyConnection, sql: &'static str, id: &str) -> Result<Vec<String>, Error> {
    Ok(sqlx::query_scalar(sql).bind(id).fetch_all(conn).await?)
}

async fn delete_series(conn: &mut AnyConnection, series: &str) -> Result<(), Error> {
    for sql in [
        "DELETE FROM instances WHERE series_instance_uid = $1",
        "DELETE FROM series WHERE series_instance_uid = $1",
    ] {
        sqlx::query(sql).bind(series).execute(&mut *conn).await?;
    }
    Ok(())
}

async fn delete_study(conn: &mut AnyConnection, study: &str) -> Result<(), Error> {
    for sql in [
        "DELETE FROM instances WHERE series_instance_uid IN
           (SELECT series_instance_uid FROM series WHERE study_instance_uid = $1)",
        "DELETE FROM series WHERE study_instance_uid = $1",
        "DELETE FROM studies WHERE study_instance_uid = $1",
    ] {
        sqlx::query(sql).bind(study).execute(&mut *conn).await?;
    }
    Ok(())
}

/// Delete the ancestors a deletion left empty, from the series up, and return the closest
/// one still there.
async fn prune(
    conn: &mut AnyConnection,
    parents: Parents,
) -> Result<Option<(Level, String)>, Error> {
    if let Some(series) = &parents.series_instance_uid {
        let left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM instances WHERE series_instance_uid = $1")
                .bind(series)
                .fetch_one(&mut *conn)
                .await?;
        if left > 0 {
            return public_id(
                conn,
                Level::Series,
                "SELECT public_id FROM series WHERE series_instance_uid = $1",
                series,
            )
            .await;
        }
        delete_series(conn, series).await?;
    }
    if let Some(study) = &parents.study_instance_uid {
        let left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM series WHERE study_instance_uid = $1")
                .bind(study)
                .fetch_one(&mut *conn)
                .await?;
        if left > 0 {
            return public_id(
                conn,
                Level::Study,
                "SELECT public_id FROM studies WHERE study_instance_uid = $1",
                study,
            )
            .await;
        }
        delete_study(conn, study).await?;
    }
    if let Some(patient) = &parents.patient_public_id {
        let left: i64 =
            sqlx::query_scalar("SELECT count(*) FROM studies WHERE patient_public_id = $1")
                .bind(patient)
                .fetch_one(&mut *conn)
                .await?;
        if left > 0 {
            return Ok(Some((Level::Patient, patient.clone())));
        }
    }
    Ok(None)
}

/// The public ID `sql` finds for `uid`, at `level`; `None` for a resource indexed before
/// public IDs existed.
async fn public_id(
    conn: &mut AnyConnection,
    level: Level,
    sql: &'static str,
    uid: &str,
) -> Result<Option<(Level, String)>, Error> {
    let id: Option<String> = sqlx::query_scalar(sql).bind(uid).fetch_one(conn).await?;
    Ok(id.map(|id| (level, id)))
}
