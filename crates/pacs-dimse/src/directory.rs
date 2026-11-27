//! The remote AEs pacs-rs knows, shared by everything that calls or answers them (the REST
//! routes, the retriever, the DIMSE server) and changed while the server runs: a remote AE
//! added, changed or removed is seen by the next association, the next request, with no
//! restart. Each read takes a copy; no lock is held across an await.

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::{ClientTls, RemoteAe};

#[derive(Debug, Default)]
struct Entries {
    remotes: Vec<RemoteAe>,
    /// How to speak TLS to a secure remote AE: given to each, those added later included.
    client_tls: Option<ClientTls>,
    /// The ids of those instances may be sent to: the destinations a C-MOVE may name.
    move_destinations: Vec<String>,
    /// The ids of those set where the server is configured, not changed while it runs.
    fixed: Vec<String>,
    /// Whether it was filled from the configuration (once, however often it is asked for).
    filled: bool,
}

/// The remote AEs, shared: a clone is the same directory.
#[derive(Clone, Default)]
pub struct Directory(Arc<RwLock<Entries>>);

impl std::fmt::Debug for Directory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let e = self.read();
        f.debug_struct("Directory")
            .field("remotes", &e.remotes)
            .field("move_destinations", &e.move_destinations)
            .finish()
    }
}

impl Directory {
    pub fn new(remotes: Vec<RemoteAe>, move_destinations: Vec<String>) -> Self {
        Directory(Arc::new(RwLock::new(Entries {
            remotes,
            client_tls: None,
            move_destinations,
            fixed: Vec::new(),
            filled: false,
        })))
    }

    // A writer that panicked left the list as it was before or after its one assignment:
    // still a list, so a poisoned lock is read through.
    fn read(&self) -> RwLockReadGuard<'_, Entries> {
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    fn write(&self) -> RwLockWriteGuard<'_, Entries> {
        self.0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Every remote AE, in the order they were added.
    pub fn remotes(&self) -> Vec<RemoteAe> {
        self.read().remotes.clone()
    }

    pub fn len(&self) -> usize {
        self.read().remotes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.read().remotes.is_empty()
    }

    pub fn remote(&self, id: &str) -> Option<RemoteAe> {
        self.read().remotes.iter().find(|r| r.id == id).cloned()
    }

    /// Whether instances may be sent to the remote AE `id`.
    pub fn may_receive(&self, id: &str) -> bool {
        self.read().move_destinations.iter().any(|d| d == id)
    }

    pub fn move_destinations(&self) -> Vec<String> {
        self.read().move_destinations.clone()
    }

    /// The remote AE a C-MOVE to `ae_title` sends to: one that may receive, with that AE
    /// title. `None` is a Move Destination this server does not know (PS3.4 Table C.4-2,
    /// status A801).
    pub fn move_destination(&self, ae_title: &str) -> Option<RemoteAe> {
        let e = self.read();
        e.remotes
            .iter()
            .find(|r| r.ae_title.matches(ae_title) && e.move_destinations.contains(&r.id))
            .cloned()
    }

    /// Whether a remote AE has this AE title.
    pub fn knows_ae_title(&self, ae_title: &str) -> bool {
        self.read()
            .remotes
            .iter()
            .any(|r| r.ae_title.matches(ae_title))
    }

    /// Adds `remote`, or replaces the one with its id in place; `may_receive` says whether
    /// instances may be sent to it.
    pub fn set(&self, mut remote: RemoteAe, may_receive: bool) {
        let mut e = self.write();
        if remote.secure && remote.tls.is_none() {
            remote.tls = e.client_tls.clone();
        }
        let id = remote.id.clone();
        match e.remotes.iter_mut().find(|r| r.id == id) {
            Some(slot) => *slot = remote,
            None => e.remotes.push(remote),
        }
        e.move_destinations.retain(|d| *d != id);
        if may_receive {
            e.move_destinations.push(id);
        }
    }

    /// Says whether instances may be sent to the remote AE `id`; whether there is one.
    pub fn set_may_receive(&self, id: &str, may_receive: bool) -> bool {
        let mut e = self.write();
        if !e.remotes.iter().any(|r| r.id == id) {
            return false;
        }
        e.move_destinations.retain(|d| d != id);
        if may_receive {
            e.move_destinations.push(id.to_owned());
        }
        true
    }

    /// Takes the remote AE `id` away; whether there was one.
    pub fn remove(&self, id: &str) -> bool {
        let mut e = self.write();
        let before = e.remotes.len();
        e.remotes.retain(|r| r.id != id);
        e.move_destinations.retain(|d| d != id);
        e.remotes.len() != before
    }

    /// How pacs-rs speaks TLS to secure remote AEs: given to those there and to those added.
    pub fn set_client_tls(&self, tls: Option<ClientTls>) {
        let mut e = self.write();
        for remote in e.remotes.iter_mut().filter(|r| r.secure) {
            remote.tls = tls.clone();
        }
        e.client_tls = tls;
    }

    /// Says that the remote AEs there now come from the configuration: not to be changed
    /// while the server runs ([`Directory::is_fixed`]), and not to be filled again.
    pub fn mark_filled(&self) {
        let mut e = self.write();
        e.fixed = e.remotes.iter().map(|r| r.id.clone()).collect();
        e.filled = true;
    }

    pub fn is_filled(&self) -> bool {
        self.read().filled
    }

    /// Whether the remote AE `id` comes from the configuration.
    pub fn is_fixed(&self, id: &str) -> bool {
        self.read().fixed.iter().any(|f| f == id)
    }

    /// Whether a secure remote AE can be called: pacs-rs has TLS to call it with.
    pub fn speaks_tls(&self) -> bool {
        self.read().client_tls.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ae(text: &str) -> RemoteAe {
        text.parse().unwrap()
    }

    #[test]
    fn a_clone_is_the_same_directory_and_sees_every_change() {
        let directory = Directory::new(vec![ae("pacs=PACS@10.0.0.1:104")], vec![]);
        let seen_by_the_server = directory.clone();
        directory.set(ae("ws=WS@10.0.0.2:11112"), true);
        assert_eq!(seen_by_the_server.len(), 2);
        assert_eq!(seen_by_the_server.move_destination("WS").unwrap().id, "ws");
        assert!(seen_by_the_server.move_destination("PACS").is_none());
        // replaced in place, its right to receive with it
        directory.set(ae("ws=WS2@10.0.0.3:104"), false);
        assert_eq!(seen_by_the_server.remote("ws").unwrap().host, "10.0.0.3");
        assert!(!seen_by_the_server.may_receive("ws"));
        assert_eq!(
            seen_by_the_server
                .remotes()
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["pacs", "ws"]
        );
        assert!(directory.remove("pacs"));
        assert!(!directory.remove("pacs"));
        assert!(!seen_by_the_server.knows_ae_title("PACS"));
    }
}
