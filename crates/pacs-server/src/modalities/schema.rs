//! The answers of the remote AE routes as types, for the API's description. As in
//! `pacs_rest::schema`: they refuse a field they do not name, and `tests/dimse.rs` reads the
//! handlers' real answers into them.

use std::collections::BTreeMap;

use serde::Deserialize;
use utoipa::ToSchema;

pub use pacs_rest::schema::Error;

/// A remote AE as configured in `PACS_DIMSE_REMOTES`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteAe {
    /// Its AE title.
    #[serde(rename = "AET")]
    pub aet: String,
    #[serde(rename = "Host")]
    pub host: String,
    #[serde(rename = "Port")]
    pub port: u16,
}

/// The remote AEs: their ids, or with `expand` each id's AE.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum RemoteAes {
    Ids(Vec<String>),
    Expanded(BTreeMap<String, RemoteAe>),
}

/// A C-ECHO that the remote AE answered.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Echo {
    /// `Success`.
    pub status: String,
    /// The AE title that answered.
    #[serde(rename = "AET")]
    pub aet: String,
    /// From the first byte sent to the association's release.
    pub elapsed_ms: u64,
}

/// One study as a remote AE describes it: its attributes in DICOM JSON, by tag.
#[derive(Debug, Deserialize, ToSchema)]
#[schema(value_type = Object)]
pub struct DicomJson(pub serde_json::Value);

/// One Study Instance UID, or several.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum Uids {
    One(String),
    Several(Vec<String>),
}

/// Which studies to bring in, and how.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Move {
    #[serde(rename = "StudyInstanceUID")]
    pub study_instance_uid: Uids,
    /// The name of a project: the studies are sent to its AE title, and de-identified on
    /// the way in. Without it they come to pacs-rs's own AE title, as they are.
    pub project: Option<String>,
    /// Answer at once with a job, instead of when the move is over.
    pub asynchronous: Option<bool>,
}

/// How a move ended.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Moved {
    /// `Success`, or `Warning` when some instances failed.
    pub status: String,
    /// Instances sent, as the remote AE counted them; `null` when it gave no count.
    #[schema(required)]
    pub completed: Option<u16>,
    #[schema(required)]
    pub failed: Option<u16>,
    #[schema(required)]
    pub warning: Option<u16>,
    /// The instances the remote AE said it could not send.
    #[serde(rename = "FailedSOPInstanceUIDs")]
    pub failed_sop_instance_uids: Vec<String>,
    pub elapsed_ms: u64,
}

/// A move started in the background.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Started {
    /// The job's id.
    #[serde(rename = "ID")]
    pub id: String,
    /// Where to ask how it goes: `/jobs/{ID}`.
    pub path: String,
}

/// A move in the background, or one that is over. Jobs are kept in memory, the latest
/// hundred finished ones with them: a restart forgets them.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Job {
    #[serde(rename = "ID")]
    pub id: String,
    /// `C-MOVE`.
    pub r#type: String,
    /// The id of the remote AE asked.
    pub remote: String,
    #[serde(rename = "StudyInstanceUIDs")]
    pub study_instance_uids: Vec<String>,
    /// `Running`, `Success`, `Warning`, `Cancelled` or `Failure`.
    pub state: String,
    /// Percent of the instances done; `null` while the remote AE has given no count.
    #[schema(required)]
    pub progress: Option<u64>,
    /// The counts of the latest response; `null` for one the remote AE has not given.
    #[schema(required)]
    pub remaining: Option<u16>,
    #[schema(required)]
    pub completed: Option<u16>,
    #[schema(required)]
    pub failed: Option<u16>,
    #[schema(required)]
    pub warning: Option<u16>,
    #[serde(rename = "FailedSOPInstanceUIDs")]
    pub failed_sop_instance_uids: Vec<String>,
    /// Why it failed; `null` unless `State` is `Failure`.
    #[schema(required)]
    pub error: Option<String>,
    pub elapsed_ms: u64,
}
