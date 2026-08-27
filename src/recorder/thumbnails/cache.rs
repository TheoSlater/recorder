use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Arc,
};

use gpui::RenderImage;

use super::layout::ThumbnailSize;

pub(crate) const MAX_CACHE_ENTRIES: usize = 128;
pub(crate) const MAX_CACHE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ThumbnailKey {
    source: PathBuf,
    timestamp_bucket: u64,
    interval_us: u64,
    size: ThumbnailSize,
}

impl ThumbnailKey {
    pub(super) fn new(
        source: PathBuf,
        timestamp_bucket: u64,
        interval_us: u64,
        size: ThumbnailSize,
    ) -> Self {
        Self {
            source,
            timestamp_bucket,
            interval_us,
            size,
        }
    }
}

struct Entry {
    image: Arc<RenderImage>,
    bytes: u64,
}

#[derive(Default)]
pub(crate) struct CacheInsert {
    pub(crate) evicted: Vec<Arc<RenderImage>>,
    pub(crate) inserted: bool,
}

pub(crate) struct ThumbnailCache {
    entries: HashMap<ThumbnailKey, Entry>,
    order: VecDeque<ThumbnailKey>,
    bytes: u64,
    max_entries: usize,
    max_bytes: u64,
}

impl Default for ThumbnailCache {
    fn default() -> Self {
        Self::new(MAX_CACHE_ENTRIES, MAX_CACHE_BYTES)
    }
}

impl ThumbnailCache {
    pub(crate) fn new(max_entries: usize, max_bytes: u64) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            max_entries: max_entries.max(1),
            max_bytes,
        }
    }

    pub(crate) fn get(&mut self, key: &ThumbnailKey) -> Option<Arc<RenderImage>> {
        let image = self.entries.get(key).map(|entry| entry.image.clone());
        if image.is_some() {
            self.touch(key);
        }
        image
    }

    pub(crate) fn insert(
        &mut self,
        key: ThumbnailKey,
        image: Arc<RenderImage>,
        size: ThumbnailSize,
    ) -> CacheInsert {
        let bytes = size.memory_bytes();
        if bytes > self.max_bytes || self.max_bytes == 0 {
            return CacheInsert::default();
        }

        let mut evicted = Vec::new();
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
            self.order.retain(|entry| entry != &key);
            evicted.push(previous.image);
        }
        self.entries.insert(key.clone(), Entry { image, bytes });
        self.order.push_back(key);
        self.bytes = self.bytes.saturating_add(bytes);

        while self.entries.len() > self.max_entries || self.bytes > self.max_bytes {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            let Some(entry) = self.entries.remove(&oldest) else {
                continue;
            };
            self.bytes = self.bytes.saturating_sub(entry.bytes);
            evicted.push(entry.image);
        }

        CacheInsert {
            evicted,
            inserted: true,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    fn touch(&mut self, key: &ThumbnailKey) {
        self.order.retain(|entry| entry != key);
        self.order.push_back(key.clone());
    }
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
