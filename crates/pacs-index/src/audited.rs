//! What an audit trail says about the data a request touched: each study, whose it is,
//! and how many instances of each SOP class it holds.

use sqlx::Row;

use crate::{Error, Index, Level};

/// A study as an audit message describes it (PS3.15 A.5.3: the study and the patient
/// participant objects).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditedStudy {
    pub study_instance_uid: String,
    pub patient_id: Option<String>,
    pub patient_name: Option<String>,
    /// SOP class UID and number of instances, by UID.
    pub sop_classes: Vec<(String, i64)>,
}

impl Index {
    /// The study with this UID, or `None`.
    pub async fn audited_study(
        &self,
        study_instance_uid: &str,
    ) -> Result<Option<AuditedStudy>, Error> {
        if !self.sees_study(study_instance_uid).await? {
            return Ok(None);
        }
        Ok(self
            .audited(vec![study_instance_uid.to_owned()])
            .await?
            .pop())
    }

    /// The studies a REST resource lies in: the study of a study, series or instance, and
    /// every study of a patient. Empty when there is no such resource.
    pub async fn audited_by_public_id(
        &self,
        level: Level,
        public_id: &str,
    ) -> Result<Vec<AuditedStudy>, Error> {
        let sql = match level {
            Level::Patient => concat!(
                "SELECT st.study_instance_uid FROM studies st
                  WHERE st.patient_public_id = $1 AND",
                seen!("st", "2"),
                "ORDER BY st.study_instance_uid"
            ),
            Level::Study => concat!(
                "SELECT st.study_instance_uid FROM studies st WHERE st.public_id = $1 AND",
                seen!("st", "2")
            ),
            Level::Series => concat!(
                "SELECT se.study_instance_uid FROM series se
                   JOIN studies st USING (study_instance_uid) WHERE se.public_id = $1 AND",
                seen!("st", "2")
            ),
            Level::Instance => concat!(
                "SELECT se.study_instance_uid FROM instances i
                   JOIN series se USING (series_instance_uid)
                   JOIN studies st ON st.study_instance_uid = se.study_instance_uid
                  WHERE i.public_id = $1 AND",
                seen!("st", "2")
            ),
        };
        let uids: Vec<String> = sqlx::query_scalar(sql)
            .bind(public_id)
            .bind(self.account())
            .fetch_all(&self.pool)
            .await?;
        self.audited(uids).await
    }

    async fn audited(&self, study_instance_uids: Vec<String>) -> Result<Vec<AuditedStudy>, Error> {
        let mut studies = Vec::with_capacity(study_instance_uids.len());
        for uid in study_instance_uids {
            let Some(row) = sqlx::query(
                "SELECT patient_id, patient_name FROM studies WHERE study_instance_uid = $1",
            )
            .bind(&uid)
            .fetch_optional(&self.pool)
            .await?
            else {
                continue;
            };
            let classes = sqlx::query(
                "SELECT i.sop_class_uid, count(*) AS instances FROM instances i
                   JOIN series se USING (series_instance_uid)
                  WHERE se.study_instance_uid = $1
                  GROUP BY i.sop_class_uid ORDER BY i.sop_class_uid",
            )
            .bind(&uid)
            .fetch_all(&self.pool)
            .await?;
            studies.push(AuditedStudy {
                study_instance_uid: uid,
                patient_id: row.try_get("patient_id")?,
                patient_name: row.try_get("patient_name")?,
                sop_classes: classes
                    .iter()
                    .map(|class| Ok((class.try_get("sop_class_uid")?, class.try_get("instances")?)))
                    .collect::<Result<_, Error>>()?,
            });
        }
        Ok(studies)
    }
}
