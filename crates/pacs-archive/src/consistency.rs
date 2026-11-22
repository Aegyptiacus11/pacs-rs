//! The store and the index against each other: files the index does not know (orphans: a
//! crash between storing a file and indexing it), and index entries whose file is gone
//! (lost). Repair indexes an orphan again when it reads as DICOM, and removes a lost
//! entry, so that searches stop offering images that cannot be served.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use pacs_index::{AuditedStudy, Level, StoredFile};
use pacs_store::StoredObject;

use pacs_index::{NewInstance, Recorded};

use crate::{Archive, CheckError};

/// What the store and the index disagree on.
#[derive(Debug, Default)]
pub struct Inconsistencies {
    /// Objects in the store that no index entry names.
    pub orphans: Vec<StoredObject>,
    /// Index entries whose file is not in the store.
    pub lost: Vec<StoredFile>,
    /// Objects written within the grace time, left out: an instance being stored has its
    /// file a moment before its index entry.
    pub too_recent: usize,
    /// Objects and entries compared.
    pub objects: usize,
    pub entries: usize,
}

/// What repair did with an orphan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrphanRepair {
    /// Indexed again, as if just received.
    Indexed,
    /// A copy of an instance indexed under another key: left as it is.
    Copy { indexed_at: String },
    /// Not DICOM, or not an instance the archive takes: left as it is.
    Unreadable(String),
}

/// What repair did with a lost entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LostRepair {
    /// Removed from the index; these studies lost it (for the audit trail).
    Removed(Vec<AuditedStudy>),
    /// Found after all, or gone from the index meanwhile: nothing done.
    NoLongerLost,
}

impl Archive {
    /// Compare the store with the index. Objects written less than `grace` ago are left out.
    pub async fn inconsistencies(&self, grace: Duration) -> Result<Inconsistencies, CheckError> {
        // the index first: a file stored after this is recent, not an orphan
        let mut indexed: HashMap<String, StoredFile> = HashMap::new();
        let mut after = String::new();
        loop {
            let page = self.index.files_after(&after, 1000).await?;
            let Some(last) = page.last() else { break };
            after = last.sop_instance_uid.clone();
            for file in page {
                indexed.insert(file.object_key.clone(), file);
            }
        }
        let entries = indexed.len();
        let objects = self.store.list().await.map_err(CheckError::Store)?;
        let recent = SystemTime::now()
            .checked_sub(grace)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let mut found = Inconsistencies {
            objects: objects.len(),
            entries,
            ..Inconsistencies::default()
        };
        for object in objects {
            if indexed.remove(&object.key).is_some() {
                continue;
            }
            // deleted files waiting to be restored or emptied: the trash's, not orphans
            if self.store.is_trash(&object.key) {
                continue;
            }
            if object.modified > recent {
                found.too_recent += 1;
            } else {
                found.orphans.push(object);
            }
        }
        found.lost = indexed.into_values().collect();
        found.orphans.sort_by(|a, b| a.key.cmp(&b.key));
        found
            .lost
            .sort_by(|a, b| a.sop_instance_uid.cmp(&b.sop_instance_uid));
        Ok(found)
    }

    /// Index an orphan again from its bytes, when it reads as an instance.
    pub async fn repair_orphan(&self, orphan: &StoredObject) -> Result<OrphanRepair, CheckError> {
        let bytes: Bytes = match self.store.get(&orphan.key).await {
            Ok(bytes) => bytes,
            // gone since the listing: nothing left to repair
            Err(pacs_store::Error::NotFound(_)) => {
                return Ok(OrphanRepair::Unreadable("gone from the store".into()));
            }
            Err(e) => return Err(CheckError::Store(e)),
        };
        let instance = match pacs_core::Instance::from_bytes(&bytes) {
            Ok(instance) => instance,
            Err(e) => return Ok(OrphanRepair::Unreadable(format!("not DICOM: {e}"))),
        };
        // indexed where it is, under its own key: storing it again would leave it behind
        let _writing = self.writes.read().await;
        let new = match NewInstance::new(&instance, orphan.key.clone(), bytes.len()) {
            Ok(new) => new
                .with_sha256(crate::sha256_hex(&bytes))
                .keeping_existing(true),
            Err(e) => return Ok(OrphanRepair::Unreadable(format!("not indexable: {e}"))),
        };
        match self.index.record(&new).await {
            Ok(Recorded::Added) => {
                tracing::info!(orphan = %orphan.key, "an orphan indexed again");
                Ok(OrphanRepair::Indexed)
            }
            Ok(Recorded::Kept { existing_key })
            | Ok(Recorded::Replaced {
                previous_key: existing_key,
            }) => Ok(OrphanRepair::Copy {
                indexed_at: existing_key,
            }),
            Err(pacs_index::Error::Conflict(why)) => {
                Ok(OrphanRepair::Unreadable(format!("not taken: {why}")))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Remove a lost entry from the index, once its file is confirmed gone and the entry
    /// still names it.
    pub async fn repair_lost(&self, lost: &StoredFile) -> Result<LostRepair, CheckError> {
        let Some((public_id, object_key)) =
            self.index.instance_file(&lost.sop_instance_uid).await?
        else {
            return Ok(LostRepair::NoLongerLost);
        };
        if object_key != lost.object_key {
            return Ok(LostRepair::NoLongerLost);
        }
        match self.store.get(&object_key).await {
            Err(pacs_store::Error::NotFound(_)) => {}
            Ok(_) => return Ok(LostRepair::NoLongerLost),
            Err(e) => return Err(CheckError::Store(e)),
        }
        let studies = self
            .index
            .audited_by_public_id(Level::Instance, &public_id)
            .await?;
        self.index.delete(Level::Instance, &public_id).await?;
        tracing::warn!(sop_instance_uid = %lost.sop_instance_uid, object_key = %object_key, "an index entry without its file removed");
        Ok(LostRepair::Removed(studies))
    }
}
