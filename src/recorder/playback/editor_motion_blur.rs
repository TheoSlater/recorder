use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex, slider::Slider, v_flex};

use super::PlaybackView;

pub(super) fn render(view: &PlaybackView, cx: &mut Context<PlaybackView>) -> AnyElement {
    let amount = view.project_settings.motion_blur.amount;
    let muted_foreground = cx.theme().muted_foreground;
    let slider = Slider::new(&view.motion_blur_slider)
        .flex_1()
        .min_w(px(72.))
        .h(px(24.));

    v_flex()
        .gap_2()
        .p_2()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child("Motion Blur"),
        )
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(64.))
                        .text_xs()
                        .text_color(muted_foreground)
                        .child("Amount"),
                )
                .child(slider)
                .child(
                    div()
                        .w(px(40.))
                        .text_xs()
                        .text_color(muted_foreground)
                        .child(format!("{}%", (amount * 100.0).round() as u32)),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(muted_foreground)
                .child("Smears fast cursor movement. 0% renders every frame sharp."),
        )
        .into_any_element()
}
