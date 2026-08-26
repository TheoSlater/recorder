use gpui::*;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable, Size, button::*, h_flex};

use super::PlaybackView;

pub(super) fn render_toolbar(
    view: &PlaybackView,
    cx: &mut Context<PlaybackView>,
) -> impl IntoElement {
    let play_icon = if view.playing {
        IconName::Pause
    } else {
        IconName::Play
    };
    let diagnostic = view.error.clone().or_else(|| {
        let status = view.cursor_overlay.status();
        (!status.starts_with("Cursor reconstructed")).then(|| status.to_string().into())
    });
    let time_label = diagnostic.unwrap_or_else(|| {
        format!(
            "{} / {}",
            format_playback_time(view.playhead_seconds()),
            format_playback_time(view.duration_seconds())
        )
        .into()
    });
    let play_button = Button::new("play-pause")
        .icon(play_icon)
        .primary()
        .with_size(Size::Small)
        .rounded(px(18.))
        .tooltip(if view.playing {
            "Pause (Space)"
        } else {
            "Play (Space)"
        })
        .on_click(cx.listener(|view, _, _, cx| view.toggle(cx)));
    let start_button = transport_button(
        "seek-start",
        IconName::ArrowLeft,
        true,
        "Jump to start",
        cx.theme().foreground,
    )
    .on_click(cx.listener(|view, _, _, cx| view.seek_to(0.0, cx)));
    let end_button = transport_button(
        "seek-end",
        IconName::ArrowRight,
        false,
        "Jump to end",
        cx.theme().foreground,
    )
    .on_click(cx.listener(|view, _, _, cx| {
        let duration = view.duration_seconds();
        view.seek_to(duration, cx);
    }));

    let add_background = toolbar_button("add-background", IconName::Plus, "Add background image")
        .on_click(cx.listener(|view, _, window, cx| {
            view.choose_background_image(window, cx);
        }));
    let fit_button = toolbar_button("fit-preview", IconName::Frame, "Reset canvas view")
        .on_click(cx.listener(|view, _, _, cx| view.reset_canvas_view(cx)));
    let zoom_out_button = toolbar_button("zoom-out", IconName::Minus, "Zoom out canvas")
        .on_click(cx.listener(|view, _, _, cx| view.adjust_canvas_zoom(-0.1, cx)));
    let zoom_in_button = toolbar_button("zoom-in", IconName::Plus, "Zoom in canvas")
        .on_click(cx.listener(|view, _, _, cx| view.adjust_canvas_zoom(0.1, cx)));

    h_flex()
        .h(px(52.))
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .px_2()
        .border_t_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .child(
            h_flex()
                .w(px(112.))
                .flex_shrink_0()
                .items_center()
                .child(add_background),
        )
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .justify_center()
                .gap_1()
                .child(start_button)
                .child(play_button)
                .child(end_button)
                .child(
                    div()
                        .min_w(px(112.))
                        .max_w(px(240.))
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .truncate()
                        .text_center()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(time_label),
                ),
        )
        .child(
            h_flex()
                .w(px(112.))
                .flex_shrink_0()
                .justify_end()
                .items_center()
                .gap_1()
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .p_1()
                        .rounded(px(7.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().popover)
                        .child(fit_button)
                        .child(zoom_out_button)
                        .child(zoom_in_button),
                ),
        )
}

fn toolbar_button(id: &'static str, icon: IconName, tooltip: &'static str) -> Button {
    Button::new(id)
        .icon(icon)
        .ghost()
        .compact()
        .rounded(px(18.))
        .tooltip(tooltip)
}

fn transport_button(
    id: &'static str,
    arrow: IconName,
    bar_first: bool,
    tooltip: &'static str,
    color: Hsla,
) -> Button {
    let terminal = div().h(px(12.)).w(px(2.)).bg(color);
    let arrow = div().text_color(color).child(Icon::new(arrow));
    let glyph = h_flex().items_center().gap_0();
    let glyph = if bar_first {
        glyph.child(terminal).child(arrow)
    } else {
        glyph.child(arrow).child(terminal)
    };

    Button::new(id)
        .ghost()
        .compact()
        .rounded(px(18.))
        .tooltip(tooltip)
        .child(glyph)
}

fn format_playback_time(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    let total_centiseconds = (seconds * 100.0).round() as u64;
    let centiseconds = total_centiseconds % 100;
    let total_seconds = total_centiseconds / 100;
    let display_seconds = total_seconds % 60;
    let total_minutes = total_seconds / 60;
    let display_minutes = total_minutes % 60;

    if total_minutes >= 60 {
        format!(
            "{:02}:{:02}:{:02}.{:02}",
            total_minutes / 60,
            display_minutes,
            display_seconds,
            centiseconds
        )
    } else {
        format!(
            "{:02}:{:02}.{:02}",
            display_minutes, display_seconds, centiseconds
        )
    }
}

#[cfg(test)]
mod tests {
    use super::format_playback_time;

    #[test]
    fn formats_playback_time() {
        assert_eq!(format_playback_time(8.64), "00:08.64");
        assert_eq!(format_playback_time(65.1), "01:05.10");
        assert_eq!(format_playback_time(3661.23), "01:01:01.23");
    }
}
