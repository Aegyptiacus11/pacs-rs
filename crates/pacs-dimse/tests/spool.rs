//! Streamed receive (`Config::spool`): a C-STORE's data set past the spool's bound is written
//! to disk as it arrives, then stored from there a part at a time, stored as received, its
//! file gone afterwards; under the archive's header bound it is read whole and compressed as
//! any other. On the real CT.

mod common;

use common::*;
use pacs_archive::Archive;
use pacs_core::real::RealSeries;
use pacs_dimse::{Config, SpoolTo};
use pacs_index::Index;
use pacs_store::Store;

const CT: &str = "1.2.840.10008.5.1.4.1.1.2";
const JPEG_LS: &str = "1.2.840.10008.1.2.4.80";

/// A server spooling past `from_bytes`, its archive reading at most `header_max` of a spooled
/// file for its header: its address, its archive, and the spool's folder.
async fn spooling(
    from_bytes: usize,
    header_max: usize,
) -> (std::net::SocketAddr, Archive, tempfile::TempDir) {
    let spool = tempfile::tempdir().unwrap();
    // compressed as the default says: only a file stored from its spool stays as received
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap())
        .with_spooled_header_max(header_max);
    let mut config: Config = config();
    config.spool = Some(SpoolTo {
        dir: spool.path().to_owned(),
        from_bytes,
    });
    let addr = start(config, archive.clone()).await;
    (addr, archive, spool)
}

async fn send(addr: std::net::SocketAddr, series: &RealSeries) -> pacs_dimse::Status {
    let mut client = Client::connect(addr, "MODALITY", "PACSRS", &[CT], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let file = series.slice_bytes(0);
    let response = client
        .store_as(CT, CT, &series.instance_uid(0), &data_set(&file))
        .await;
    client.release().await;
    response.status().unwrap()
}

async fn stored_syntax(archive: &Archive, series: &RealSeries) -> String {
    archive
        .index()
        .instance(
            &series.study_uid(),
            &series.series_uid(),
            &series.instance_uid(0),
        )
        .await
        .unwrap()
        .unwrap()
        .transfer_syntax_uid
}

#[tokio::test]
async fn a_large_data_set_is_stored_from_its_spool() {
    // spooled past 4 KiB; the header read from the first 16 KiB of a file of more
    let (addr, archive, spool) = spooling(4 << 10, 16 << 10).await;
    let series = RealSeries::new(1301, 1);
    assert_eq!(send(addr, &series).await, pacs_dimse::Status::SUCCESS);
    // stored as received, byte for byte the data set sent
    assert_eq!(stored_syntax(&archive, &series).await, EXPLICIT_VR_LE);
    let entry = archive
        .index()
        .instance(
            &series.study_uid(),
            &series.series_uid(),
            &series.instance_uid(0),
        )
        .await
        .unwrap()
        .unwrap();
    let stored = archive.file(&entry).await.unwrap();
    assert_eq!(data_set(&stored), data_set(&series.slice_bytes(0)));
    // and hashed as stored: the integrity check finds nothing to say
    let files = archive
        .index()
        .files_of_study(&series.study_uid())
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(archive.check_file(&files[0]).await.unwrap(), None);
    // the spool's file is gone
    assert_eq!(std::fs::read_dir(spool.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn a_spooled_file_under_the_header_bound_is_stored_as_any_other() {
    let (addr, archive, spool) = spooling(4 << 10, 64 << 20).await;
    let series = RealSeries::new(1302, 1);
    assert_eq!(send(addr, &series).await, pacs_dimse::Status::SUCCESS);
    assert_eq!(stored_syntax(&archive, &series).await, JPEG_LS);
    assert_eq!(std::fs::read_dir(spool.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn a_spooled_data_set_is_checked_against_its_request() {
    let (addr, archive, _spool) = spooling(4 << 10, 16 << 10).await;
    let series = RealSeries::new(1303, 1);
    let mut client = Client::connect(addr, "MODALITY", "PACSRS", &[CT], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let file = series.slice_bytes(0);
    // sent as another instance than it is
    let response = client
        .store_as(CT, CT, "1.2.3.4.5.6", &data_set(&file))
        .await;
    client.release().await;
    assert_eq!(
        response.status().unwrap(),
        pacs_dimse::Status::CANNOT_UNDERSTAND_INSTANCE_UID
    );
    assert!(
        archive
            .index()
            .instance(
                &series.study_uid(),
                &series.series_uid(),
                &series.instance_uid(0)
            )
            .await
            .unwrap()
            .is_none()
    );
}
