//! Frames read from the store on their own are the frames cut from the whole file, and
//! what cannot be read that way is left to the whole file.

use bytes::Bytes;
use pacs_archive::{Archive, Compression};
use pacs_core::Instance;
use pacs_core::codec::JPEG_LS_LOSSLESS;
use pacs_core::real::RealSeries;
use pacs_index::{Index, InstanceEntry};
use pacs_store::Store;

/// One that reads frames alone from a file of any size, and stores files as received.
async fn archive_on(store: Store) -> Archive {
    Archive::new(store, Index::in_memory().await.unwrap())
        .with_compression(Compression::Off)
        .with_frames_alone_from(0)
}

async fn archive() -> Archive {
    archive_on(Store::memory()).await
}

async fn stored(archive: &Archive, series: &RealSeries, file: Vec<u8>) -> InstanceEntry {
    archive.ingest(Bytes::from(file)).await.unwrap();
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
}

/// Every frame alone, and several at once in an order of their own, are those of the file.
async fn assert_frames_alone(archive: &Archive, entry: &InstanceEntry) {
    let whole = Instance::from_bytes(&archive.file(entry).await.unwrap()).unwrap();
    let layout = archive.frame_layout(entry).await.expect("a layout");
    assert_eq!(layout.frames(), whole.number_of_frames());
    for number in 1..=layout.frames() {
        let alone = archive.frames_alone(entry, &layout, &[number]).await;
        assert_eq!(alone, Some(vec![whole.frame(number).unwrap()]));
    }
    let last = layout.frames();
    let alone = archive.frames_alone(entry, &layout, &[last, 1, 1]).await;
    assert_eq!(
        alone,
        Some(vec![
            whole.frame(last).unwrap(),
            whole.frame(1).unwrap(),
            whole.frame(1).unwrap()
        ])
    );
    assert_eq!(
        archive.frames_alone(entry, &layout, &[]).await,
        Some(vec![])
    );
    // a frame that is not there is the whole file's to refuse
    assert_eq!(
        archive.frames_alone(entry, &layout, &[last + 1]).await,
        None
    );
    assert_eq!(archive.frames_alone(entry, &layout, &[1, 0]).await, None);
}

/// Two frames in three fragments, and where the second frame starts.
fn two_frames_in_three() -> (Vec<Vec<u8>>, u32) {
    let mut frames = RealSeries::jpeg_ls_frames();
    let (second, first) = (frames.remove(1), frames.remove(0));
    let (head, tail) = first.split_at(1000);
    let offset = (8 + head.len() + 8 + tail.len()) as u32;
    (vec![head.to_vec(), tail.to_vec(), second], offset)
}

#[tokio::test]
async fn frames_alone_are_the_frames_of_the_file() {
    let archive = archive().await;
    // native
    let series = RealSeries::new(71, 1);
    let entry = stored(&archive, &series, series.multiframe_bytes(0)).await;
    assert_frames_alone(&archive, &entry).await;
    // a fragment a frame, no offset table
    let series = RealSeries::new(72, 1);
    let entry = stored(&archive, &series, series.jpeg_ls_multiframe_bytes(0)).await;
    assert_frames_alone(&archive, &entry).await;
    // by the offset table
    let series = RealSeries::new(73, 1);
    let (fragments, offset) = two_frames_in_three();
    let file = series.refragmented(0, 2, vec![0, offset], fragments);
    let entry = stored(&archive, &series, file).await;
    assert_frames_alone(&archive, &entry).await;
}

#[tokio::test]
async fn frames_alone_of_what_the_archive_compressed() {
    // native pixel data stored as JPEG-LS, as the archive does unless told otherwise
    let archive =
        Archive::new(Store::memory(), Index::in_memory().await.unwrap()).with_frames_alone_from(0);
    let series = RealSeries::new(74, 1);
    let entry = stored(&archive, &series, series.multiframe_bytes(0)).await;
    assert_eq!(entry.transfer_syntax_uid, JPEG_LS_LOSSLESS);
    assert_frames_alone(&archive, &entry).await;
}

#[tokio::test]
async fn only_the_frame_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let archive = archive_on(Store::local(dir.path(), "dicom").unwrap()).await;
    let series = RealSeries::new(75, 1);
    let file = series.multiframe_bytes(0);
    let entry = stored(&archive, &series, file.clone()).await;
    let whole = Instance::from_bytes(&file).unwrap();
    let (third, fourth) = (whole.frame(3).unwrap(), whole.frame(4).unwrap());

    // the file with every other frame gone: frames 3 and 4 kept, zeros before and after
    let layout = archive.frame_layout(&entry).await.expect("a layout");
    let kept = layout.range(3).unwrap().start as usize..layout.range(4).unwrap().end as usize;
    let first = layout.range(1).unwrap().start as usize;
    let mut holed = file.clone();
    holed[first..kept.start].fill(0);
    holed[kept.end..].fill(0);
    assert_ne!(holed, file);
    std::fs::write(dir.path().join(&entry.object_key), &holed).unwrap();

    let alone = archive.frames_alone(&entry, &layout, &[4, 3]).await;
    assert_eq!(alone, Some(vec![fourth, third.clone()]));
    // and an archive that never saw the file whole finds them the same way
    let fresh = Archive::new(
        Store::local(dir.path(), "dicom").unwrap(),
        archive.index().clone(),
    )
    .with_frames_alone_from(0);
    let layout = fresh.frame_layout(&entry).await.expect("a layout");
    assert_eq!(
        fresh.frames_alone(&entry, &layout, &[3]).await,
        Some(vec![third])
    );
}

