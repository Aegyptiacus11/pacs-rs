//! Times the first request for one frame of a real multi-frame file, as stored three ways:
//! compressed with a Basic Offset Table, compressed without one, and uncompressed; each read
//! on its own and from the whole file.
//!
//! `cargo run --release -p pacs-archive --example first_frame -- <file.dcm> [s3 endpoint]`

use std::time::{Duration, Instant};

use bytes::Bytes;
use pacs_archive::{Archive, Compression};
use pacs_core::Instance;
use pacs_core::codec::JPEG_LS_LOSSLESS;
use pacs_core::frames::Frame;
use pacs_core::wire::{PixelDataSearch, pixel_data_at};
use pacs_index::{Index, InstanceEntry};
use pacs_store::{S3Config, Store};

const RUNS: usize = 15;

/// The file with its Basic Offset Table emptied: what compression wrote before the table.
fn without_table(file: &[u8]) -> Vec<u8> {
    let PixelDataSearch::Found(at) = pixel_data_at(file) else {
        panic!("no pixel data");
    };
    assert_eq!(at.len, None, "encapsulated pixel data");
    let len = u32::from_le_bytes(file[at.value + 4..at.value + 8].try_into().unwrap()) as usize;
    assert!(len > 0, "a table to empty");
    let mut out = file[..at.value + 4].to_vec();
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&file[at.value + 8 + len..]);
    out
}

fn median(mut times: Vec<Duration>) -> f64 {
    times.sort();
    times[times.len() / 2].as_secs_f64() * 1e3
}

async fn entry_of(archive: &Archive, key: &pacs_core::InstanceKey) -> InstanceEntry {
    archive
        .index()
        .instance(
            &key.study_instance_uid,
            &key.series_instance_uid,
            &key.sop_instance_uid,
        )
        .await
        .unwrap()
        .unwrap()
}

async fn whole(archive: &Archive, entry: &InstanceEntry, number: u32) -> Frame {
    let file = archive.file(entry).await.unwrap();
    Instance::from_bytes(&file).unwrap().frame(number).unwrap()
}

async fn measure(name: &str, store: impl Fn(&str) -> Store, files: &[(&str, &[u8])]) {
    println!("\n{name}: median of {RUNS} runs, ms");
    println!(
        "{:<28} {:>10} {:>12} {:>12} {:>12}",
        "stored as", "MB", "whole file", "alone, 1st", "alone, next"
    );
    for (label, file) in files {
        let index = Index::in_memory().await.unwrap();
        let store = store(label);
        let archive = |store: &Store| {
            Archive::new(store.clone(), index.clone())
                .with_compression(Compression::Off)
                .with_frames_alone_from(0)
        };
        let first = archive(&store);
        let key = first
            .ingest(Bytes::copy_from_slice(file))
            .await
            .unwrap()
            .key;
        let entry = entry_of(&first, &key).await;
        let frames = Instance::from_bytes(file).unwrap().number_of_frames();
        let number = frames / 2;
        let expected = whole(&first, &entry, number).await;

        let (mut by_whole, mut cold, mut warm) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..RUNS {
            let started = Instant::now();
            let frame = whole(&first, &entry, number).await;
            by_whole.push(started.elapsed());
            assert_eq!(frame, expected);

            // an archive that has not seen the file: no layout kept
            let fresh = archive(&store);
            let started = Instant::now();
            let layout = fresh.frame_layout(&entry).await.expect("a layout");
            let alone = fresh.frames_alone(&entry, &layout, &[number]).await;
            cold.push(started.elapsed());
            assert_eq!(alone, Some(vec![expected.clone()]));

            let started = Instant::now();
            let layout = fresh.frame_layout(&entry).await.expect("a layout");
            let alone = fresh.frames_alone(&entry, &layout, &[number + 1]).await;
            warm.push(started.elapsed());
            assert!(alone.is_some());
        }
        println!(
            "{:<28} {:>10.1} {:>12.2} {:>12.2} {:>12.2}",
            label,
            file.len() as f64 / 1e6,
            median(by_whole),
            median(cold),
            median(warm)
        );
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a DICOM file");
    let endpoint = args.next();
    let native = std::fs::read(&path).unwrap();
    let instance = Instance::from_bytes(&native).unwrap();
    let shape = instance.shape().unwrap();
    println!(
        "{path}\n{} frames of {} x {}, {} bits, transfer syntax {}, {:.1} MB",
        instance.number_of_frames(),
        shape.columns,
        shape.rows,
        shape.bits_allocated,
        instance.key.transfer_syntax_uid,
        native.len() as f64 / 1e6
    );
    let started = Instant::now();
    let tabled = instance.transcoded(JPEG_LS_LOSSLESS, true).unwrap();
    println!(
        "compressed to JPEG-LS and checked in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    let untabled = without_table(&tabled);
    println!(
        "the offset table: {} bytes of {}",
        tabled.len() - untabled.len(),
        tabled.len()
    );
    let files: [(&str, &[u8]); 3] = [
        ("JPEG-LS, offset table", &tabled),
        ("JPEG-LS, no offset table", &untabled),
        ("uncompressed", &native),
    ];

    let dir = tempfile::tempdir().unwrap();
    measure(
        "local disk",
        |label| {
            let dir = dir.path().join(label.replace([' ', ','], "-"));
            std::fs::create_dir_all(&dir).unwrap();
            Store::local(dir, "").unwrap()
        },
        &files,
    )
    .await;

    if let Some(endpoint) = endpoint {
        let config = S3Config {
            endpoint: Some(endpoint.clone()),
            region: "us-east-1".into(),
            bucket: "pacs-test".into(),
            access_key_id: "test".into(),
            secret_access_key: "test".into(),
        };
        measure(
            &format!("S3 at {endpoint}"),
            |label| Store::s3(&config, &label.replace([' ', ','], "-")).unwrap(),
            &files,
        )
        .await;
    }
}
