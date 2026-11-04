//! What uploads cost in memory, counted at the allocator: requested bytes, a stricter measure
//! than resident memory. Alone in its own test binary, so nothing else moves the count.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use dicom_dictionary_std::uids;
use futures::StreamExt;
use http_body_util::BodyExt;
use pacs_archive::Archive;
use pacs_core::synthetic::{
    ITEM, RawFile, SEQUENCE_END, UNDEFINED, un_sequence_file, un_study_uid_file,
};
use pacs_index::Index;
use pacs_store::Store;
use tower::ServiceExt;

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn added(size: usize) {
    let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

// SAFETY: passes every call to `System` unchanged; the counters do not allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        added(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        added(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        added(new_size);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const FILES: u32 = 16;
const CONCURRENT: usize = 6;
/// Far above what small files and a few requests need, far below one wrong length.
const LIMIT: usize = 32 << 20;

/// The peak above the start while all of `files` are uploaded, `CONCURRENT` at a time; the statuses.
async fn peak_of(app: &Router, files: Vec<Vec<u8>>) -> (usize, Vec<StatusCode>) {
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let statuses = futures::stream::iter(files)
        .map(|file| {
            let app = app.clone();
            tokio::spawn(async move {
                let request = Request::post("/instances").body(Body::from(file)).unwrap();
                let response = app.oneshot(request).await.unwrap();
                let status = response.status();
                response.into_body().collect().await.unwrap();
                status
            })
        })
        .buffer_unordered(CONCURRENT)
        .map(|status| status.unwrap())
        .collect()
        .await;
    (PEAK.load(Ordering::Relaxed) - before, statuses)
}

/// A length of 3 GiB declared in a small file, in the VRs dicom-rs reads differently: bytes (UN,
/// OW), text (UT), a VR it does not know.
fn lying_file(n: u32) -> Vec<u8> {
    let base = || RawFile::new(uids::EXPLICIT_VR_LITTLE_ENDIAN).identifiers(70_000 + n, b"UI");
    let lie = 0xC000_0000;
    match n % 4 {
        0 => base()
            .long_declared((0x0043, 0x1001), b"UN", lie, b"short")
            .bytes(),
        1 => base()
            .long_declared((0x0043, 0x1001), b"UT", lie, b"short")
            .bytes(),
        2 => base()
            .long_declared((0x0043, 0x1001), b"OW", lie, b"short")
            .bytes(),
        _ => base()
            .long_declared((0x0043, 0x1001), b"XX", lie, b"short")
            .bytes(),
    }
}

/// A file with nothing but `content`, and what reading it and its metadata JSON requests at the
/// peak, against the estimate: the constants in `pacs_core::wire` must stay near what is measured.
fn estimate_holds(name: &str, file: Vec<u8>) {
    let estimate = pacs_core::wire::read_cost(&file).unwrap() as usize + file.len();
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let instance = pacs_core::Instance::from_bytes(&file).unwrap();
    let json = instance.metadata_json().unwrap();
    let peak = PEAK.load(Ordering::Relaxed) - before;
    drop(json);
    // Vectors double as they grow, so a peak can be over the estimate, but not by much.
    assert!(
        peak < estimate + estimate / 2 && peak > estimate / 2,
        "{name}: estimated {estimate} bytes, the peak was {peak}"
    );
}

fn uploads_of_values(seed: u32, values: usize) -> Vec<u8> {
    RawFile::new(uids::EXPLICIT_VR_LITTLE_ENDIAN)
        .identifiers(seed, b"UI")
        .long((0x0043, 0x1001), b"UC", &vec![b'\\'; values])
        .bytes()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn uploads_cost_little_however_their_lengths_lie() {
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap());
    let app = pacs_rest::router(archive, 1 << 30);

    let (peak, statuses) = peak_of(&app, (0..FILES).map(un_sequence_file).collect()).await;
    assert!(peak < LIMIT, "sequences in UN took {peak} bytes");
    assert_eq!(statuses, vec![StatusCode::OK; FILES as usize]);

    let (peak, statuses) = peak_of(
        &app,
        (0..FILES).map(|n| un_study_uid_file(n + 1000)).collect(),
    )
    .await;
    assert!(peak < LIMIT, "UIDs in UN took {peak} bytes");
    assert_eq!(statuses, vec![StatusCode::OK; FILES as usize]);

    let (peak, statuses) = peak_of(&app, (0..FILES).map(lying_file).collect()).await;
    assert!(peak < LIMIT, "lying lengths took {peak} bytes");
    assert_eq!(statuses, vec![StatusCode::BAD_REQUEST; FILES as usize]);

    // Well-formed files costly to read. The estimate first: of values, elements, items.
    estimate_holds("values", uploads_of_values(1, 1 << 20));
    let mut elements = Vec::new();
    for n in 0..(1u32 << 17) {
        elements.extend((0x0101 + 2 * (n >> 16) as u16).to_le_bytes());
        elements.extend((n as u16).to_le_bytes());
        elements.extend(b"LO\0\0");
    }
    estimate_holds(
        "elements",
        RawFile::new(uids::EXPLICIT_VR_LITTLE_ENDIAN)
            .identifiers(2, b"UI")
            .raw(&elements)
            .bytes(),
    );
    let mut items = Vec::new();
    for _ in 0..(1u32 << 19) {
        items.extend(ITEM);
        items.extend(0u32.to_le_bytes());
    }
    items.extend(SEQUENCE_END);
    estimate_holds(
        "items",
        RawFile::new(uids::EXPLICIT_VR_LITTLE_ENDIAN)
            .identifiers(3, b"UI")
            .long_declared((0x0008, 0x1115), b"SQ", UNDEFINED, &items)
            .bytes(),
    );

    // A limit of 64 MiB. Over it, a file costs nothing but its bytes and the walk: 8 MB that
    // would take 1344 MB.
    let limit = 64 << 20;
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap())
        .with_read_limit(limit as u64);
    let app = pacs_rest::router(archive, 1 << 30);
    let (peak, statuses) = peak_of(
        &app,
        (0..FILES)
            .map(|n| uploads_of_values(80_000 + n, 8 << 20))
            .collect(),
    )
    .await;
    assert!(peak < 200 << 20, "refused files took {peak} bytes");
    assert_eq!(
        statuses,
        vec![StatusCode::PAYLOAD_TOO_LARGE; FILES as usize]
    );

    // Under it, each estimated at most at half the limit: six at once would be three limits, but
    // they take turns. Their bodies come on top, 1 MiB each at most.
    let values = limit / 2 / 170 - 2000;
    let (peak, statuses) = peak_of(
        &app,
        (0..FILES)
            .map(|n| uploads_of_values(90_000 + n, values))
            .collect(),
    )
    .await;
    assert_eq!(statuses, vec![StatusCode::OK; FILES as usize]);
    assert!(
        peak < 2 * limit,
        "files under the limit took {peak} bytes at once"
    );
}
