use std::sync::Arc;

use gpui::{Bounds, Pixels, RenderImage, point, px, size};

pub(crate) const THUMBNAIL_CELL_WIDTH_PX: f32 = 80.;
pub(crate) const THUMBNAIL_HEIGHT: u32 = 64;

const MIN_INTERVAL_US: u64 = 40_000;
const MAX_INTERVAL_US: u64 = 60_000_000;
const MAX_TARGETS: usize = 64;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ThumbnailSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl ThumbnailSize {
    pub(crate) fn memory_bytes(self) -> u64 {
        u64::from(self.width)
            .saturating_mul(u64::from(self.height))
            .saturating_mul(4)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TimelineViewport {
    pub(crate) duration_us: u64,
    pub(crate) scroll_us: u64,
    pub(crate) pixels_per_second: f32,
    pub(crate) width_px: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PlanSignature {
    pub(crate) duration_us: u64,
    pub(crate) interval_us: u64,
    pub(crate) first_bucket: u64,
    pub(crate) last_bucket: u64,
    pub(crate) size: ThumbnailSize,
}

#[derive(Clone, Debug)]
pub(crate) struct ThumbnailTarget {
    pub(crate) bucket: u64,
    pub(crate) interval_us: u64,
    pub(crate) start_us: u64,
    pub(crate) end_us: u64,
    pub(crate) timestamp_us: u64,
    pub(crate) size: ThumbnailSize,
}

#[derive(Clone, Debug)]
pub(crate) struct ThumbnailPlan {
    pub(crate) signature: PlanSignature,
    pub(crate) targets: Vec<ThumbnailTarget>,
}

#[derive(Clone, Debug)]
pub(crate) struct ThumbnailSlot {
    pub(crate) start_us: u64,
    pub(crate) end_us: u64,
    pub(crate) image: Option<Arc<RenderImage>>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ThumbnailStrip {
    pub(crate) slots: Vec<ThumbnailSlot>,
}

pub(crate) fn thumbnail_size(source_width: u32, source_height: u32) -> ThumbnailSize {
    if source_width == 0 || source_height == 0 {
        return ThumbnailSize::default();
    }

    let width = (f64::from(THUMBNAIL_HEIGHT) * f64::from(source_width) / f64::from(source_height))
        .round()
        .clamp(1., 512.) as u32;
    ThumbnailSize {
        width,
        height: THUMBNAIL_HEIGHT,
    }
}

pub(crate) fn plan(viewport: TimelineViewport, size: ThumbnailSize) -> ThumbnailPlan {
    let interval_us = thumbnail_interval_us(viewport.pixels_per_second);
    let scroll_us = viewport.scroll_us.min(viewport.duration_us);
    let (first_bucket, last_bucket) = visible_buckets(viewport, interval_us, scroll_us);
    let signature = PlanSignature {
        duration_us: viewport.duration_us,
        interval_us,
        first_bucket,
        last_bucket,
        size,
    };

    if viewport.duration_us == 0
        || viewport.width_px <= 0.
        || !viewport.width_px.is_finite()
        || !viewport.pixels_per_second.is_finite()
        || viewport.pixels_per_second <= 0.
        || size.width == 0
        || size.height == 0
    {
        return ThumbnailPlan {
            signature,
            targets: Vec::new(),
        };
    }

    let mut buckets = Vec::with_capacity(MAX_TARGETS);
    let mut targets = Vec::with_capacity(MAX_TARGETS);
    for bucket in first_bucket..=last_bucket {
        push_target(
            bucket,
            viewport.duration_us,
            interval_us,
            size,
            &mut buckets,
            &mut targets,
        );
    }
    if first_bucket > 0 {
        push_target(
            first_bucket - 1,
            viewport.duration_us,
            interval_us,
            size,
            &mut buckets,
            &mut targets,
        );
    }
    if let Some(bucket) = last_bucket.checked_add(1) {
        push_target(
            bucket,
            viewport.duration_us,
            interval_us,
            size,
            &mut buckets,
            &mut targets,
        );
    }

    ThumbnailPlan { signature, targets }
}

fn visible_buckets(viewport: TimelineViewport, interval_us: u64, scroll_us: u64) -> (u64, u64) {
    if viewport.duration_us == 0 || interval_us == 0 {
        return (0, 0);
    }
    let visible_us = micros_from_seconds(
        f64::from(viewport.width_px.max(0.)) / f64::from(viewport.pixels_per_second),
    )
    .max(1);
    let visible_end = scroll_us
        .saturating_add(visible_us)
        .min(viewport.duration_us);
    let last_time = visible_end.saturating_sub(1).max(scroll_us);
    (scroll_us / interval_us, last_time / interval_us)
}

fn push_target(
    bucket: u64,
    duration_us: u64,
    interval_us: u64,
    size: ThumbnailSize,
    buckets: &mut Vec<u64>,
    targets: &mut Vec<ThumbnailTarget>,
) {
    if targets.len() >= MAX_TARGETS || buckets.contains(&bucket) {
        return;
    }
    let Some(start_us) = bucket.checked_mul(interval_us) else {
        return;
    };
    if start_us >= duration_us {
        return;
    }
    let end_us = start_us.saturating_add(interval_us).min(duration_us);
    if end_us <= start_us {
        return;
    }
    let timestamp_us = start_us
        .saturating_add(interval_us / 2)
        .min(end_us.saturating_sub(1));
    let bucket = quantize_timestamp_us(timestamp_us, interval_us) / interval_us;
    if buckets.contains(&bucket) {
        return;
    }
    buckets.push(bucket);
    targets.push(ThumbnailTarget {
        bucket,
        interval_us,
        start_us,
        end_us,
        timestamp_us,
        size,
    });
}

pub(crate) fn thumbnail_interval_us(pixels_per_second: f32) -> u64 {
    if !pixels_per_second.is_finite() || pixels_per_second <= 0. {
        return MAX_INTERVAL_US;
    }
    let raw = (f64::from(THUMBNAIL_CELL_WIDTH_PX) / f64::from(pixels_per_second) * 1_000_000.)
        .clamp(MIN_INTERVAL_US as f64, MAX_INTERVAL_US as f64);
    let magnitude = 10_f64.powf(raw.log10().floor());
    let normalized = raw / magnitude;
    let factor = if normalized <= 1. {
        1.
    } else if normalized <= 2. {
        2.
    } else if normalized <= 5. {
        5.
    } else {
        10.
    };
    (factor * magnitude)
        .round()
        .clamp(MIN_INTERVAL_US as f64, MAX_INTERVAL_US as f64) as u64
}

pub(crate) fn quantize_timestamp_us(timestamp_us: u64, interval_us: u64) -> u64 {
    timestamp_us
        .checked_div(interval_us)
        .map(|bucket| bucket.saturating_mul(interval_us))
        .unwrap_or(timestamp_us)
}

pub(crate) fn aspect_fill_bounds(cell: Bounds<Pixels>, image_aspect: f32) -> Bounds<Pixels> {
    let cell_width = cell.size.width.as_f32().max(0.);
    let cell_height = cell.size.height.as_f32().max(0.);
    if cell_width <= 0. || cell_height <= 0. || !image_aspect.is_finite() || image_aspect <= 0. {
        return cell;
    }

    let cell_aspect = cell_width / cell_height;
    if image_aspect > cell_aspect {
        let image_width = cell_height * image_aspect;
        Bounds::new(
            point(
                px(cell.origin.x.as_f32() - (image_width - cell_width) / 2.),
                cell.origin.y,
            ),
            size(px(image_width), px(cell_height)),
        )
    } else {
        let image_height = cell_width / image_aspect;
        Bounds::new(
            point(
                cell.origin.x,
                px(cell.origin.y.as_f32() - (image_height - cell_height) / 2.),
            ),
            size(px(cell_width), px(image_height)),
        )
    }
}

pub(crate) fn clip_to_viewport(
    cell: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
) -> Option<Bounds<Pixels>> {
    let clipped = cell.intersect(&viewport);
    (clipped.size.width > Pixels::ZERO && clipped.size.height > Pixels::ZERO).then_some(clipped)
}

fn micros_from_seconds(seconds: f64) -> u64 {
    if !seconds.is_finite() || seconds <= 0. {
        return 0;
    }
    (seconds * 1_000_000.).round().clamp(0., u64::MAX as f64) as u64
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
