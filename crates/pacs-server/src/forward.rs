//! Forwarding by rule (`PACS_FORWARD`): what arrives is sent on to other AEs, without anyone
//! asking each time, as a hospital routes a scanner's studies to a viewer or a research
//! archive.
//!
//! A rule names a remote AE studies may be sent to (`PACS_DIMSE_MOVE_DESTINATIONS`) and,
//! after a colon, what a study must be to go there: `viewer` sends every study,
//! `research:Modality=CT;StudyDescription=*LIVER*` the CT studies of the liver. Each key is
//! matched as a search matches it (PS3.4 C.2.2.2: a value, or a wild card with `*` and `?`;
//! person names without regard to case), several values of one key are joined by `|`, and
//! every key must be met. The series keys (Modality, Series Description) are met by one
//! series of the study, the same one for all of them.
//!
//! A study is forwarded once it has been quiet for `PACS_FORWARD_AFTER` seconds: no instance
//! of it stored since. The index notes it with each instance it records, in the same write,
//! so that a study stored is forwarded even when the server stops before it is; one server
//! of those sharing the index takes it out, and sends it as a job of sending (`/jobs`),
//! taken up again after a restart like any other. A study that takes in instances later is
//! forwarded again, whole: the remote AE keeps what it already has.
//!
//! Write-back is the same for results (an instance made on a project's de-identified copy,
//! see `pacs_archive`): a study that took in results is noted with them, and once quiet its
//! results go to the remote AE its project names (`"write_back"`), each re-identified by the
//! way back the project recorded, as a job of write-back (`/jobs`, `WRITE-BACK`).

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use pacs_index::{Index, Kind, Match};
use serde_json::Value;

use crate::modalities::jobs;
use crate::modalities::retriever::Retriever;

/// Where an attribute of a rule is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Study,
    Series,
}

/// The keys a rule can name: attributes the index keeps of a study or of its series.
const KEYS: &[(&str, &str, Level, Kind)] = &[
    ("PatientID", "00100020", Level::Study, Kind::Text),
    ("PatientName", "00100010", Level::Study, Kind::Name),
    ("AccessionNumber", "00080050", Level::Study, Kind::Text),
    ("StudyDescription", "00081030", Level::Study, Kind::Text),
    ("StudyID", "00200010", Level::Study, Kind::Text),
    (
        "ReferringPhysicianName",
        "00080090",
        Level::Study,
        Kind::Name,
    ),
    ("Modality", "00080060", Level::Series, Kind::Text),
    ("SeriesDescription", "0008103E", Level::Series, Kind::Text),
];

/// One condition of a rule: an attribute, and the values it may match, any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Condition {
    tag: &'static str,
    level: Level,
    kind: Kind,
    any_of: Vec<Match>,
}

impl Condition {
    fn answered_by(&self, attributes: &Value) -> bool {
        self.any_of
            .iter()
            .any(|asked| asked.answered_by(self.kind, attributes, self.tag))
    }
}

/// A rule of `PACS_FORWARD`: a destination, the conditions a study must meet, and the
/// projects it must be in when some are named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardRule {
    /// The remote AE's id in `PACS_DIMSE_REMOTES`.
    pub destination: String,
    conditions: Vec<Condition>,
    /// `Project=…`: a study in one of these projects, the name of each as `PACS_PROJECTS`
    /// gives it.
    projects: Option<Vec<String>>,
}

impl FromStr for ForwardRule {
    type Err = String;

