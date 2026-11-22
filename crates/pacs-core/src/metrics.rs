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

/// One association more while the guard lives.
pub struct OpenAssociation(());

impl OpenAssociation {
    pub fn new() -> Self {
        ASSOCIATIONS_OPEN.fetch_add(1, Ordering::Relaxed);
        add(&ASSOCIATIONS_ACCEPTED);
        OpenAssociation(())
    }
}

impl Default for OpenAssociation {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for OpenAssociation {
    fn drop(&mut self) {
        ASSOCIATIONS_OPEN.fetch_sub(1, Ordering::Relaxed);
    }
}
