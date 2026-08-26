use gpui::*;
use gpui_component::{ActiveTheme as _, scroll::ScrollableElement as _, v_flex};

use super::{PlaybackView, editor_canvas_controls, editor_cursor, editor_zoom};

pub(super) fn render(view: &PlaybackView, cx: &mut Context<PlaybackView>) -> impl IntoElement {
    let border = cx.theme().border;
    let muted_foreground = cx.theme().muted_foreground;

    v_flex()
        .w(px(248.))
        .flex_shrink_0()
        .min_h(px(0.))
        .border_l_1()
        .border_color(border)
        .bg(cx.theme().popover)
        .p_3()
        .overflow_y_scrollbar()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Inspector"),
        )
        .child(editor_zoom::render(view, cx))
        .child(editor_cursor::render(view, cx))
        .child(editor_canvas_controls::render(view, cx))
        .child(render_section("Video", border, muted_foreground))
}

fn render_section(title: &'static str, border: Hsla, muted_foreground: Hsla) -> impl IntoElement {
    v_flex()
        .gap_1()
        .p_2()
        .border_b_1()
        .border_color(border)
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
        .child(
            div()
                .text_xs()
                .text_color(muted_foreground)
                .child("Controls will appear here"),
        )
}
