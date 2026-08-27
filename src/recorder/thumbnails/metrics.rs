use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use super::cache::ThumbnailCache;

#[derive(Default)]
pub(crate) struct ThumbnailMetrics {
    pub(crate) requested: AtomicU64,
    pub(crate) completed: AtomicU64,
    pub(crate) failed: AtomicU64,
    pub(crate) stale: AtomicU64,
    pub(crate) dropped: AtomicU64,
    pub(crate) cache_hits: AtomicU64,
    pub(crate) cache_misses: AtomicU64,
    pub(crate) evicted: AtomicU64,
    pub(crate) decode_nanos: AtomicU64,
    pub(crate) resize_nanos: AtomicU64,
}

impl ThumbnailMetrics {
    pub(crate) fn snapshot(&self, cache_entries: usize, cache_bytes: u64) -> Snapshot {
        Snapshot {
            requested: self.requested.load(Ordering::Relaxed),
            completed: self.completed.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            stale: self.stale.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            evicted: self.evicted.load(Ordering::Relaxed),
            decode_ms: nanos_to_millis(self.decode_nanos.load(Ordering::Relaxed)),
            resize_ms: nanos_to_millis(self.resize_nanos.load(Ordering::Relaxed)),
            cache_entries,
            cache_bytes,
        }
    }
}

pub(super) fn report(source: &Path, metrics: &ThumbnailMetrics, cache: &ThumbnailCache) {
    let snapshot = metrics.snapshot(cache.len(), cache.bytes());
    tracing::info!(
        target: "recorder::thumbnails",
        source = %source.display(),
        requested = snapshot.requested,
        completed = snapshot.completed,
        failed = snapshot.failed,
        stale = snapshot.stale,
        dropped = snapshot.dropped,
        cache_hits = snapshot.cache_hits,
        cache_misses = snapshot.cache_misses,
        evicted = snapshot.evicted,
        decode_ms = snapshot.decode_ms,
        resize_ms = snapshot.resize_ms,
        cache_entries = snapshot.cache_entries,
        cache_bytes = snapshot.cache_bytes,
        "thumbnail metrics"
    );
}

pub(crate) struct Snapshot {
    pub(crate) requested: u64,
    pub(crate) completed: u64,
    pub(crate) failed: u64,
    pub(crate) stale: u64,
    pub(crate) dropped: u64,
    pub(crate) cache_hits: u64,
    pub(crate) cache_misses: u64,
    pub(crate) evicted: u64,
    pub(crate) decode_ms: f64,
    pub(crate) resize_ms: f64,
    pub(crate) cache_entries: usize,
    pub(crate) cache_bytes: u64,
}

fn nanos_to_millis(nanos: u64) -> f64 {
    nanos as f64 / 1_000_000.
}
