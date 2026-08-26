use std::rc::Rc;

use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex, v_flex};

use super::super::alerts::{self, AlertCloseHandler};
use super::{
    PlaybackView, editor_inspector, editor_preview, editor_timeline, editor_toolbar, playback_ui,
};

pub(super) fn render(view: &PlaybackView, cx: &mut Context<PlaybackView>) -> impl IntoElement {
    let view_entity = cx.entity().downgrade();
    let on_close: AlertCloseHandler = Rc::new(move |id, _, cx| {
        let _ = view_entity.update(cx, |view, cx| {
            if view.pending_alerts.dismiss(id) {
                cx.notify();
            }
        });
    });
    let alert_layer = alerts::render_layer(
        &view.pending_alerts,
        on_close,
        cx.theme().popover,
        cx.theme().border,
    );

    v_flex()
        .relative()
        .size_full()
        .min_h(px(0.))
        .bg(cx.theme().background)
        .text_color(cx.theme().foreground)
        .capture_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
            if !event.is_held
                && event.keystroke.key.eq_ignore_ascii_case("space")
                && !event.keystroke.modifiers.modified()
            {
                window.prevent_default();
                view.toggle(cx);
            }
        }))
        .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| {
            view.update_timeline_hover(event.position, cx);
        }))
        .child(editor_toolbar::render(view, cx))
        .child(
            h_flex()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .child(editor_preview::render(view, cx))
                .child(editor_inspector::render(view, cx)),
        )
        .child(playback_ui::render_toolbar(view, cx))
        .child(editor_timeline::render(view, cx))
        .children(alert_layer)
}
