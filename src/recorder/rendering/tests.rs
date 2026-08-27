use super::{
    Backend, CanvasPlacement, CompositionState, FrameId, FrameQueue, PhysicalSize, PreviewBounds,
};
use crate::recorder::{
    composition::{self, NormalizedRect, SourceSize},
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
fn defaults_to_the_legacy_preview() {
    // The native compositor is opt-in until it has been validated on hardware,
    // so an editor that asks for nothing keeps the preview that draws.
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
    assert!(queue.offer(FrameId::new(4, 1, 1_000), "current"));

    // A decode that finishes after the user has already seeked past it must not
    // pull the preview backwards.
    assert!(!queue.offer(FrameId::new(3, 9, 9_000), "stale"));
    assert_eq!(queue.take().map(|(_, frame)| frame), Some("current"));
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
    let size = PhysicalSize::new(1920, 1080);
    let state = CompositionState::new(
        size,
        CanvasPlacement::filling(size),
        source(),
        frame,
        settings.canvas_composition.background.clone(),
        MotionBlurDescriptor::inactive(),
    );

    assert!(!state.is_empty());
    assert_eq!(state.frame, frame);
    assert!((state.target_size.aspect() - 16.0 / 9.0).abs() < 1e-4);
}

/// Export draws into a target that *is* the canvas, which is what keeps its
/// pixels free of anything the editor's placement carries.
#[test]
fn a_filling_canvas_leaves_composition_rects_untouched() {
    let placement = CanvasPlacement::filling(PhysicalSize::new(1920, 1080));
    let recording = NormalizedRect {
        x: 0.1,
        y: 0.2,
        width: 0.5,
        height: 0.25,
    };

    assert_eq!(placement.place(recording), recording);
    assert_eq!(placement.corner_radius, 0.0);
}

#[test]
fn places_canvas_rects_inside_the_preview_surface() {
    // A canvas occupying the middle half of the surface: everything drawn on it
    // has to land inside that half, at half the size.
    let placement = CanvasPlacement {
        rect: NormalizedRect {
            x: 0.25,
            y: 0.25,
            width: 0.5,
            height: 0.5,
        },
        size: PhysicalSize::new(960, 540),
        corner_radius: 20.0,
        surround: [0.0, 0.0, 0.0, 1.0],
    };

    let whole_canvas = placement.place(NormalizedRect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    });
    assert_eq!(whole_canvas, placement.rect);

    let centered = placement.place(NormalizedRect {
        x: 0.5,
        y: 0.5,
        width: 0.25,
        height: 0.25,
    });
    assert!((centered.x - 0.5).abs() < 1e-9, "{centered:?}");
    assert!((centered.y - 0.5).abs() < 1e-9, "{centered:?}");
    assert!((centered.width - 0.125).abs() < 1e-9, "{centered:?}");
}

/// A zoom can push the recording past the canvas. The placement must map it
/// faithfully — clipping is the renderer's scissor, not this conversion's job.
#[test]
fn maps_layers_that_overflow_the_canvas() {
    let placement = CanvasPlacement {
        rect: NormalizedRect {
            x: 0.2,
            y: 0.1,
            width: 0.6,
            height: 0.8,
        },
        size: PhysicalSize::new(600, 800),
        corner_radius: 0.0,
        surround: [0.0; 4],
    };

    let overflowing = placement.place(NormalizedRect {
        x: -0.5,
        y: -0.5,
        width: 2.0,
        height: 2.0,
    });

    assert!(overflowing.x < placement.rect.x);
    assert!(overflowing.width > placement.rect.width);
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