    /// `destination` or `destination:Key=value|value;Key=value`.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (destination, conditions) = match text.split_once(':') {
            Some((destination, conditions)) => (destination.trim(), Some(conditions)),
            None => (text.trim(), None),
        };
        if destination.is_empty() {
            return Err(format!("a rule is destination[:Key=value;…], not {text:?}"));
        }
        let mut rule = ForwardRule {
            destination: destination.to_owned(),
            conditions: Vec::new(),
            projects: None,
        };
        let Some(conditions) = conditions else {
            return Ok(rule);
        };
        for condition in conditions.split(';') {
            let Some((key, values)) = condition.split_once('=') else {
                return Err(format!("rule {text:?}: {condition:?} is not Key=value"));
            };
            let key = key.trim();
            let values: Vec<&str> = values.split('|').map(str::trim).collect();
            if values.iter().any(|v| v.is_empty()) {
                return Err(format!("rule {text:?}: {key} has an empty value"));
            }
            let named = |known: &str| rule.conditions.iter().any(|c| c.tag == known);
            if key == "Project" {
                if rule.projects.is_some() {
                    return Err(format!("rule {text:?}: Project is given twice"));
                }
                rule.projects = Some(values.iter().map(|v| (*v).to_owned()).collect());
                continue;
            }
            let Some((_, tag, level, kind)) = KEYS.iter().find(|(keyword, ..)| *keyword == key)
            else {
                let known: Vec<&str> = KEYS.iter().map(|(keyword, ..)| *keyword).collect();
                return Err(format!(
                    "rule {text:?}: {key} is not a key of a rule; one of {}, or Project",
                    known.join(", ")
                ));
            };
            if named(tag) {
                return Err(format!("rule {text:?}: {key} is given twice"));
            }
            let mut any_of = Vec::new();
            for value in values {
                match Match::parse(*kind, value) {
                    Ok(Some(asked)) => any_of.push(asked),
                    // `*` alone: anything, which is no condition
                    Ok(None) => {
                        return Err(format!(
                            "rule {text:?}: {key}={value} matches anything; leave the key out"
                        ));
                    }
                    Err(e) => return Err(format!("rule {text:?}: {key}: {e}")),
                }
            }
            rule.conditions.push(Condition {
                tag,
                level: *level,
                kind: *kind,
                any_of,
            });
        }
        Ok(rule)
    }
}

impl ForwardRule {
    /// Whether a study, by its attributes, its series' and its project, meets the rule.
    fn takes(&self, study: &Value, series: &[Value], project: Option<&str>) -> bool {
        if let Some(projects) = &self.projects
            && !project.is_some_and(|p| projects.iter().any(|named| named == p))
        {
            return false;
        }
        let of = |level| self.conditions.iter().filter(move |c| c.level == level);
        if !of(Level::Study).all(|c| c.answered_by(study)) {
            return false;
        }
        let mut series_conditions = of(Level::Series).peekable();
        series_conditions.peek().is_none()
            || series
                .iter()
                .any(|one| of(Level::Series).all(|c| c.answered_by(one)))
    }
}

/// Forward the studies quiet for `quiet`: each taken out of the index once, and sent as a
/// job to each destination whose rule it meets. How many jobs started.
pub async fn forward_due(
    index: &Index,
    retriever: Arc<Retriever>,
    rules: &[ForwardRule],
    quiet: Duration,
) -> Result<usize, pacs_index::Error> {
    let before = jobs::now_ms() - i64::try_from(quiet.as_millis()).unwrap_or(i64::MAX);
    let mut started = 0;
    for study_uid in index.claim_forwards(before).await? {
        // deleted meanwhile: nothing to send
        let Some(study) = index.study(&study_uid).await? else {
            continue;
        };
        let series: Vec<Value> = index
            .series(&study_uid, None)
            .await?
            .into_iter()
            .map(|one| one.attributes)
            .collect();
        let project = index.study_project(&study_uid).await?.flatten();
        for rule in rules {
            if !rule.takes(&study.attributes, &series, project.as_deref()) {
                continue;
            }
            let id = jobs::start_send(
                index.clone(),
                retriever.clone(),
                rule.destination.clone(),
                vec![study_uid.clone()],
            )
            .await?;
            tracing::info!(study = %study_uid, destination = %rule.destination, job = id, "a study forwarded by a rule");
            started += 1;
        }
    }
    Ok(started)
}

/// Forward due studies for the life of the server, looking every few seconds.
pub fn spawn_forwarding(
    index: Index,
    retriever: Arc<Retriever>,
    rules: Vec<ForwardRule>,
    quiet: Duration,
) {
    let every = quiet.clamp(Duration::from_secs(1), Duration::from_secs(5));
    tokio::spawn(async move {
        loop {
            if let Err(error) = forward_due(&index, retriever.clone(), &rules, quiet).await {
                tracing::error!(%error, "forwarding the quiet studies failed; trying again");
            }
            tokio::time::sleep(every).await;
        }
    });
}

