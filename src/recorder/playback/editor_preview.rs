use gpui::*;
use gpui_component::ActiveTheme as _;

use super::super::zoom;
use super::{PlaybackView, editor_canvas::Canvas};

/// Inset between the preview pane and the canvas stage.
///
/// Named rather than inlined because the native compositor has to cover the
/// whole pane, not just the stage: it is the only thing painting inside the
/// preview once GPUI's fills there are suppressed, and an uncovered strip would
/// be transparent all the way through the window.
pub(super) const PREVIEW_PADDING: Rems = Rems(0.75);

pub(super) fn render(view: &PlaybackView, cx: &mut Context<PlaybackView>) -> impl IntoElement {
    let stage = if view.player.is_some() {
        Canvas::new(
            view.image.clone(),
            view.video_width,
            view.video_height,
            view.cursor_frame,
            view.cursor_images.clone(),
            view.motion_blur.sprite(),
            view.project_settings.canvas,
            view.project_settings.canvas_composition.clone(),
            view.background_image.clone(),
            zoom::effect_at(
                &view.project_settings.zoom_regions,
                view.timeline.playhead_us,
            ),
            cx.theme().background,
            cx.theme().popover,
            cx.theme().border,
            cx.theme().primary,
            cx.theme().foreground.opacity(0.24),
            view.selected_recording,
            view.canvas_bounds.clone(),
            view.metrics.clone(),
            view.frame_timing.clone(),
            view.frame_invalidated_at,
            view.playing,
            view.native_preview.composing(),
        )
        .size_full()
        .cursor(view.canvas_cursor_style())
        .on_scroll_wheel(cx.listener(|view, event, _, cx| {
            view.zoom_from_scroll(event, cx);
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|view, event: &MouseDownEvent, _, cx| {
                view.begin_canvas_interaction(event.position, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(|view, event: &MouseDownEvent, _, cx| {
                view.begin_pan(event.position, cx);
            }),
        )
        .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, window, cx| {
            view.update_canvas_hover(event.position, cx);
            view.update_canvas_interaction(event.position, window, cx);
            view.pan_to(event.position, cx);
        }))
        .on_mouse_exit(cx.listener(|view, _, _, cx| view.clear_canvas_hover(cx)))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|view, _, _, cx| {
                view.end_canvas_interaction(cx);
            }),
        )
        .on_mouse_up(
            MouseButton::Middle,
            cx.listener(|view, _, _, cx| {
                view.end_pan(cx);
            }),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|view, _, _, cx| {
                view.end_canvas_interaction(cx);
            }),
        )
        .on_mouse_up_out(
            MouseButton::Middle,
            cx.listener(|view, _, _, cx| {
                view.end_pan(cx);
            }),
        )
        .into_any_element()
    } else {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .child("Recording could not be loaded")
            .into_any_element()
    };

    let fps_badge = div()
        .absolute()
        .top_2()
        .right_2()
        .px_2()
        .py_1()
        .rounded(px(6.))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .text_xs()
        .child(format!("{:.0} FPS", view.preview_fps()));

    div()
        .flex_1()
        .self_stretch()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(view.native_preview.background(cx.theme().background))
        .child(
            div()
                .size_full()
                .p(PREVIEW_PADDING)
                .child(div().relative().size_full().child(stage).child(fps_badge)),
        )
}
