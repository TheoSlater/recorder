use gpui::*;
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable, Size, button::*, h_flex, select::Select,
    slider::Slider, switch::Switch, v_flex,
};

use super::PlaybackView;

pub(super) fn render(view: &PlaybackView, cx: &mut Context<PlaybackView>) -> AnyElement {
    let settings = view.project_settings.cursor;
    let show_cursor = Switch::new("cursor-visible")
        .checked(settings.visible)
        .with_size(Size::Small)
        .tooltip("Show reconstructed cursor")
        .on_click(cx.listener(|view, checked, _, cx| {
            let mut settings = view.project_settings.cursor;
            settings.visible = *checked;
            view.set_cursor_settings(settings, cx);
        }));
    let style = Select::new(&view.cursor_style_select)
        .with_size(Size::Small)
        .w(px(116.));
    let size_slider = Slider::new(&view.cursor_size_slider)
        .flex_1()
        .min_w(px(72.))
        .h(px(24.));
    let smoothing_slider = Slider::new(&view.cursor_smoothing_slider)
        .flex_1()
        .min_w(px(72.))
        .h(px(24.));
    let muted_foreground = cx.theme().muted_foreground;
    let border = cx.theme().border;
    let add_button = Button::new("add-cursor-size-keyframe")
        .primary()
        .compact()
        .with_size(Size::Small)
        .label("Add keyframe")
        .disabled(view.duration_seconds() <= 0.0)
        .tooltip("Animate the cursor size from the current playhead")
        .on_click(cx.listener(|view, _, _, cx| {
            view.add_cursor_size_keyframe(cx);
        }));

    let keyframe_controls =
        render_keyframe_controls(view, cx, add_button, border, muted_foreground);

    v_flex()
        .gap_2()
        .p_2()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child("Cursor"),
        )
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(label("Show cursor"))
                .child(show_cursor),
        )
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(label("Style"))
                .child(style),
        )
        .child(slider_row(
            "Size",
            size_slider,
            format!("{:.1}x", settings.scale),
            muted_foreground,
        ))
        .child(slider_row(
            "Smoothing",
            smoothing_slider,
            format!("{}%", (settings.smoothing * 100.0).round() as u32),
            muted_foreground,
        ))
        .child(keyframe_controls)
        .into_any_element()
}

fn render_keyframe_controls(
    view: &PlaybackView,
    cx: &mut Context<PlaybackView>,
    add_button: impl IntoElement,
    border: Hsla,
    muted: Hsla,
) -> AnyElement {
    let Some(index) = view.selected_cursor_size_region else {
        return v_flex()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(border)
            .child(section_title("Cursor size"))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("No keyframe selected"),
            )
            .child(add_button)
            .into_any_element();
    };
    let Some(region) = view
        .project_settings
        .cursor_size_regions
        .get(index)
        .copied()
    else {
        return v_flex()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(border)
            .child(section_title("Cursor size"))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("No keyframe selected"),
            )
            .child(add_button)
            .into_any_element();
    };

    let peak_down = Button::new("cursor-size-keyframe-down")
        .outline()
        .compact()
        .with_size(Size::Small)
        .label("−")
        .on_click(cx.listener(|view, _, _, cx| {
            view.adjust_selected_cursor_size(-0.1, cx);
        }));
    let peak_up = Button::new("cursor-size-keyframe-up")
        .outline()
        .compact()
        .with_size(Size::Small)
        .label("+")
        .on_click(cx.listener(|view, _, _, cx| {
            view.adjust_selected_cursor_size(0.1, cx);
        }));
    let delete_button = Button::new("delete-cursor-size-keyframe")
        .ghost()
        .compact()
        .with_size(Size::Small)
        .label("Delete")
        .on_click(cx.listener(|view, _, _, cx| {
            view.delete_selected_cursor_size_region(cx);
        }));

    v_flex()
        .gap_2()
        .pt_2()
        .border_t_1()
        .border_color(border)
        .child(section_title("Cursor size"))
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(div().text_xs().text_color(muted).child("Range"))
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(format_range(region.start_us, region.end_us)),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(div().text_xs().child("Peak size"))
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .child(peak_down)
                        .child(
                            div()
                                .w(px(44.))
                                .text_xs()
                                .text_center()
                                .child(format!("{:.1}×", region.end_scale)),
                        )
                        .child(peak_up),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(div().text_xs().child("Easing"))
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(region.easing.label()),
                ),
        )
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(add_button)
                .child(delete_button),
        )
        .into_any_element()
}

fn section_title(title: &'static str) -> impl IntoElement {
    div().text_sm().font_weight(FontWeight::MEDIUM).child(title)
}

fn format_range(start_us: u64, end_us: u64) -> String {
    format!(
        "{:.2}s – {:.2}s",
        start_us as f64 / 1_000_000.,
        end_us as f64 / 1_000_000.
    )
}

fn label(text: &'static str) -> impl IntoElement {
    div().w(px(76.)).text_xs().child(text)
}

fn slider_row(
    name: &'static str,
    slider: impl IntoElement,
    value: String,
    value_color: Hsla,
) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap_1()
        .child(label(name))
        .child(slider)
        .child(
            div()
                .w(px(40.))
                .text_xs()
                .text_right()
                .text_color(value_color)
                .child(value),
        )
}
