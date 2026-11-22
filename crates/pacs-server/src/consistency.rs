//! `pacs-server --consistency [--repair]`: the store and the index compared, with the
//! server's own settings (see `pacs_archive::Inconsistencies`).
//!
//! It lists the files no index entry names (orphans: a crash between storing a file and
//! indexing it) and the index entries whose file is gone (lost), and exits 1 when there are
//! any, 0 when the two agree. With `--repair`:
//!
//! - an orphan that reads as DICOM is indexed again, as if just received; one that does not,
//!   or that is a copy of an instance indexed elsewhere, is listed and left where it is:
//!   repair never deletes a file;
//! - a lost entry is removed from the index (its series and study too when nothing else is
//!   left in them), once its file is confirmed gone, and the removal is in the audit trail as
//!   a deletion by the server.
//!
//! Files written in the last ten minutes are left out: an instance being stored has its file
//! a moment before its index entry. It can run beside a running server.

use std::io::Write;
use std::time::Duration;

use anyhow::{Context, bail};
use pacs_archive::{Archive, LostRepair, OrphanRepair};

use crate::Settings;
use crate::audit::Audit;

/// How recent a file may be and still be left out.
pub const GRACE: Duration = Duration::from_secs(600);

/// A quarantine kept inside the store would look like orphans, and repair would index files
/// that must never be: refuse to compare then.
pub fn check_quarantine(settings: &Settings) -> anyhow::Result<()> {
    let Some(quarantine) = &settings.quarantine else {
        return Ok(());
    };
    let (store, quarantine) = (
        settings.storage.trim_end_matches('/'),
        quarantine.trim_end_matches('/'),
    );
    if quarantine == store || quarantine.starts_with(&format!("{store}/")) {
        bail!(
            "PACS_QUARANTINE ({quarantine}) is inside PACS_STORAGE ({store}): its files would \
             look like files the index lost, so the store and the index are not compared"
        );
    }
    Ok(())
}

/// What the comparison found, and with `repair`, what was done: written to `out`. Whether
/// the store and the index agree (after repair, when asked).
pub async fn run(
    archive: &Archive,
    audit: &Audit,
    repair: bool,
    grace: Duration,
    out: &mut impl Write,
) -> anyhow::Result<bool> {
    let found = archive
        .inconsistencies(grace)
        .await
        .context("comparing the store and the index")?;
    writeln!(
        out,
        "{} objects in the store, {} index entries; {} written too recently to judge",
        found.objects, found.entries, found.too_recent
    )?;
    let mut left = 0;
    for orphan in &found.orphans {
        if !repair {
            writeln!(out, "orphan: {} ({} bytes)", orphan.key, orphan.size)?;
            left += 1;
            continue;
        }
        match archive.repair_orphan(orphan).await? {
            OrphanRepair::Indexed => writeln!(out, "orphan indexed again: {}", orphan.key)?,
            OrphanRepair::Copy { indexed_at } => {
                left += 1;
                writeln!(
                    out,
                    "orphan left: {}: a copy of an instance indexed at {indexed_at}",
                    orphan.key
                )?;
            }
            OrphanRepair::Unreadable(why) => {
                left += 1;
                writeln!(out, "orphan left: {}: {why}", orphan.key)?;
            }
        }
    }
    for lost in &found.lost {
        if !repair {
            writeln!(
                out,
                "lost: {} (its file {} is not in the store)",
                lost.sop_instance_uid, lost.object_key
            )?;
            left += 1;
            continue;
        }
        match archive.repair_lost(lost).await? {
            LostRepair::Removed(studies) => {
                audit
                    .removed_by_repair(archive.index(), &studies)
                    .await
                    .context("the audit trail of a removed entry")?;
                writeln!(out, "lost entry removed: {}", lost.sop_instance_uid)?;
            }
            LostRepair::NoLongerLost => {
                writeln!(out, "no longer lost: {}", lost.sop_instance_uid)?;
            }
        }
    }
    writeln!(
        out,
        "{} orphans, {} lost{}",
        found.orphans.len(),
        found.lost.len(),
        if repair {
            format!("; {left} left as they are")
        } else {
            String::new()
        }
    )?;
    Ok(left == 0)
}
