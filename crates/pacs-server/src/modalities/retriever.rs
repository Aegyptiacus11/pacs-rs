//! Bringing studies in from a remote AE, by whichever retrieval works: C-MOVE when pacs-rs
//! listens for DIMSE and the remote AE sends to it, C-GET otherwise (PS3.4 C.4.2, C.4.3).
//!
//! A C-MOVE is asked first, the way a hospital archive is used to being asked. It gives way
//! to a C-GET, on which the studies come back on the association that asks, when the remote
//! AE does not know pacs-rs as a destination (status A801), when it takes no C-MOVE, and
//! when pacs-rs does not listen for DIMSE, so that nothing moved could arrive.

use std::ops::ControlFlow;
use std::sync::Arc;

use pacs_archive::Archive;
use pacs_dimse::scu::get::Receiving;
use pacs_dimse::{
    AeTitle, Config, ConnectError, Destination, MoveError, MoveOutcome, Observer, Progress,
    Project, RemoteAe, Status,
};

/// How a retrieval was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Move,
    Get,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Method::Move => "C-MOVE",
            Method::Get => "C-GET",
        }
    }
}

/// What a retrieval needs: the DIMSE configuration, whether pacs-rs listens for DIMSE, and
/// where what a C-GET brings goes (the archive, or a project's de-identification for the
/// project's AE title), with the audit trail told of it.
pub struct Retriever {
    pub config: Config,
    pub receiving: bool,
    archive: Archive,
    /// The projects, or why they could not be opened (a missing secret): a C-GET into a
    /// project then fails with that reason, and one into the archive still works.
    projects: Result<Vec<Arc<Project>>, String>,
    observer: Option<Arc<dyn Observer>>,
}

impl Retriever {
    pub fn new(
        config: Config,
        receiving: bool,
        archive: Archive,
        projects: Result<Vec<Arc<Project>>, String>,
        observer: Option<Arc<dyn Observer>>,
    ) -> Self {
        Retriever {
            config,
            receiving,
            archive,
            projects,
            observer,
        }
    }

    /// Where a C-GET stores what is sent for `ae_title`: pacs-rs's own AE title is the
    /// archive, a project's AE title that project.
    fn destination(&self, ae_title: &AeTitle) -> Result<Destination, String> {
        if *ae_title == self.config.ae_title {
            return Ok(Destination::Archive(self.archive.clone()));
        }
        let projects = self.projects.as_ref().map_err(Clone::clone)?;
        projects
            .iter()
            .find(|project| project.ae_title == *ae_title)
            .map(|project| Destination::Project(project.clone()))
            .ok_or_else(|| format!("{ae_title} is neither pacs-rs's AE title nor a project's"))
    }

    /// Bring `studies` of `remote` in for `to`, pacs-rs's AE title or a project's: by
    /// C-MOVE, or by C-GET when the move cannot be made (see the module). Each Pending
    /// response's counts go to `on_progress`, whose `Break` cancels. The outcome, and the
    /// operation that gave it.
    pub async fn retrieve<F>(
        &self,
        remote: &RemoteAe,
        patient_id: Option<&str>,
        studies: &[String],
        to: &AeTitle,
        mut on_progress: F,
    ) -> (Result<MoveOutcome, MoveError>, Method)
    where
        F: FnMut(Progress) -> ControlFlow<()>,
    {
        let config = &self.config;
        let mut refused_move = None;
        if self.receiving {
            let moved = pacs_dimse::move_studies(
                remote,
                &config.ae_title,
                patient_id,
                studies,
                to,
                config.timeout,
                config.move_timeout,
                &mut on_progress,
            )
            .await;
            match moved {
                Err(e @ MoveError::Failed { status, .. })
                    if status == Status::MOVE_DESTINATION_UNKNOWN =>
                {
                    tracing::info!(remote = %remote.id, "the remote AE does not know pacs-rs as a destination: C-GET instead");
                    refused_move = Some(e);
                }
                Err(e @ MoveError::Connect(ConnectError::NotAccepted(_))) => {
                    tracing::info!(remote = %remote.id, "the remote AE takes no C-MOVE: C-GET instead");
                    refused_move = Some(e);
                }
                other => return (other, Method::Move),
            }
        }
        let destination = match self.destination(to) {
            Ok(destination) => destination,
            Err(e) => return (Err(MoveError::Request(e)), Method::Get),
        };
        let got = pacs_dimse::get_studies(
            remote,
            &config.ae_title,
            patient_id,
            studies,
            Receiving {
                destination: &destination,
                max_data_set_bytes: config.max_data_set_bytes,
                observer: self.observer.as_deref(),
            },
            config.timeout,
            config.move_timeout,
            &mut on_progress,
        )
        .await;
        match (got, refused_move) {
            // an archive that takes neither: the move's refusal says more, A801 above all
            (Err(MoveError::Connect(_)), Some(refused)) => (Err(refused), Method::Move),
            (got, _) => (got, Method::Get),
        }
    }

    /// Send every instance of `studies`, as `index` sees them, to `remote` by C-STORE.
    pub async fn send<F>(
        &self,
        remote: &RemoteAe,
        index: &pacs_index::Index,
        studies: &[String],
        on_progress: F,
    ) -> Result<MoveOutcome, MoveError>
    where
        F: FnMut(Progress) -> ControlFlow<()>,
    {
        let instances = match super::send::instances_of(index, studies).await {
            Ok(Ok(instances)) => instances,
            Ok(Err(missing)) => {
                return Err(MoveError::Request(format!(
                    "{missing} is not in the archive"
                )));
            }
            Err(e) => return Err(MoveError::Request(format!("the index: {e}"))),
        };
        pacs_dimse::send_instances(
            remote,
            &self.config.ae_title,
            &self.archive,
            &instances,
            self.config.timeout,
            on_progress,
        )
        .await
    }
}
