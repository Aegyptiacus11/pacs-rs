//! Counters of what the server does, kept by the crates that do it and read by the server's
//! metrics endpoint: process-wide atomics, nothing to configure, no cost worth measuring.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// DIMSE associations being served now.
pub static ASSOCIATIONS_OPEN: AtomicI64 = AtomicI64::new(0);
/// DIMSE associations established since the start.
pub static ASSOCIATIONS_ACCEPTED: AtomicU64 = AtomicU64::new(0);
/// Connections that did not become an association: refused, unreadable, the limit reached.
pub static ASSOCIATIONS_REFUSED: AtomicU64 = AtomicU64::new(0);
/// Instances stored since the start, by outcome.
pub static INSTANCES_ADDED: AtomicU64 = AtomicU64::new(0);
pub static INSTANCES_REPLACED: AtomicU64 = AtomicU64::new(0);
pub static INSTANCES_ALREADY_STORED: AtomicU64 = AtomicU64::new(0);

pub fn add(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

/// An association being served, as the administration page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAssociation {
    pub id: u64,
    /// The caller's address.
    pub peer: String,
    pub calling: String,
    pub called: String,
    pub since: std::time::SystemTime,
    /// Instances it stored, queries it made, retrievals it asked, so far.
    pub stored: u64,
    pub queries: u64,
    pub retrievals: u64,
}

type Active = std::collections::BTreeMap<u64, ActiveAssociation>;
static ACTIVE: std::sync::LazyLock<std::sync::Mutex<Active>> =
    std::sync::LazyLock::new(Default::default);
static NEXT_ASSOCIATION: AtomicU64 = AtomicU64::new(1);

/// The associations being served now, the oldest first.
pub fn active_associations() -> Vec<ActiveAssociation> {
    ACTIVE
        .lock()
        .map(|active| active.values().cloned().collect())
        .unwrap_or_default()
}

/// One association more while the guard lives: counted, and listed with what it does.
pub struct OpenAssociation(u64);

impl OpenAssociation {
    pub fn new(peer: &str, calling: &str, called: &str) -> Self {
        ASSOCIATIONS_OPEN.fetch_add(1, Ordering::Relaxed);
        add(&ASSOCIATIONS_ACCEPTED);
        let id = NEXT_ASSOCIATION.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut active) = ACTIVE.lock() {
            active.insert(
                id,
                ActiveAssociation {
                    id,
                    peer: peer.to_owned(),
                    calling: calling.to_owned(),
                    called: called.to_owned(),
                    since: std::time::SystemTime::now(),
                    stored: 0,
                    queries: 0,
                    retrievals: 0,
                },
            );
        }
        OpenAssociation(id)
    }

    /// What it has done so far.
    pub fn served(&self, stored: u64, queries: u64, retrievals: u64) {
        if let Ok(mut active) = ACTIVE.lock()
            && let Some(entry) = active.get_mut(&self.0)
        {
            (entry.stored, entry.queries, entry.retrievals) = (stored, queries, retrievals);
        }
    }
}

impl Drop for OpenAssociation {
    fn drop(&mut self) {
        ASSOCIATIONS_OPEN.fetch_sub(1, Ordering::Relaxed);
        if let Ok(mut active) = ACTIVE.lock() {
            active.remove(&self.0);
        }
    }
}
