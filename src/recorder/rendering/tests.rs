use super::{Backend, CompositionState, FrameId, FrameQueue, PhysicalSize, PreviewBounds};
use crate::recorder::{
    composition::{self, SourceSize},
    motion_blur::MotionBlurDescriptor,
    project_settings::ProjectSettings,
};

fn source() -> SourceSize {
    SourceSize {
        width: 1920,
        height: 1080,
    }
}

#[test]
fn converts_logical_sizes_at_each_dpi_scale() {
    assert_eq!(
        PhysicalSize::from_logical(800.0, 450.0, 1.0),
        Some(PhysicalSize::new(800, 450))
    );
    assert_eq!(
        PhysicalSize::from_logical(800.0, 450.0, 1.5),
        Some(PhysicalSize::new(1200, 675))
    );
    assert_eq!(
        PhysicalSize::from_logical(800.0, 450.0, 2.0),
        Some(PhysicalSize::new(1600, 900))
    );
}

#[test]
fn rejects_sizes_a_surface_cannot_use() {
    // A collapsed pane, a hidden window, and a nonsense scale must not reach a
    // swapchain as a zero or negative dimension.
    assert_eq!(PhysicalSize::from_logical(0.0, 450.0, 1.0), None);
    assert_eq!(PhysicalSize::from_logical(800.0, 0.4, 1.0), None);
    assert_eq!(PhysicalSize::from_logical(800.0, 450.0, 0.0), None);
    assert_eq!(PhysicalSize::from_logical(800.0, 450.0, f32::NAN), None);
    assert!(PhysicalSize::default().is_empty());
}

#[test]
fn converts_preview_rectangles_to_device_pixels() {
    let bounds = PreviewBounds::from_logical(10.0, 20.0, 800.0, 450.0, 2.0).expect("valid rect");

    assert_eq!(bounds.x, 20);
    assert_eq!(bounds.y, 40);
    assert_eq!(bounds.size, PhysicalSize::new(1600, 900));
}

#[test]
fn keeps_preview_edges_on_whole_pixels() {
    // Rounding the edges rather than the size keeps a rectangle the same width
    // as it slides, instead of breathing by a pixel at fractional offsets.
    let width = |x: f32| {
        PreviewBounds::from_logical(x, 0.0, 100.5, 50.0, 1.5)
            .expect("valid rect")
            .size
            .width
    };
    let widths: Vec<u32> = [0.0, 0.3, 0.5, 0.7].into_iter().map(width).collect();

    assert!(
        widths.iter().all(|value| value.abs_diff(widths[0]) <= 1),
        "{widths:?}"
    );
}

#[test]
fn rejects_collapsed_preview_rectangles() {
    assert_eq!(PreviewBounds::from_logical(0.0, 0.0, 0.0, 450.0, 1.0), None);
    assert_eq!(
        PreviewBounds::from_logical(0.0, 0.0, 800.0, 450.0, -1.0),
        None
    );
}

#[test]
fn selects_the_legacy_preview_until_a_backend_exists() {
    // No platform has a working native backend yet, so selection must keep the
    // editor on the preview that draws rather than a blank rectangle.
    assert_eq!(super::available_backend(), Backend::LegacyGpui);
    assert_eq!(Backend::default(), Backend::LegacyGpui);
}

#[test]
fn presents_the_newest_frame_of_a_generation() {
    let mut queue = FrameQueue::new();

    assert!(queue.offer(FrameId::new(0, 1, 1_000), "first"));
    assert!(queue.offer(FrameId::new(0, 2, 2_000), "second"));

    // The first frame never reached the screen, so it is coalesced away rather
    // than presented late.
    let (id, frame) = queue.take().expect("a frame is pending");
    assert_eq!(id.sequence, 2);
    assert_eq!(frame, "second");
    assert_eq!(queue.dropped(), 1);
    assert!(queue.take().is_none());
}

#[test]
fn rejects_frames_from_a_replaced_seek() {
    let mut queue = FrameQueue::new();
    queue.set_generation(4);

    assert!(!queue.offer(FrameId::new(3, 9, 9_000), "stale"));
    assert!(queue.take().is_none());
    assert!(queue.offer(FrameId::new(4, 1, 1_000), "current"));
    assert_eq!(queue.take().map(|(_, frame)| frame), Some("current"));
}

#[test]
fn drops_pending_frames_when_a_seek_supersedes_them() {
    let mut queue = FrameQueue::new();
    queue.offer(FrameId::new(0, 1, 1_000), "before");
    queue.set_generation(1);

    assert!(queue.take().is_none());
    assert_eq!(queue.dropped(), 1);
}

#[test]
fn ignores_out_of_order_decodes() {
    let mut queue = FrameQueue::new();
    queue.offer(FrameId::new(0, 5, 5_000), "newest");

    assert!(!queue.offer(FrameId::new(0, 4, 4_000), "older"));
    assert_eq!(queue.take().map(|(_, frame)| frame), Some("newest"));
}

#[test]
fn composition_state_carries_the_shared_frame() {
    let settings = ProjectSettings::default();
    let frame = composition::evaluate(&settings, source(), 0, None);
    let state = CompositionState::new(
        PhysicalSize::new(1920, 1080),
        source(),
        frame,
        settings.canvas_composition.background.clone(),
        MotionBlurDescriptor::inactive(),
    );

    assert!(!state.is_empty());
    assert_eq!(state.frame, frame);
    assert!((state.output_size.aspect() - 16.0 / 9.0).abs() < 1e-4);
}

#[test]
fn editor_camera_never_reaches_composition_state() {
    // The renderer is handed a camera-free description. Moving the editor
    // viewport must produce an identical CompositionState.
    let mut navigated = ProjectSettings::default();
    navigated.canvas.zoom = 3.5;
    navigated.canvas.pan_x = 240.0;
    navigated.canvas.pan_y = -120.0;

    let resting = composition::evaluate(&ProjectSettings::default(), source(), 0, None);
    let moved = composition::evaluate(&navigated, source(), 0, None);

    assert_eq!(resting, moved);
}
