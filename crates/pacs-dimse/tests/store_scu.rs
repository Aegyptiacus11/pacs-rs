//! pacs-rs as a Storage SCU (PS3.4 Annex B; PS3.7 9.1.1, 9.3.1): instances of the archive
//! sent to another AE in the transfer syntax it takes, and the destinations a C-MOVE may
//! name.

mod common;

use std::time::Duration;

use common::storage::{Peer, storage_scp};
use common::*;
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;
use pacs_archive::{Archive, Compression};
use pacs_core::real::RealSeries;
use pacs_dimse::command::{field, tag};
use pacs_dimse::{AeTitle, Originator, RemoteAe, SendError, Sender, Status};
use pacs_index::Index;
use pacs_store::Store;

const JPEG_LS: &str = "1.2.840.10008.1.2.4.80";

/// An archive that compresses what it takes in (JPEG-LS lossless), or not.
async fn archive_with(compression: Compression) -> Archive {
    Archive::new(Store::memory(), Index::in_memory().await.unwrap()).with_compression(compression)
}

/// What the index says of `series`' instances, and their stored files.
async fn stored(
    archive: &Archive,
    series: &RealSeries,
) -> Vec<(pacs_index::InstanceEntry, bytes::Bytes)> {
    ingest(archive, series).await;
    let entries = archive
        .index()
        .instances(&series.study_uid(), &series.series_uid(), None)
        .await
        .unwrap();
    let mut files = Vec::new();
    for entry in entries {
        let file = archive.file(&entry).await.unwrap();
        files.push((entry, file));
    }
    files
}

fn pairs(files: &[(pacs_index::InstanceEntry, bytes::Bytes)]) -> Vec<(String, String)> {
    files
        .iter()
        .map(|(entry, _)| {
            (
                entry.sop_class_uid.clone(),
                entry.transfer_syntax_uid.clone(),
            )
        })
        .collect()
}

fn calling() -> AeTitle {
    AeTitle::new("PACSRS").unwrap()
}

const TIMEOUT: Duration = Duration::from_secs(5);

async fn send_all(
    sender: &mut Sender,
    files: &[(pacs_index::InstanceEntry, bytes::Bytes)],
) -> Vec<Result<Status, SendError>> {
    let mut results = Vec::new();
    for (entry, file) in files {
        results.push(
            sender
                .send(
                    file.clone(),
                    &entry.sop_class_uid,
                    &entry.sop_instance_uid,
                    &entry.transfer_syntax_uid,
                )
                .await,
        );
    }
    results
}

