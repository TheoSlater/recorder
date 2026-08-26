use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable, Size, button::*, h_flex, v_flex,
};

use super::{PlaybackView, preview_rate::PREVIEW_RATES};

pub(super) fn render(view: &PlaybackView, cx: &mut Context<PlaybackView>) -> impl IntoElement {
    let back_button = Button::new("back-projects")
        .ghost()
        .compact()
        .with_size(Size::Small)
        .label("← Projects")
        .tooltip("Return to projects")
        .on_click(cx.listener(|_, _, window, _| window.remove_window()));
    let export_button = Button::new("export")
        .outline()
        .compact()
        .with_size(Size::Small)
        .label(view.export_label())
        .disabled(!view.export_available() && !view.exporting())
        .tooltip(if view.exporting() {
            "Cancel export"
        } else {
            "Export edited recording"
        })
        .on_click(cx.listener(|view, _, window, cx| {
            view.export_or_cancel(window, cx);
        }));
    let preview_rates = h_flex()
        .items_center()
        .gap_1()
        .p_1()
        .rounded(px(7.))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .child(
            div()
                .px_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Preview"),
        )
        .children(PREVIEW_RATES.iter().copied().map(|rate| {
            let selected = rate == view.preview_rate();
            Button::new(rate.id())
                .outline()
                .compact()
                .with_size(Size::Small)
                .label(rate.label())
                .when(selected, |button| button.primary())
                .tooltip(format!("Preview at {}", rate.label()))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.set_preview_rate(rate, cx);
                }))
        }));

    h_flex()
        .h(px(44.))
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .px_3()
        .border_b_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .child(back_button)
        .child(div().h(px(20.)).w(px(1.)).bg(cx.theme().border))
        .child(
            v_flex()
                .justify_center()
                .gap_0()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Recording"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Editor"),
                ),
        )
        .child(div().flex_1())
        .child(preview_rates)
        .child(export_button)
}
