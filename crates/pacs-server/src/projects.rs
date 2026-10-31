//! Projects: `PACS_PROJECTS` names a JSON file listing them, each an AE title of its own and
//! a de-identification profile (pacs-deid), e.g.
//!
//! ```json
//! [{"ae_title": "LIVER_2026",
//!   "profile": {"name": "liver-2026", "options": ["retain-longitudinal-modified-dates"]}}]
//! ```
//!
//! Each project's secret, from which its pseudonyms, UIDs and date shifts derive, comes from
//! the environment only: `PACS_PROJECT_SECRET_<AE title>` (letters and digits kept, anything
//! else `_`), or the file named by that variable with `_FILE` appended.

use std::sync::Arc;

use anyhow::{Context, bail};
use pacs_archive::Archive;
use pacs_deid::{Keys, Profile};
use pacs_dimse::{AeTitle, Project};
use serde::Deserialize;

use crate::{Settings, open_store, process_env, secret};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    ae_title: String,
    profile: Profile,
}

/// A project as configured: its AE title and its profile, checked.
#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub ae_title: AeTitle,
    pub profile: Profile,
}

/// The variable a project's secret is read from.
pub fn secret_variable(ae_title: &AeTitle) -> String {
    let name: String = ae_title
        .as_str()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("PACS_PROJECT_SECRET_{name}")
}

/// The projects of `PACS_PROJECTS`, each profile checked; names and AE titles unique, and no
/// AE title one pacs-rs uses already.
pub fn project_configs(settings: &Settings) -> anyhow::Result<Vec<ProjectConfig>> {
    let Some(path) = &settings.projects else {
        return Ok(Vec::new());
    };
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading PACS_PROJECTS ({})", path.display()))?;
    let entries: Vec<Entry> = serde_json::from_str(&text).with_context(|| {
        format!(
            "PACS_PROJECTS ({}) is not a list of projects",
            path.display()
        )
    })?;
    let mut projects: Vec<ProjectConfig> = Vec::new();
    for entry in entries {
        let ae_title = AeTitle::new(&entry.ae_title)
            .with_context(|| format!("project {}", entry.profile.name))?;
        entry
            .profile
            .check()
            .with_context(|| format!("project {}", entry.profile.name))?;
        let own = std::iter::once(&settings.dimse_ae_title).chain(&settings.dimse_extra_ae_titles);
        if own.into_iter().any(|t| t == &ae_title) {
            bail!(
                "project {}: {ae_title} is already one of pacs-rs's own AE titles",
                entry.profile.name
            );
        }
        if projects
            .iter()
            .any(|p| p.ae_title == ae_title || p.profile.name == entry.profile.name)
        {
            bail!(
                "project {} ({ae_title}) is listed twice",
                entry.profile.name
            );
        }
        projects.push(ProjectConfig {
            ae_title,
            profile: entry.profile,
        });
    }
    Ok(projects)
}

/// The projects ready to receive: each with its secret and the archive, and the quarantine
/// store of `PACS_QUARANTINE` if there is one.
pub fn open_projects(
    settings: &Settings,
    archive: &Archive,
    env: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Vec<Arc<Project>>> {
    let quarantine = match &settings.quarantine {
        Some(location) => Some(open_store(location, settings).context("opening PACS_QUARANTINE")?),
        None => None,
    };
    project_configs(settings)?
        .into_iter()
        .map(|config| {
            let variable = secret_variable(&config.ae_title);
            let secret = secret(&variable, &env)?.with_context(|| {
                format!(
                    "project {} needs a secret in {variable} (or {variable}_FILE), \
                     at least 32 bytes, e.g. from `openssl rand -hex 32`",
                    config.profile.name
                )
            })?;
            let keys = Keys::new(secret.into_bytes()).with_context(|| variable.clone())?;
            Ok(Arc::new(Project {
                ae_title: config.ae_title,
                profile: config.profile,
                keys,
                archive: archive.clone(),
                quarantine: quarantine.clone(),
            }))
        })
        .collect()
}

/// [`open_projects`] with the process environment.
pub fn open_projects_from_env(
    settings: &Settings,
    archive: &Archive,
) -> anyhow::Result<Vec<Arc<Project>>> {
    open_projects(settings, archive, process_env)
}