/// The pixels of `object`'s one frame, as 16-bit little endian values.
fn pixels(object: &InMemDicomObject) -> Vec<u16> {
    object
        .get(tags::PIXEL_DATA)
        .unwrap()
        .to_bytes()
        .unwrap()
        .chunks(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

/// A peer that takes the transfer syntax an instance is stored in gets the stored data set,
/// byte for byte, in a C-STORE-RQ of PS3.7 Table 9.3-1.
#[tokio::test]
async fn an_instance_goes_as_stored_when_the_peer_takes_its_syntax() {
    let archive = archive_with(Compression::JpegLs).await;
    let series = RealSeries::new(501, 2);
    let files = stored(&archive, &series).await;
    assert_eq!(files[0].0.transfer_syntax_uid, JPEG_LS);
    let (remote, received) =
        storage_scp(&[CT], &[EXPLICIT_VR_LE, JPEG_LS], Peer::Answers(&[])).await;
    let mut sender = Sender::open(&remote, &calling(), &pairs(&files), TIMEOUT, None)
        .await
        .unwrap();
    // one context for the stored syntax, one for the uncompressed ones
    assert_eq!(sender.accepted().len(), 2);
    for result in send_all(&mut sender, &files).await {
        assert_eq!(result.unwrap(), Status::SUCCESS);
    }
    sender.release().await;

    let received = received.lock().unwrap();
    assert_eq!(received.len(), 2);
    for (n, ((entry, file), got)) in files.iter().zip(received.iter()).enumerate() {
        assert_eq!(got.transfer_syntax, JPEG_LS);
        assert_eq!(got.data_set, data_set(file));
        let command = &got.command;
        assert_eq!(command.command_field(), Ok(field::C_STORE_RQ));
        assert_eq!(command.ui(tag::AFFECTED_SOP_CLASS_UID), Ok(CT));
        assert_eq!(
            command.ui(tag::AFFECTED_SOP_INSTANCE_UID),
            Ok(entry.sop_instance_uid.as_str())
        );
        assert_eq!(command.message_id(), Ok(n as u16 + 1));
        assert_eq!(command.us(tag::PRIORITY), Ok(0));
        assert_eq!(command.has_data_set(), Ok(true));
        // not the sub-operation of a move
        assert!(command.get(tag::MOVE_ORIGINATOR_AE_TITLE).is_none());
        assert!(command.get(tag::MOVE_ORIGINATOR_MESSAGE_ID).is_none());
    }
}

/// A peer that takes only Implicit VR Little Endian, as every AE does, gets the pixel data
/// decoded and the same values; the C-STORE-RQ of a move's sub-operation names the move.
#[tokio::test]
async fn a_compressed_instance_is_decoded_for_a_peer_that_takes_only_the_default_syntax() {
    let archive = archive_with(Compression::JpegLs).await;
    let series = RealSeries::new(502, 2).with_patient("Doe^Jane", None);
    let files = stored(&archive, &series).await;
    let (remote, received) = storage_scp(&[CT], &[IMPLICIT_VR_LE], Peer::Answers(&[])).await;
    let originator = Originator {
        ae_title: "VIEWER".into(),
        message_id: 7,
    };
    let mut sender = Sender::open(
        &remote,
        &calling(),
        &pairs(&files),
        TIMEOUT,
        Some(originator),
    )
    .await
    .unwrap();
    assert_eq!(sender.accepted().len(), 1);
    for result in send_all(&mut sender, &files).await {
        assert_eq!(result.unwrap(), Status::SUCCESS);
    }
    sender.release().await;

    let received = received.lock().unwrap();
    assert_eq!(received.len(), 2);
    for (slice, got) in received.iter().enumerate() {
        assert_eq!(got.transfer_syntax, IMPLICIT_VR_LE);
        let object = got.object();
        assert_eq!(
            value(&object, tags::SOP_INSTANCE_UID),
            Some(series.instance_uid(slice as u32))
        );
        assert_eq!(value(&object, tags::PATIENT_NAME).unwrap(), "Doe^Jane");
        assert_eq!(pixels(&object), series.pixels(slice as u32, 0));
        assert_eq!(
            got.command.text(tag::MOVE_ORIGINATOR_AE_TITLE),
            Some("VIEWER")
        );
        assert_eq!(got.command.us(tag::MOVE_ORIGINATOR_MESSAGE_ID), Ok(7));
    }
}

/// Stored uncompressed in Explicit VR, sent to a peer that takes Implicit VR only: the
/// same values under the other encoding.
#[tokio::test]
async fn an_uncompressed_instance_is_written_in_the_syntax_the_peer_takes() {
    let archive = archive_with(Compression::Off).await;
    let series = RealSeries::new(503, 1);
    let files = stored(&archive, &series).await;
    assert_eq!(files[0].0.transfer_syntax_uid, EXPLICIT_VR_LE);
    let (remote, received) = storage_scp(&[CT], &[IMPLICIT_VR_LE], Peer::Answers(&[])).await;
    let mut sender = Sender::open(&remote, &calling(), &pairs(&files), TIMEOUT, None)
        .await
        .unwrap();
    assert_eq!(
        send_all(&mut sender, &files).await.remove(0).unwrap(),
        Status::SUCCESS
    );
    sender.release().await;
    let received = received.lock().unwrap();
    assert_eq!(received[0].transfer_syntax, IMPLICIT_VR_LE);
    let object = received[0].object();
    assert_eq!(pixels(&object), series.pixels(0, 0));
    assert_eq!(
        object.get(tags::ROWS).unwrap().to_int::<u16>().unwrap(),
        128
    );
}

/// PS3.4 C.4.2.3.1: an instance whose SOP class the peer accepted no context for is not
/// sent, and the others are. A failure the peer answers with is that instance's; a
/// Warning is a stored instance.
#[tokio::test]
async fn what_the_peer_does_not_take_fails_alone() {
    let archive = archive_with(Compression::Off).await;
    let ct = RealSeries::new(504, 3);
    let mut files = stored(&archive, &ct).await;
    // an instance of a SOP class the peer does not have, between the others
    let mut mr = files[0].clone();
    mr.0.sop_class_uid = MR.into();
    files.insert(1, mr);
    // out of resources for the first it gets, a warning (coercion of data elements) for
    // the second, then success
    let (remote, received) =
        storage_scp(&[CT], &[EXPLICIT_VR_LE], Peer::Answers(&[0xA700, 0xB000])).await;
    let mut sender = Sender::open(&remote, &calling(), &pairs(&files), TIMEOUT, None)
        .await
        .unwrap();
    let results = send_all(&mut sender, &files).await;
    sender.release().await;
    match &results[0] {
        Err(e @ SendError::Refused { status, .. }) => {
            assert_eq!(*status, Status(0xA700));
            assert!(e.association_holds());
        }
        other => panic!("{other:?}"),
    }
    match &results[1] {
        Err(e @ SendError::NoContext(sop_class)) => {
            assert_eq!(sop_class, MR);
            assert!(e.association_holds());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(*results[2].as_ref().unwrap(), Status(0xB000));
    assert_eq!(*results[3].as_ref().unwrap(), Status::SUCCESS);
    // the MR never left
    assert_eq!(received.lock().unwrap().len(), 3);
}

/// A peer that accepts nothing of what is proposed, one that is not there, and one that
/// aborts in the middle: each is an error that says so, not a hang.
#[tokio::test]
async fn a_peer_that_cannot_be_sent_to_is_an_error() {
    let archive = archive_with(Compression::Off).await;
    let files = stored(&archive, &RealSeries::new(505, 2)).await;

    let (takes_mr_only, _) = storage_scp(&[MR], &[EXPLICIT_VR_LE], Peer::Answers(&[])).await;
    // an association with no presentation context accepted is not one (dicom-ul ends it)
    let error = Sender::open(&takes_mr_only, &calling(), &pairs(&files), TIMEOUT, None)
        .await
        .err()
        .expect("no context was accepted");
    assert!(error.to_string().contains("STORESCP"), "{error}");

    let nobody: RemoteAe = "gone=NOBODY@127.0.0.1:1".parse().unwrap();
    let error = Sender::open(&nobody, &calling(), &pairs(&files), TIMEOUT, None)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("NOBODY@127.0.0.1:1"), "{error}");

    let (aborting, _) = storage_scp(&[CT], &[EXPLICIT_VR_LE], Peer::Aborts).await;
    let mut sender = Sender::open(&aborting, &calling(), &pairs(&files), TIMEOUT, None)
        .await
        .unwrap();
    let results = send_all(&mut sender, &files[..1]).await;
    let error = results[0].as_ref().unwrap_err();
    assert!(!error.association_holds(), "{error}");
    sender.abort().await;
}

/// From one pacs-rs to another: a series stored compressed arrives whole, and what the
/// second serves has the pixels the first was given.
#[tokio::test]
async fn a_series_sent_to_another_pacs_rs_arrives_whole() {
    let here = archive_with(Compression::JpegLs).await;
    let series = RealSeries::new(506, 3);
    let files = stored(&here, &series).await;
    let there = archive().await;
    let addr = start(config(), there.clone()).await;
    let remote: RemoteAe = format!("there=PACSRS@127.0.0.1:{}", addr.port())
        .parse()
        .unwrap();
    let mut sender = Sender::open(
        &remote,
        &AeTitle::new("SENDER").unwrap(),
        &pairs(&files),
        TIMEOUT,
        None,
    )
    .await
    .unwrap();
    // pacs-rs takes the stored syntax
    assert!(
        sender
            .accepted()
            .iter()
            .any(|context| context.transfer_syntax == JPEG_LS && context.abstract_syntax == CT)
    );
    for result in send_all(&mut sender, &files).await {
        assert_eq!(result.unwrap(), Status::SUCCESS);
    }
    sender.release().await;
    let arrived = there
        .index()
        .instances(&series.study_uid(), &series.series_uid(), None)
        .await
        .unwrap();
    assert_eq!(arrived.len(), 3);
    for (slice, entry) in arrived.iter().enumerate() {
        assert_eq!(entry.transfer_syntax_uid, JPEG_LS);
        let file = there.file(entry).await.unwrap();
        let instance = pacs_core::Instance::from_bytes(&file).unwrap();
        let shape = instance.shape().unwrap();
        let native = instance.native_frame(1, shape).unwrap();
        let got: Vec<u16> = native
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(got, series.pixels(slice as u32, 0));
    }
}

/// The destinations a C-MOVE may name are the remote AEs listed as such, found by their AE
/// title; any other AE title, a remote AE that is not listed included, is unknown.
#[test]
fn a_move_destination_is_a_remote_ae_that_is_listed_as_one() {
    let mut config = config();
    config.remotes = vec![
        "hospital=HOSP_PACS@10.0.0.5:104".parse().unwrap(),
        "viewer=VIEWER@10.0.0.9:11112".parse().unwrap(),
        "ws=WS1@10.0.0.7:104/tls".parse().unwrap(),
    ];
    // nothing is a destination until it is named
    for title in ["HOSP_PACS", "VIEWER", "WS1", "PACSRS"] {
        assert!(config.move_destination(title).is_none(), "{title}");
    }
    config.move_destinations = vec!["viewer".into(), "ws".into()];
    assert_eq!(config.move_destination("VIEWER").unwrap().id, "viewer");
    // as it comes in a command: padded
    assert_eq!(config.move_destination("WS1 ").unwrap().port, 104);
    // the archive this server only reads from, a title nobody has, and an id
    for title in ["HOSP_PACS", "NOBODY", "viewer", ""] {
        assert!(config.move_destination(title).is_none(), "{title}");
    }
}
