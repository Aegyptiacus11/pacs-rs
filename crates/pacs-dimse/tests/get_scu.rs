//! The C-GET SCU (PS3.4 C.4.3.2, C.5.3): pacs-rs asks an archive for a study and receives
//! it on the association it asks on, the instances stored as the C-STORE SCP stores them.
//! The archive is pacs-rs's own Retrieve SCP, serving real images from another archive.

mod common;

use std::ops::ControlFlow;
use std::sync::Mutex;
use std::time::Duration;

use common::*;
use pacs_archive::Archive;
use pacs_archive::observed::Stored;
use pacs_core::real::RealSeries;
use pacs_dimse::scu::get::Receiving;
use pacs_dimse::{
    AeTitle, ConnectError, Destination, MoveError, Observer, Peer, Queried, RemoteAe, Retrieved,
    Status, get_studies,
};

/// Records what a C-GET stored, as the audit trail is told.
#[derive(Debug, Default)]
struct Told(Mutex<Vec<(Peer, Vec<Stored>)>>);

impl Observer for Told {
    fn queried(&self, _: &Peer, _: &Queried<'_>) {}
    fn retrieved(&self, _: &Peer, _: &Retrieved<'_>) {}
    fn received(&self, peer: &Peer, stored: Vec<Stored>) {
        self.0.lock().unwrap().push((peer.clone(), stored));
    }
    fn refused(&self, _: std::net::SocketAddr, _: &str) {}
}

/// The hospital's archive, served as HOSPITAL: Doe's study of two series (two and three
/// slices) and Roe's of one.
async fn hospital() -> (RemoteAe, [RealSeries; 3]) {
    let archive = archive().await;
    let series = [
        RealSeries::new(801, 2).with_patient("Doe^Jane", None),
        RealSeries::new(802, 3)
            .in_study(801, 2)
            .with_patient("Doe^Jane", None),
        RealSeries::new(803, 1).with_patient("Roe^Richard", None),
    ];
    for one in &series {
        ingest(&archive, one).await;
    }
    let mut config = config();
    config.ae_title = AeTitle::new("HOSPITAL").unwrap();
    // the hospital answers the retrievals of the AEs it knows
    config.remotes = vec!["pacsrs=PACSRS@127.0.0.1:11112".parse().unwrap()];
    let addr = start(config, archive).await;
    let remote = format!("hospital=HOSPITAL@127.0.0.1:{}", addr.port())
        .parse()
        .unwrap();
    (remote, series)
}

async fn pull(
    remote: &RemoteAe,
    into: &Archive,
    studies: &[String],
    observer: Option<&dyn Observer>,
    max_data_set_bytes: usize,
    stop_after: Option<u16>,
) -> Result<pacs_dimse::MoveOutcome, MoveError> {
    let destination = Destination::Archive(into.clone());
    get_studies(
        remote,
        &AeTitle::new("PACSRS").unwrap(),
        None,
        studies,
        Receiving {
            destination: &destination,
            max_data_set_bytes,
            observer,
        },
        Duration::from_secs(5),
        Duration::from_secs(5),
        |progress| match (stop_after, progress.completed) {
            (Some(n), Some(done)) if done >= n => ControlFlow::Break(()),
            _ => ControlFlow::Continue(()),
        },
    )
    .await
}

async fn instances_of(archive: &Archive, study: &str) -> usize {
    let index = archive.index();
    let mut count = 0;
    for series in index.series(study, None).await.unwrap() {
        count += index
            .instances(study, &series.series_instance_uid, None)
            .await
            .unwrap()
            .len();
    }
    count
}

/// The study asked for comes back on the association, every instance stored and indexed,
/// the other study left where it is; the audit is told what arrived and from whom.
#[tokio::test]
async fn a_study_comes_back_on_the_association_that_asks() {
    let (remote, [doe, _, roe]) = hospital().await;
    let here = archive().await;
    let told = Told::default();
    let outcome = pull(
        &remote,
        &here,
        &[doe.study_uid()],
        Some(&told),
        1 << 30,
        None,
    )
    .await
    .unwrap();
    assert_eq!(outcome.status, Status::SUCCESS);
    assert_eq!(
        (outcome.progress.completed, outcome.progress.failed),
        (Some(5), Some(0))
    );
    assert_eq!(instances_of(&here, &doe.study_uid()).await, 5);
    assert_eq!(instances_of(&here, &roe.study_uid()).await, 0);
    // a stored file is the hospital's instance, read back whole
    let entry = here
        .index()
        .instance(&doe.study_uid(), &doe.series_uid(), &doe.instance_uid(1))
        .await
        .unwrap()
        .unwrap();
    let file = here.file(&entry).await.unwrap();
    let object = dicom_object::from_reader(&*file).unwrap();
    assert_eq!(
        object
            .meta()
            .media_storage_sop_instance_uid()
            .trim_end_matches('\0'),
        doe.instance_uid(1)
    );
    let told = told.0.lock().unwrap();
    let [(peer, stored)] = &told[..] else {
        panic!("told {} times", told.len())
    };
    assert_eq!(
        (peer.calling.as_str(), peer.called.as_str()),
        ("HOSPITAL", "PACSRS")
    );
    assert!(peer.address.ip().is_loopback());
    assert_eq!(stored.len(), 5);
    assert!(
        stored
            .iter()
            .all(|s| s.study_instance_uid == doe.study_uid())
    );
}

/// An instance larger than the limit is refused as the SCP refuses it: the archive counts
/// the failed sub-operations, and nothing is stored here.
#[tokio::test]
async fn what_is_too_large_is_refused_and_counted() {
    let (remote, [_, _, roe]) = hospital().await;
    let here = archive().await;
    let result = pull(&remote, &here, &[roe.study_uid()], None, 1000, None).await;
    let failed = match result {
        Ok(outcome) => outcome.progress.failed,
        Err(MoveError::Failed { progress, .. }) => progress.failed,
        Err(e) => panic!("{e}"),
    };
    assert_eq!(failed, Some(1));
    assert_eq!(instances_of(&here, &roe.study_uid()).await, 0);
}

/// Stopping sends C-CANCEL-GET-RQ: the archive ends with Cancel before the last instance.
#[tokio::test]
async fn stopping_cancels_the_get() {
    let (remote, [doe, _, _]) = hospital().await;
    let here = archive().await;
    let outcome = pull(&remote, &here, &[doe.study_uid()], None, 1 << 30, Some(1))
        .await
        .unwrap();
    assert!(outcome.cancelled);
    assert_eq!(outcome.status, Status::CANCEL);
    let arrived = instances_of(&here, &doe.study_uid()).await;
    assert!((1..5).contains(&arrived), "{arrived} arrived");
}

/// What cannot be asked is refused before any association; an archive without C-GET is
/// reported as such.
#[tokio::test]
async fn what_cannot_work_is_said() {
    let here = archive().await;
    let nowhere: RemoteAe = "hospital=HOSPITAL@127.0.0.1:9".parse().unwrap();
    for bad in [vec![], vec!["not a uid".to_owned()]] {
        let error = pull(&nowhere, &here, &bad, None, 1 << 30, None)
            .await
            .unwrap_err();
        assert!(matches!(error, MoveError::Request(_)), "{error}");
    }
    // an archive of C-FIND only
    let (remote, _seen) = common::scripted::serve(
        "QRSCP",
        pacs_dimse::query::STUDY_ROOT_FIND,
        &[EXPLICIT_VR_LE],
        |_| Vec::new(),
    )
    .await;
    let error = pull(&remote, &here, &["2.25.1".to_owned()], None, 1 << 30, None)
        .await
        .unwrap_err();
    // refused for want of a GET context, whether the archive accepted nothing or
    // something else
    assert!(
        matches!(
            &error,
            MoveError::Connect(ConnectError::NotAccepted(_) | ConnectError::Association { .. })
        ) && (error.to_string().contains("presentation contexts")
            || error.to_string().contains("did not accept")),
        "{error}"
    );
}
