use super::cursor_bounds;
use crate::recorder::cursor_settings::CursorStyle;
use crate::recorder::{
    composition::{self, SourceSize},
    cursor::CursorFrame,
    project_settings::ProjectSettings,
};
use gpui::{Bounds, point, px, size};

#[test]
fn cursor_can_overflow_recording_layer() {
    let cursor = CursorFrame {
        x: 0.0,
        y: 0.0,
        visible: true,
        scale: 1.0,
        asset: CursorStyle::Default.asset(),
    };

    let settings = ProjectSettings::default();
    let source = SourceSize {
        width: 1920,
        height: 1080,
    };
    let frame = composition::evaluate(&settings, source, 0, Some(cursor));
    let canvas = Bounds::new(point(px(100.0), px(100.0)), size(px(200.0), px(100.0)));
    let bounds = cursor_bounds(canvas, frame, source, cursor.asset).expect("valid cursor bounds");
    let recording_layer = Bounds::new(
        point(
            px(canvas.origin.x.as_f32()
                + frame.base_recording.x as f32 * canvas.size.width.as_f32()),
            px(canvas.origin.y.as_f32()
                + frame.base_recording.y as f32 * canvas.size.height.as_f32()),
        ),
        size(
            px(frame.base_recording.width as f32 * canvas.size.width.as_f32()),
            px(frame.base_recording.height as f32 * canvas.size.height.as_f32()),
        ),
    );

    assert!(bounds.origin.x < recording_layer.origin.x);
    assert!(bounds.origin.y < recording_layer.origin.y);
}
