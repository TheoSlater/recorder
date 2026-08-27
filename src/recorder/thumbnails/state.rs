use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use gpui::RenderImage;

use super::{
    ThumbnailEvent, ThumbnailManager,
    cache::{CacheInsert, ThumbnailCache, ThumbnailKey},
    layout::PlanSignature,
};

const MAX_FAILURES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Status {
    Active,
    Unavailable,
}

pub(super) struct State {
    pub(super) cache: ThumbnailCache,
    pub(super) in_flight: HashMap<ThumbnailKey, u64>,
    pub(super) current_keys: HashSet<ThumbnailKey>,
    pub(super) failed: VecDeque<ThumbnailKey>,
    pub(super) signature: Option<PlanSignature>,
    pub(super) generation: u64,
    pub(super) status: Status,
    pub(super) pending_releases: Vec<Arc<RenderImage>>,
}

impl State {
    pub(super) fn new(status: Status) -> Self {
        Self {
            cache: ThumbnailCache::default(),
            in_flight: HashMap::new(),
            current_keys: HashSet::new(),
            failed: VecDeque::new(),
            signature: None,
            generation: 0,
            status,
            pending_releases: Vec::new(),
        }
    }
}

impl ThumbnailManager {
    pub(crate) fn apply_events(&self, events: impl IntoIterator<Item = ThumbnailEvent>) -> bool {
        let mut state = self.state.lock();
        let mut changed = false;
        for event in events {
            match event {
                ThumbnailEvent::Complete {
                    key,
                    generation,
                    image,
                    size,
                    decode_time,
                    resize_time,
                } => {
                    if state.in_flight.get(&key).copied() == Some(generation) {
                        state.in_flight.remove(&key);
                    }
                    if generation != state.generation {
                        self.metrics
                            .stale
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    self.metrics
                        .completed
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    self.metrics.decode_nanos.fetch_add(
                        duration_nanos(decode_time),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    self.metrics.resize_nanos.fetch_add(
                        duration_nanos(resize_time),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    let CacheInsert { evicted, inserted } =
                        state.cache.insert(key.clone(), image, size);
                    self.record_evictions(&mut state, evicted);
                    changed |= inserted && state.current_keys.contains(&key);
                }
                ThumbnailEvent::Failed {
                    key,
                    generation,
                    error,
                } => {
                    if state.in_flight.get(&key).copied() == Some(generation) {
                        state.in_flight.remove(&key);
                    }
                    self.metrics
                        .failed
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    self.remember_failure(&mut state, key);
                    tracing::debug!(
                        target: "recorder::thumbnails",
                        generation,
                        error = %error,
                        "thumbnail decode failed"
                    );
                }
                ThumbnailEvent::Stale { key, generation } => {
                    if state.in_flight.get(&key).copied() == Some(generation) {
                        state.in_flight.remove(&key);
                    }
                    self.metrics
                        .stale
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    // A zoom or scroll can fill the bounded request queue with
                    // obsolete work before current requests are enqueued. A
                    // stale event gives the view a chance to retry those
                    // current requests after the worker drains that work.
                    changed = true;
                }
                ThumbnailEvent::Unavailable(error) => {
                    state.status = Status::Unavailable;
                    state.in_flight.clear();
                    tracing::warn!(
                        target: "recorder::thumbnails",
                        error = %error,
                        "thumbnail subsystem unavailable"
                    );
                }
            }
        }
        changed
    }

    fn record_evictions(&self, state: &mut State, evicted: Vec<Arc<RenderImage>>) {
        if evicted.is_empty() {
            return;
        }
        self.metrics
            .evicted
            .fetch_add(evicted.len() as u64, std::sync::atomic::Ordering::Relaxed);
        state.pending_releases.extend(evicted);
    }

    fn remember_failure(&self, state: &mut State, key: ThumbnailKey) {
        if state.failed.contains(&key) {
            return;
        }
        state.failed.push_back(key);
        while state.failed.len() > MAX_FAILURES {
            state.failed.pop_front();
        }
    }
}

fn duration_nanos(duration: std::time::Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}
