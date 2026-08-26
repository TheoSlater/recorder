use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use super::{Nv12Layout, copy_nv12_rows, copy_nv12_rows_parallel, nv12_plane_height};

#[test]
fn finds_aligned_nv12_plane_height() {
    assert_eq!(nv12_plane_height(1920 * 1_632, 1920, 1_080), 1_088);
    assert_eq!(nv12_plane_height(1920 * 1_620, 1920, 1_080), 1_080);
    assert_eq!(nv12_plane_height(1_936 * 1_584, 1_936, 1_048), 1_056);
}

#[test]
fn reads_chroma_after_aligned_luma_rows() {
    let width = 4;
    let height = 2;
    let source_stride = 4;
    let plane_height = 4;
    let mut source = vec![0; source_stride * (plane_height + plane_height / 2)];
    source[..source_stride * height].fill(100);
    source[source_stride * plane_height..].fill(128);
    let mut pixels = vec![0; width * height * 4];
    let layout = Nv12Layout {
        width,
        height,
        plane_height,
        stride: source_stride as i32,
        source_stride,
        row_bytes: width * 4,
    };
    let cancelled = AtomicBool::new(false);

    copy_nv12_rows(
        &source,
        layout,
        0,
        height,
        &mut pixels,
        &|| true,
        &cancelled,
    );

    assert_eq!(&pixels[0..4], &[98, 98, 98, 255]);
    assert_eq!(&pixels[width * 4..width * 4 + 4], &[98, 98, 98, 255]);
}

#[test]
fn cancellation_stops_parallel_conversion() {
    let width = 64;
    let height = 64;
    let source_stride = width;
    let plane_height = height;
    let source = vec![128; source_stride * (plane_height + plane_height / 2)];
    let layout = Nv12Layout {
        width,
        height,
        plane_height,
        stride: source_stride as i32,
        source_stride,
        row_bytes: width * 4,
    };
    let mut pixels = vec![0; width * height * 4];
    let cancelled = AtomicBool::new(false);

    assert!(
        copy_nv12_rows_parallel(&source, layout, 4, &mut pixels, &|| false, &cancelled,).unwrap()
    );
    assert!(cancelled.load(Ordering::Acquire));
}

#[test]
#[ignore = "manual performance probe; run with cargo test -- --ignored --nocapture"]
fn benchmarks_row_scheduler() {
    const WIDTH: usize = 3_440;
    const HEIGHT: usize = 1_440;
    const ITERATIONS: usize = 8;
    let source_stride = WIDTH;
    let plane_height = HEIGHT;
    let source = vec![128; source_stride * (plane_height + plane_height / 2)];
    let layout = Nv12Layout {
        width: WIDTH,
        height: HEIGHT,
        plane_height,
        stride: source_stride as i32,
        source_stride,
        row_bytes: WIDTH * 4,
    };
    let old_started = Instant::now();
    for _ in 0..ITERATIONS {
        let mut pixels = vec![0; WIDTH * HEIGHT * 4];
        convert_with_per_frame_threads(&source, layout, 8, &mut pixels);
        std::hint::black_box(pixels);
    }
    let old_per_frame = old_started.elapsed().as_secs_f64() * 1_000. / ITERATIONS as f64;

    let new_started = Instant::now();
    for _ in 0..ITERATIONS {
        let mut pixels = vec![0; WIDTH * HEIGHT * 4];
        let cancelled = AtomicBool::new(false);
        assert!(
            !copy_nv12_rows_parallel(&source, layout, 8, &mut pixels, &|| true, &cancelled,)
                .unwrap()
        );
        std::hint::black_box(pixels);
    }
    let persistent = new_started.elapsed().as_secs_f64() * 1_000. / ITERATIONS as f64;
    eprintln!(
        "NV12 row scheduler 3440x1440: per_frame_threads={old_per_frame:.2}ms ({:.1} FPS), rayon_pool={persistent:.2}ms ({:.1} FPS)",
        1_000. / old_per_frame,
        1_000. / persistent,
    );
}

fn convert_with_per_frame_threads(
    source: &[u8],
    layout: Nv12Layout,
    worker_count: usize,
    pixels: &mut [u8],
) {
    let rows_per_worker = layout.height.div_ceil(worker_count);
    std::thread::scope(|scope| {
        let mut output = pixels;
        for row_start in (0..layout.height).step_by(rows_per_worker) {
            let row_end = (row_start + rows_per_worker).min(layout.height);
            let output_len = (row_end - row_start) * layout.row_bytes;
            let (worker_output, remaining) = output.split_at_mut(output_len);
            output = remaining;
            scope.spawn(move || {
                let cancelled = AtomicBool::new(false);
                copy_nv12_rows(
                    source,
                    layout,
                    row_start,
                    row_end,
                    worker_output,
                    &|| true,
                    &cancelled,
                );
            });
        }
    });
}
