use gpui::{
    Context, Div, FontWeight, InteractiveElement as _, ParentElement as _, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};

use super::model::{MonitorInfo, RecorderState};
use super::ui::RecorderView;

/// The single primary action: starts when idle, stops while recording.
pub(super) fn record_button(
    state: RecorderState,
    can_start: bool,
    cx: &mut Context<RecorderView>,
) -> Button {
    let active = matches!(state, RecorderState::Recording | RecorderState::Stopping);
    let enabled = match state {
        RecorderState::Idle => can_start,
        RecorderState::Recording => true,
        RecorderState::Starting | RecorderState::Stopping => false,
    };
    let label = match state {
        RecorderState::Idle => "Start Recording",
        RecorderState::Starting => "Starting.",
        RecorderState::Recording => "Stop Recording",
        RecorderState::Stopping => "Finishing.",
    };
    let glyph_color = if active {
        cx.theme().danger_foreground
    } else {
        cx.theme().primary_foreground
    };
    let glyph = if active {
        div().size(px(9.)).rounded(px(2.)).bg(glyph_color)
    } else {
        div().size(px(9.)).rounded_full().bg(glyph_color)
    };

    let button = Button::new(if active {
        "stop-recording"
    } else {
        "start-recording"
    });
    let button = if active {
        button.danger()
    } else {
        button.primary()
    };
    button
        .w_full()
        .disabled(!enabled)
        .on_click(cx.listener(move |view, _, _, cx| {
            if active {
                view.stop_recording(cx);
            } else {
                view.start_recording(cx);
            }
        }))
        .child(
            h_flex()
                .items_center()
                .justify_center()
                .gap_2()
                .child(glyph)
                .child(label),
        )
}

/// A selectable capture-source item showing its name with resolution below.
pub(super) fn monitor_card(
    index: usize,
    monitor: &MonitorInfo,
    selected: bool,
    enabled: bool,
    cx: &mut Context<RecorderView>,
) -> Stateful<Div> {
    let hover_bg = cx.theme().muted;
    let accent = cx.theme().accent;
    div()
        .id(format!("monitor-{index}"))
        .flex_1()
        .min_w(px(150.))
        .rounded_md()
        .px_3()
        .py_2()
        .when(selected, |card| card.bg(cx.theme().accent.opacity(0.15)))
        .when(!selected && enabled, |card| {
            card.cursor_pointer().hover(move |style| style.bg(hover_bg))
        })
        .when(!enabled, |card| card.opacity(0.65))
        .on_click(cx.listener(move |view, _, _, cx| view.select_monitor(index, cx)))
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(monitor.label.clone()),
                        )
                        .child(resolution_label(monitor.width, monitor.height, cx)),
                )
                .when(selected, |row| {
                    row.child(
                        Icon::new(IconName::Check)
                            .with_size(Size::Small)
                            .text_color(accent),
                    )
                }),
        )
}

pub(super) fn resolution_label(width: u32, height: u32, cx: &Context<RecorderView>) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(format!("{width} × {height}"))
}

pub(super) fn refresh_projects_button(enabled: bool, cx: &mut Context<RecorderView>) -> Button {
    Button::new("refresh-projects")
        .ghost()
        .compact()
        .with_size(Size::Small)
        .disabled(!enabled)
        .label("Refresh")
        .on_click(cx.listener(|view, _, _, cx| view.refresh_projects(cx)))
}

pub(super) fn refresh_windows_button(enabled: bool, cx: &mut Context<RecorderView>) -> Button {
    Button::new("refresh-windows")
        .ghost()
        .compact()
        .with_size(Size::Small)
        .disabled(!enabled)
        .label("Refresh")
        .on_click(cx.listener(|view, _, _, cx| view.refresh_windows(cx)))
}
