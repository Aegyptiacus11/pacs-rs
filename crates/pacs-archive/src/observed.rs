//! What a request stored, for whoever must account for it (the server's audit trail).
//!
//! The archive knows which study and patient an instance belongs to; the layer around the
//! request knows who sent it. [`stored_during`] runs a request and hands back what
//! [`Archive::ingest`](crate::Archive::ingest) took in while it ran, on the same task.

use std::cell::RefCell;
use std::future::Future;

use crate::Outcome;

/// One instance the archive accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub study_instance_uid: String,
    pub sop_class_uid: String,
    pub patient_id: Option<String>,
    pub patient_name: Option<String>,
    pub outcome: Outcome,
}

tokio::task_local! {
    static STORED: RefCell<Vec<Stored>>;
}

/// Run `request`, and return with its output every instance ingested while it ran.
///
/// Only what is ingested on the task that polls `request` is seen: an ingest on a task the
/// request spawns is not.
pub async fn stored_during<F: Future>(request: F) -> (F::Output, Vec<Stored>) {
    STORED
        .scope(RefCell::new(Vec::new()), async {
            let output = request.await;
            let stored = STORED.with(|stored| stored.take());
            (output, stored)
        })
        .await
}

/// Note an accepted instance, when someone is listening.
pub(crate) fn note(stored: impl FnOnce() -> Stored) {
    // outside a `stored_during` there is nobody to tell
    let _ = STORED.try_with(|list| list.borrow_mut().push(stored()));
}
