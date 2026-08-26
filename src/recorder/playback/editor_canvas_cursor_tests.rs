use super::cursor_bounds;
use crate::recorder::cursor::CursorFrame;
use crate::recorder::cursor_settings::CursorStyle;
use gpui::{Bounds, point, px, size};

#[test]
fn cursor_can_overflow_recording_layer() {
    let recording_layer = Bounds::new(point(px(100.0), px(100.0)), size(px(200.0), px(100.0)));
    let cursor = CursorFrame {
        x: 0.0,
        y: 0.0,
        visible: true,
        scale: 1.0,
        asset: CursorStyle::Default.asset(),
    };

    let bounds = cursor_bounds(recording_layer, cursor, 200).expect("valid cursor bounds");

    assert!(bounds.origin.x < recording_layer.origin.x);
    assert!(bounds.origin.y < recording_layer.origin.y);
}