#[tokio::test]
async fn files_to_read_whole_have_no_layout() {
    let series = RealSeries::new(76, 1);
    let file = series.multiframe_bytes(0);
    // smaller than frames are read alone from, as every file here is by default
    let small = Archive::new(Store::memory(), Index::in_memory().await.unwrap())
        .with_compression(Compression::Off);
    let entry = stored(&small, &series, file.clone()).await;
    assert!(small.frame_layout(&entry).await.is_none());
    let at_its_size = small.clone().with_frames_alone_from(file.len() as u64);
    assert!(at_its_size.frame_layout(&entry).await.is_some());
    let over_its_size = small.with_frames_alone_from(file.len() as u64 + 1);
    assert!(over_its_size.frame_layout(&entry).await.is_none());

    let archive = archive().await;
    // one frame, asked twice: the second answer is the one remembered
    let series = RealSeries::new(77, 1);
    let entry = stored(&archive, &series, series.slice_bytes(0)).await;
    assert!(archive.frame_layout(&entry).await.is_none());
    assert!(archive.frame_layout(&entry).await.is_none());
    // fragments that are not one a frame, without a table
    let series = RealSeries::new(78, 1);
    let (fragments, offset) = two_frames_in_three();
    let file = series.refragmented(0, 2, vec![], fragments.clone());
    let entry = stored(&archive, &series, file).await;
    assert!(archive.frame_layout(&entry).await.is_none());
    // a table that points inside a fragment: a layout, and no frame by it
    let series = RealSeries::new(79, 1);
    let file = series.refragmented(0, 2, vec![0, offset - 4], fragments);
    let entry = stored(&archive, &series, file).await;
    let layout = archive.frame_layout(&entry).await.expect("a layout");
    assert_eq!(archive.frames_alone(&entry, &layout, &[1]).await, None);
    assert_eq!(archive.frames_alone(&entry, &layout, &[2]).await, None);
}

#[tokio::test]
async fn a_store_that_fails_leaves_it_to_the_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    let archive = archive_on(Store::local(dir.path(), "dicom").unwrap()).await;
    let series = RealSeries::new(80, 1);
    let file = series.multiframe_bytes(0);
    let entry = stored(&archive, &series, file.clone()).await;
    let path = dir.path().join(&entry.object_key);

    // the file is gone: no layout, and that is not remembered once it is back
    std::fs::remove_file(&path).unwrap();
    assert!(archive.frame_layout(&entry).await.is_none());
    assert!(
        archive.file(&entry).await.is_err(),
        "the whole file reports it"
    );
    std::fs::write(&path, &file).unwrap();
    let layout = archive.frame_layout(&entry).await.expect("a layout");

    // gone, or cut short, after its layout was found
    std::fs::remove_file(&path).unwrap();
    assert_eq!(archive.frames_alone(&entry, &layout, &[1]).await, None);
    std::fs::write(&path, &file[..file.len() - 10]).unwrap();
    let last = layout.frames();
    assert_eq!(archive.frames_alone(&entry, &layout, &[last]).await, None);
    assert!(archive.frames_alone(&entry, &layout, &[1]).await.is_some());
}

#[tokio::test]
async fn a_replaced_file_gets_its_own_layout() {
    let archive = archive().await.with_overwrite(true);
    let series = RealSeries::new(81, 1);
    let entry = stored(&archive, &series, series.jpeg_ls_multiframe_bytes(0)).await;
    let layout = archive.frame_layout(&entry).await.expect("a layout");
    assert_eq!(layout.frames(), RealSeries::FRAMES);

    // the same instance sent again, as two frames
    let (fragments, offset) = two_frames_in_three();
    let file = series.refragmented(0, 2, vec![0, offset], fragments);
    let replaced = stored(&archive, &series, file).await;
    // a file of its own: the layout found for the first cannot be taken for it
    assert_ne!(replaced.object_key, entry.object_key);
    let layout = archive.frame_layout(&replaced).await.expect("a layout");
    assert_eq!(layout.frames(), 2);
    assert_frames_alone(&archive, &replaced).await;
}
