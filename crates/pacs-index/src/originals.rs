//! The way back from a project's de-identified copy to the original (write-back): which UID
//! each replacement replaced, and the original patient and study of each de-identified
//! study. Kept by the project's receiver as it de-identifies; read only to send a result to
//! the hospital.

use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::{Error, Index};

/// The original patient and study of a de-identified study, as main tags by keyword.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalStudy {
    pub patient: Map<String, Value>,
    pub study: Map<String, Value>,
}

fn object(text: &str) -> Result<Map<String, Value>, Error> {
    Ok(serde_json::from_str(text)?)
}

impl Index {
    /// Record what de-identifying one instance for `project` replaced: its UID pairs
    /// (replacement, original), and the original patient and study of its de-identified
    /// study. What is recorded already is kept: the same original always gets the same
    /// replacement.
    pub async fn record_originals(
        &self,
        project: &str,
        uids: &[(String, String)],
        replaced_study: &str,
        original: &OriginalStudy,
    ) -> Result<(), Error> {
        let mut tx = self.begin_write().await?;
        for (replaced, original) in uids {
            sqlx::query(
                "INSERT INTO project_uids (project, replaced, original) VALUES ($1, $2, $3)
                 ON CONFLICT (project, replaced) DO NOTHING",
            )
            .bind(project)
            .bind(replaced)
            .bind(original)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO project_studies (project, replaced_study, patient_main_tags,
                                          study_main_tags)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (project, replaced_study) DO NOTHING",
        )
        .bind(project)
        .bind(replaced_study)
        .bind(Value::Object(original.patient.clone()).to_string())
        .bind(Value::Object(original.study.clone()).to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The originals of those of `replaced` that `project` replaced; the others are not in
    /// the answer.
    pub async fn original_uids(
        &self,
        project: &str,
        replaced: &[String],
    ) -> Result<HashMap<String, String>, Error> {
        let mut found = HashMap::new();
        for uid in replaced {
            let original: Option<String> = sqlx::query_scalar(
                "SELECT original FROM project_uids WHERE project = $1 AND replaced = $2",
            )
            .bind(project)
            .bind(uid)
            .fetch_optional(&self.pool)
            .await?;
            if let Some(original) = original {
                found.insert(uid.clone(), original);
            }
        }
        Ok(found)
    }

    /// The original patient and study of `project`'s de-identified study `replaced_study`;
    /// `None` too for a study whose UID the project kept (`retain-uids`). Its copy has the
    /// original's own UID, so an identified instance of the study cannot be told from a
    /// result made on the copy, and none is taken for one.
    pub async fn original_study(
        &self,
        project: &str,
        replaced_study: &str,
    ) -> Result<Option<OriginalStudy>, Error> {
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT patient_main_tags, study_main_tags FROM project_studies
              WHERE project = $1 AND replaced_study = $2
                AND EXISTS (SELECT 1 FROM project_uids u
                             WHERE u.project = $1 AND u.replaced = $2 AND u.original <> $2)",
        )
        .bind(project)
        .bind(replaced_study)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|(patient, study)| {
            Ok(OriginalStudy {
                patient: object(&patient)?,
                study: object(&study)?,
            })
        })
        .transpose()
    }
}
