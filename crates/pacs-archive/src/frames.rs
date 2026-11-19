//! Frames read from the store on their own: the start of the file up to its pixel data,
//! then the bytes of the frames asked for, instead of the whole file. What one frame of a
//! file of thousands costs no longer grows with the file.
//!
//! It is a shorter way to the same bytes, never another answer: a file whose frames cannot
//! be found this way, a file that is not what its layout said, and a store that fails
//! along the way all end in `None`, and the caller reads the whole file, which finds the
//! frame or reports why not.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use bytes::Bytes;
use pacs_core::Instance;
use pacs_core::frames::{Frame, FrameLayout, FramePlan};
use pacs_core::wire::{PixelDataSearch, pixel_data_at};
use pacs_index::InstanceEntry;

use crate::Archive;

/// Files smaller than this are read whole for a frame, unless set with
/// [`Archive::with_frames_alone_from`]: for a file of a few frames, one read of all of it
/// is quicker than the two or more that finding a frame takes.
pub const DEFAULT_FRAMES_ALONE_FROM: u64 = 1 << 20;

/// How much of a file is read first to find where its pixel data starts, and how much at
/// the most: the data set before the pixel data of a file with thousands of frames can
/// take megabytes.
const FIRST_READ: u64 = 64 << 10;
const LAST_READ: u64 = 32 << 20;

/// How much memory the layouts kept may take ([`FrameLayout::weight`]).
const LAYOUTS_WEIGHT: usize = 32 << 20;

/// The layouts of the files frames were last asked of, the oldest dropped first. A file
/// without one is kept too, so that it is not searched at each request.
#[derive(Debug, Default)]
pub(crate) struct Layouts {
    by_key: HashMap<String, (Stamp, Option<Arc<FrameLayout>>)>,
    order: VecDeque<String>,
    weight: usize,
}

/// What changes when the file under a key does: when it was received, and its size.
type Stamp = (i64, i64);

fn weight(key: &str, layout: &Option<Arc<FrameLayout>>) -> usize {
    2 * key.len() + 64 + layout.as_ref().map_or(0, |layout| layout.weight())
}

impl Layouts {
    fn get(&self, key: &str, stamp: Stamp) -> Option<Option<Arc<FrameLayout>>> {
        let (known, layout) = self.by_key.get(key)?;
        (*known == stamp).then(|| layout.clone())
    }

    fn forget(&mut self, key: &str) {
        if let Some((_, layout)) = self.by_key.remove(key) {
            self.weight -= weight(key, &layout);
            self.order.retain(|kept| kept != key);
        }
    }

    fn put(&mut self, key: &str, stamp: Stamp, layout: Option<Arc<FrameLayout>>) {
        self.forget(key);
        let added = weight(key, &layout);
        if added > LAYOUTS_WEIGHT {
            return;
        }
        self.weight += added;
        self.by_key.insert(key.to_owned(), (stamp, layout));
        self.order.push_back(key.to_owned());
        while self.weight > LAYOUTS_WEIGHT {
            let oldest = self
                .order
                .front()
                .cloned()
                .expect("weight is of what is kept");
            self.forget(&oldest);
        }
    }
}

impl Archive {
    /// Read frames on their own from files of at least this many bytes; smaller files are
    /// read whole. [`DEFAULT_FRAMES_ALONE_FROM`] unless set.
    pub fn with_frames_alone_from(mut self, bytes: u64) -> Self {
        self.frames_alone_from = bytes;
        self
    }

    /// Where the frames of an instance's file are, for [`Archive::frames_alone`]. `None`
    /// when its frames are to be cut from the whole file ([`Archive::file`]).
    pub async fn frame_layout(&self, entry: &InstanceEntry) -> Option<Arc<FrameLayout>> {
        let size = u64::try_from(entry.size_bytes).ok()?;
        if size < self.frames_alone_from {
            return None;
        }
        let stamp = (entry.received_ms, entry.size_bytes);
        let known = self
            .layouts
            .lock()
            .expect("no holder of the layouts panics")
            .get(&entry.object_key, stamp);
        if let Some(layout) = known {
            return layout;
        }
        let layout = match self.find_layout(&entry.object_key, size).await {
            Ok(layout) => layout.map(Arc::new),
            // Not kept: the store may answer the next time. Reading the whole file reports it.
            Err(e) => {
                tracing::debug!(key = entry.object_key, error = %e, "no frame layout: the store failed");
                return None;
            }
        };
        self.layouts
            .lock()
            .expect("no holder of the layouts panics")
            .put(&entry.object_key, stamp, layout.clone());
        layout
    }

    async fn find_layout(
        &self,
        key: &str,
        size: u64,
    ) -> Result<Option<FrameLayout>, pacs_store::Error> {
        let mut want = FIRST_READ.min(size);
        let (start, at) = loop {
            let start = self.store.get_range(key, 0..want).await?;
            match pixel_data_at(&start) {
                PixelDataSearch::Found(at) => break (start, at),
                PixelDataSearch::NotIn if want < size && want < LAST_READ => {
                    want = (want * 8).min(size);
                }
                PixelDataSearch::NotIn | PixelDataSearch::Unreadable => return Ok(None),
            }
        };
        // The file cut before its pixel data reads as an instance, in the room its size asks.
        let header = start.slice(..at.element);
        let reading = self.admit_stored(&header).await;
        let planned = tokio::task::spawn_blocking(move || {
            let _reading = reading;
            let header = Instance::from_bytes(&header).ok()?;
            FrameLayout::plan(&header, at, size)
        })
        .await;
        let mut plan = match planned {
            Ok(Some(plan)) => plan,
            Ok(None) => return Ok(None),
            // Reading the whole file meets what stopped this, and reports it.
            Err(e) => {
                tracing::warn!(key, error = %e, "no frame layout: reading the file's start failed");
                return Ok(None);
            }
        };
        loop {
            let search = match plan {
                FramePlan::Ready(layout) => return Ok(Some(layout)),
                FramePlan::Read(search) => search,
            };
            let wanted = search.wants();
            let bytes = if wanted.end <= start.len() as u64 {
                start.slice(wanted.start as usize..wanted.end as usize)
            } else {
                self.store.get_range(key, wanted).await?
            };
            match search.give(&bytes) {
                Some(next) => plan = next,
                None => return Ok(None),
            }
        }
    }

    /// Frames `numbers` (from 1) of an instance, by its [`Archive::frame_layout`], reading
    /// of its file only where they are. `None` when what is read is not those frames (the
    /// file's offset table is wrong, a number is past the last frame, the store failed):
    /// the whole file is to be read then.
    pub async fn frames_alone(
        &self,
        entry: &InstanceEntry,
        layout: &FrameLayout,
        numbers: &[u32],
    ) -> Option<Vec<Frame>> {
        let ranges = numbers
            .iter()
            .map(|number| layout.range(*number).ok())
            .collect::<Option<Vec<_>>>()?;
        let read: Vec<Bytes> = match self.store.get_ranges(&entry.object_key, &ranges).await {
            Ok(read) => read,
            Err(e) => {
                tracing::debug!(key = entry.object_key, error = %e, "frames not read alone: the store failed");
                return None;
            }
        };
        numbers
            .iter()
            .zip(&read)
            .map(|(number, bytes)| layout.frame(*number, bytes))
            .collect()
    }
}
