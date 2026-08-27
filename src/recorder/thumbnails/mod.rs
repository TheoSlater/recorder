mod cache;
mod decoder;
mod extractor;
mod layout;
mod manager;
mod metrics;
mod state;

use std::sync::Arc;

use gpui::RenderImage;

#[cfg(test)]
pub(crate) use layout::ThumbnailSize;
pub(crate) use layout::{
    ThumbnailPlan, ThumbnailSlot, ThumbnailStrip, TimelineViewport, aspect_fill_bounds,
    clip_to_viewport, plan, thumbnail_size,
};
pub(crate) use manager::ThumbnailManager;

use std::sync::atomic::{AtomicBool, AtomicU64};

use crossbeam_channel::Sender;

use self::{cache::ThumbnailKey as CacheKey, layout::ThumbnailSize as OutputSize};

pub(crate) enum ThumbnailEvent {
    Complete {
        key: CacheKey,
        generation: u64,
        image: Arc<RenderImage>,
        size: OutputSize,
        decode_time: std::time::Duration,
        resize_time: std::time::Duration,
    },
    Failed {
        key: CacheKey,
        generation: u64,
        error: String,
    },
    Stale {
        key: CacheKey,
        generation: u64,
    },
    Unavailable(String),
}

pub(super) enum WorkerCommand {
    Request(ExtractionRequest),
    Shutdown,
}

pub(super) struct ExtractionRequest {
    pub(super) key: CacheKey,
    pub(super) timestamp_us: u64,
    pub(super) size: OutputSize,
    pub(super) generation: u64,
}

pub(super) struct WorkerContext {
    pub(super) commands: crossbeam_channel::Receiver<WorkerCommand>,
    pub(super) events: Sender<ThumbnailEvent>,
    pub(super) stop: std::sync::Arc<AtomicBool>,
    pub(super) latest_generation: std::sync::Arc<AtomicU64>,
}