/// Start a job of write-back for each study of a project whose latest result was stored at
/// least `quiet` ago, to the remote AE its project writes back to (`write_backs`, project
/// to remote AE id); how many started. A study of a project that does not write back is
/// taken out and left: its results stay in the project.
pub async fn write_back_due(
    index: &Index,
    retriever: Arc<Retriever>,
    write_backs: &std::collections::HashMap<String, String>,
    quiet: Duration,
) -> Result<usize, pacs_index::Error> {
    let before = jobs::now_ms() - i64::try_from(quiet.as_millis()).unwrap_or(i64::MAX);
    let mut started = 0;
    for study_uid in index.claim_write_backs(before).await? {
        let Some(project) = index.study_project(&study_uid).await?.flatten() else {
            continue;
        };
        let Some(remote) = write_backs.get(&project) else {
            tracing::info!(study = %study_uid, %project, "results kept: the project writes nothing back");
            continue;
        };
        let id = jobs::start_write_back(
            index.clone(),
            retriever.clone(),
            remote.clone(),
            vec![study_uid.clone()],
        )
        .await?;
        tracing::info!(study = %study_uid, %project, destination = %remote, job = id, "results written back");
        started += 1;
    }
    Ok(started)
}

/// Write results back for the life of the server, looking every few seconds.
pub fn spawn_write_back(
    index: Index,
    retriever: Arc<Retriever>,
    write_backs: std::collections::HashMap<String, String>,
    quiet: Duration,
) {
    let every = quiet.clamp(Duration::from_secs(1), Duration::from_secs(5));
    tokio::spawn(async move {
        loop {
            if let Err(error) = write_back_due(&index, retriever.clone(), &write_backs, quiet).await
            {
                tracing::error!(%error, "writing results back failed; trying again");
            }
            tokio::time::sleep(every).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn study(description: &str, patient: &str) -> Value {
        json!({
            "00081030": {"vr": "LO", "Value": [description]},
            "00100010": {"vr": "PN", "Value": [{"Alphabetic": patient}]},
        })
    }

    fn series(modality: &str, description: &str) -> Value {
        json!({
            "00080060": {"vr": "CS", "Value": [modality]},
            "0008103E": {"vr": "LO", "Value": [description]},
        })
    }

    #[test]
    fn a_rule_without_conditions_takes_every_study() {
        let rule: ForwardRule = "viewer".parse().unwrap();
        assert_eq!(rule.destination, "viewer");
        assert!(rule.takes(&json!({}), &[], None));
    }

    #[test]
    fn every_condition_must_be_met() {
        let rule: ForwardRule = "research:Modality=CT|MR;StudyDescription=*LIVER*"
            .parse()
            .unwrap();
        let liver = study("CT ABDOMEN LIVER", "DOE^JANE");
        assert!(rule.takes(&liver, &[series("CT", "AXIAL")], None));
        assert!(rule.takes(&liver, &[series("US", ""), series("MR", "T1")], None));
        assert!(!rule.takes(&liver, &[series("US", "")], None));
        assert!(!rule.takes(&study("CHEST", "DOE^JANE"), &[series("CT", "")], None));
        // names without regard to case
        let rule: ForwardRule = "viewer:PatientName=doe^*".parse().unwrap();
        assert!(rule.takes(&liver, &[], None));
    }

    #[test]
    fn the_series_keys_are_met_by_one_series() {
        let rule: ForwardRule = "ai:Modality=CT;SeriesDescription=*ART*".parse().unwrap();
        let liver = study("LIVER", "DOE^JANE");
        assert!(rule.takes(&liver, &[series("CT", "ARTERIAL")], None));
        // a CT and an arterial series, but not one arterial CT
        assert!(!rule.takes(
            &liver,
            &[series("CT", "PORTAL"), series("MR", "ARTERIAL")],
            None
        ));
    }

    #[test]
    fn a_project_rule_takes_that_project_only() {
        let rule: ForwardRule = "research:Project=liver|kidney".parse().unwrap();
        assert!(rule.takes(&json!({}), &[], Some("liver")));
        assert!(!rule.takes(&json!({}), &[], Some("lung")));
        assert!(!rule.takes(&json!({}), &[], None));
    }

    #[test]
    fn a_rule_that_cannot_be_read_says_why() {
        for (rule, said) in [
            ("", "destination[:Key=value"),
            (":Modality=CT", "destination[:Key=value"),
            ("ai:Modality", "is not Key=value"),
            ("ai:Colour=red", "Colour is not a key of a rule"),
            ("ai:Modality=CT;Modality=MR", "Modality is given twice"),
            ("ai:Modality=", "empty value"),
            ("ai:Modality=CT|", "empty value"),
            ("ai:Modality=*", "matches anything"),
            ("ai:Project=a;Project=b", "Project is given twice"),
        ] {
            let error = rule.parse::<ForwardRule>().unwrap_err();
            assert!(error.contains(said), "{rule:?}: {error}");
        }
    }
}
