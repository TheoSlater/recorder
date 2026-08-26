use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable, Size, button::*, h_flex, v_flex,
};

use super::super::zoom::ZoomTarget;
use super::PlaybackView;

pub(super) fn render(view: &PlaybackView, cx: &mut Context<PlaybackView>) -> AnyElement {
    let border = cx.theme().border;
    let muted = cx.theme().muted_foreground;
    let add_button = Button::new("add-zoom-region")
        .primary()
        .compact()
        .with_size(Size::Small)
        .label("Add region")
        .disabled(view.duration_seconds() <= 0.0)
        .on_click(cx.listener(|view, _, _, cx| view.add_zoom_region(cx)));
    let auto_zoom_button = Button::new("generate-auto-zooms")
        .outline()
        .compact()
        .with_size(Size::Small)
        .label("Generate auto zooms")
        .tooltip("Generate zoom regions from cursor activity")
        .on_click(cx.listener(|view, _, _, cx| view.generate_auto_zooms(cx)));

    let Some(index) = view.selected_zoom_region else {
        return v_flex()
            .gap_2()
            .p_2()
            .border_b_1()
            .border_color(border)
            .child(section_title("Zoom"))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("No region selected"),
            )
            .child(add_button)
            .child(auto_zoom_button)
            .into_any_element();
    };
    let Some(region) = view.project_settings.zoom_regions.get(index).copied() else {
        return v_flex()
            .gap_2()
            .p_2()
            .border_b_1()
            .border_color(border)
            .child(section_title("Zoom"))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("No region selected"),
            )
            .child(add_button)
            .child(auto_zoom_button)
            .into_any_element();
    };

    let cursor_selected = region.target == ZoomTarget::Cursor;
    let center_selected = region.target == ZoomTarget::CanvasCenter;
    let cursor_button = Button::new("zoom-target-cursor")
        .outline()
        .compact()
        .with_size(Size::Small)
        .label(if cursor_selected {
            "✓ Cursor"
        } else {
            "Cursor"
        })
        .when(cursor_selected, |button| button.primary())
        .on_click(cx.listener(|view, _, _, cx| {
            view.set_selected_zoom_target(ZoomTarget::Cursor, cx);
        }));
    let center_button = Button::new("zoom-target-center")
        .outline()
        .compact()
        .with_size(Size::Small)
        .label(if center_selected {
            "✓ Canvas center"
        } else {
            "Canvas center"
        })
        .when(center_selected, |button| button.primary())
        .on_click(cx.listener(|view, _, _, cx| {
            view.set_selected_zoom_target(ZoomTarget::CanvasCenter, cx);
        }));
    let scale_down = Button::new("zoom-scale-down")
        .outline()
        .compact()
        .with_size(Size::Small)
        .label("−")
        .on_click(cx.listener(|view, _, _, cx| {
            view.adjust_selected_zoom_scale(-0.1, cx);
        }));
    let scale_up = Button::new("zoom-scale-up")
        .outline()
        .compact()
        .with_size(Size::Small)
        .label("+")
        .on_click(cx.listener(|view, _, _, cx| {
            view.adjust_selected_zoom_scale(0.1, cx);
        }));
    let delete_button = Button::new("delete-zoom-region")
        .ghost()
        .compact()
        .with_size(Size::Small)
        .label("Delete")
        .on_click(cx.listener(|view, _, _, cx| view.delete_selected_zoom_region(cx)));

    v_flex()
        .gap_2()
        .p_2()
        .border_b_1()
        .border_color(border)
        .child(section_title("Zoom"))
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
                .child(div().text_xs().child("Scale"))
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .child(scale_down)
                        .child(
                            div()
                                .w(px(44.))
                                .text_xs()
                                .text_center()
                                .child(format!("{:.1}×", region.scale)),
                        )
                        .child(scale_up),
                ),
        )
        .child(div().text_xs().child("Target"))
        .child(h_flex().gap_1().child(cursor_button).child(center_button))
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
        .child(auto_zoom_button)
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
