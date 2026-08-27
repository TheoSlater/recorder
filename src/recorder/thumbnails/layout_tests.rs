use gpui::{Bounds, point, px, size};

use super::{
    THUMBNAIL_CELL_WIDTH_PX, ThumbnailSize, TimelineViewport, aspect_fill_bounds, clip_to_viewport,
    plan, quantize_timestamp_us, thumbnail_interval_us, thumbnail_size,
};

fn viewport(duration_us: u64, scroll_us: u64, pixels_per_second: f32) -> TimelineViewport {
    TimelineViewport {
        duration_us,
        scroll_us,
        pixels_per_second,
        width_px: 800.,
    }
}

#[test]
fn calculates_visible_timestamps_with_one_bucket_prefetch() {
    let result = plan(
        viewport(120_000_000, 30_000_000, 80.),
        thumbnail_size(1920, 1080),
    );

    assert_eq!(result.signature.interval_us, 1_000_000);
    assert_eq!(
        result
            .targets
            .iter()
            .map(|target| target.timestamp_us)
            .collect::<Vec<_>>(),
        vec![
            30_500_000, 31_500_000, 32_500_000, 33_500_000, 34_500_000, 35_500_000, 36_500_000,
            37_500_000, 38_500_000, 39_500_000, 29_500_000, 40_500_000
        ]
    );
}

#[test]
fn zoomed_in_density_is_higher() {
    let zoomed_out = thumbnail_interval_us(20.);
    let zoomed_in = thumbnail_interval_us(320.);

    assert!(zoomed_in < zoomed_out);
    assert!(1_000_000. / zoomed_in as f64 > 1_000_000. / zoomed_out as f64);
}

#[test]
fn timestamp_requests_are_stable_buckets() {
    assert_eq!(quantize_timestamp_us(2_049_999, 1_000_000), 2_000_000);
    assert_eq!(quantize_timestamp_us(2_999_999, 1_000_000), 2_000_000);
    assert_eq!(quantize_timestamp_us(2_999_999, 0), 2_999_999);
}

#[test]
fn targets_stay_inside_recording() {
    let result = plan(
        viewport(2_500_000, 0, 12.),
        ThumbnailSize {
            width: 114,
            height: 64,
        },
    );

    assert!(result.targets.iter().all(|target| {
        target.start_us < target.end_us
            && target.start_us < 2_500_000
            && target.end_us <= 2_500_000
            && target.timestamp_us < 2_500_000
    }));
}

#[test]
fn aspect_fill_crops_without_distorting() {
    let cell = Bounds::new(point(px(10.), px(20.)), size(px(80.), px(60.)));
    let bounds = aspect_fill_bounds(cell, 16. / 9.);

    assert_eq!(bounds.size.height, cell.size.height);
    assert!(bounds.size.width > cell.size.width);
    assert_eq!(bounds.origin.y, cell.origin.y);
}

#[test]
fn label_gutter_is_outside_thumbnail_viewport() {
    let gutter = Bounds::new(point(px(0.), px(0.)), size(px(64.), px(100.)));
    let viewport = Bounds::new(point(px(64.), px(0.)), size(px(800.), px(100.)));
    let cell = Bounds::new(point(px(40.), px(44.)), size(px(80.), px(20.)));

    assert!(gutter.intersect(&cell).size.width > px(0.));
    let clipped = clip_to_viewport(cell, viewport).expect("cell reaches the timeline viewport");
    assert_eq!(clipped.origin.x, px(64.));
    assert_eq!(clipped.size.width, px(56.));
}

#[test]
fn thumbnail_size_preserves_source_aspect() {
    assert_eq!(
        thumbnail_size(1920, 1080),
        ThumbnailSize {
            width: 114,
            height: 64
        }
    );
    assert_eq!(
        thumbnail_size(3440, 1440),
        ThumbnailSize {
            width: 153,
            height: 64
        }
    );
    assert_eq!(THUMBNAIL_CELL_WIDTH_PX, 80.);
}
