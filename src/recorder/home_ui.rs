use std::rc::Rc;

use super::alerts::{self, AlertCloseHandler};
use super::components::{monitor_card, record_button, refresh_windows_button, resolution_label};
use super::model::{CaptureSourceKind, RecorderState};
use super::project_ui::render_saved_projects;
use super::ui::RecorderView;
use gpui::{
    AnyElement, Context, FontWeight, Hsla, IntoElement, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable as _, Size,
    button::Button,
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    tab::{Tab, TabBar},
    v_flex,
};

const SOURCE_KINDS: [CaptureSourceKind; 2] =
    [CaptureSourceKind::Monitor, CaptureSourceKind::Window];

pub(super) fn render(view: &mut RecorderView, cx: &mut Context<RecorderView>) -> impl IntoElement {
    let can_select_source = view.state == RecorderState::Idle;
    let can_start = can_select_source
        && match view.source_kind {
            CaptureSourceKind::Monitor => !view.monitors.is_empty(),
            CaptureSourceKind::Window => !view.windows.is_empty(),
        };

    let output = view
        .session
        .as_ref()
        .map(|session| session.directory().display().to_string())
        .or_else(|| view.last_output.as_ref().map(ToString::to_string))
        .unwrap_or_else(|| "recordings/".to_string());
    let status_color = if view.status_error {
        cx.theme().danger
    } else {
        status_color(view.state, cx)
    };
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

    div()
        .relative()
        .size_full()
        .bg(cx.theme().background)
        .text_color(cx.theme().foreground)
        .child(
            v_flex()
                .size_full()
                .gap_3()
                .p_4()
                .child(render_header(view.state, status_color, cx))
                .child(render_capture_section(view, can_select_source, cx))
                .child(record_button(view.state, can_start, cx))
                .child(render_status_row(
                    &view.status,
                    &output,
                    view.status_error,
                    status_color,
                    cx,
                ))
                .child(render_saved_projects(&view.projects, can_select_source, cx)),
        )
        .children(alert_layer)
}

fn render_header(
    state: RecorderState,
    status_color: Hsla,
    cx: &Context<RecorderView>,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            v_flex()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Screen Recorder"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child("Capture a monitor or window"),
                ),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .gap_1p5()
                .child(div().size(px(6.)).rounded_full().bg(status_color))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(status_color)
                        .child(status_label(state)),
                ),
        )
}

fn render_capture_section(
    view: &RecorderView,
    can_select: bool,
    cx: &mut Context<RecorderView>,
) -> AnyElement {
    v_flex()
        .w_full()
        .gap_2()
        .child(section_label("Capture"))
        .child(source_tabs(view.source_kind, can_select, cx))
        .when(view.source_kind == CaptureSourceKind::Monitor, |this| {
            this.child(render_monitors(view, can_select, cx))
        })
        .when(view.source_kind == CaptureSourceKind::Window, |this| {
            this.child(render_window_selector(view, can_select, cx))
        })
        .into_any_element()
}

fn source_tabs(
    selected: CaptureSourceKind,
    enabled: bool,
    cx: &mut Context<RecorderView>,
) -> TabBar {
    TabBar::new("capture-source")
        .segmented()
        .with_size(Size::Small)
        .selected_index(source_index(selected))
        .on_click(cx.listener(move |view, index: &usize, _, cx| {
            if let Some(kind) = SOURCE_KINDS.get(*index) {
                view.select_source(*kind, cx);
            }
        }))
        .children(SOURCE_KINDS.map(|kind| Tab::new().label(kind.label()).disabled(!enabled)))
}

fn source_index(kind: CaptureSourceKind) -> usize {
    SOURCE_KINDS
        .iter()
        .position(|candidate| *candidate == kind)
        .unwrap_or(0)
}

fn render_monitors(
    view: &RecorderView,
    can_select: bool,
    cx: &mut Context<RecorderView>,
) -> AnyElement {
    if view.monitors.is_empty() {
        return muted_hint("No monitors detected.", cx);
    }

    h_flex()
        .w_full()
        .flex_wrap()
        .gap_2()
        .children(view.monitors.iter().enumerate().map(|(index, monitor)| {
            monitor_card(
                index,
                monitor,
                index == view.selected_monitor,
                can_select,
                cx,
            )
        }))
        .into_any_element()
}

fn render_window_selector(
    view: &RecorderView,
    can_select: bool,
    cx: &mut Context<RecorderView>,
) -> AnyElement {
    let window_items: Vec<_> = view
        .windows
        .iter()
        .enumerate()
        .map(|(index, window)| (index, window.label(), index == view.selected_window))
        .collect();
    let selected_label = view
        .windows
        .get(view.selected_window)
        .map(|window| window.label())
        .unwrap_or_else(|| "No capturable windows".into());
    let view_entity = cx.entity().clone();
    let selector = Button::new("window-selector")
        .outline()
        .compact()
        .with_size(Size::Small)
        .flex_1()
        .disabled(!can_select || window_items.is_empty())
        .label(selected_label)
        .dropdown_caret(true)
        .dropdown_menu(move |menu, window, _| {
            if window_items.is_empty() {
                return menu.item(PopupMenuItem::new("No capturable windows").disabled(true));
            }

            window_items
                .iter()
                .fold(menu, |menu, (index, label, selected)| {
                    let index = *index;
                    let view_entity = view_entity.clone();
                    menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(*selected)
                            .on_click(window.listener_for(&view_entity, move |view, _, _, cx| {
                                view.select_window(index, cx);
                            })),
                    )
                })
        });
    let resolution = view
        .windows
        .get(view.selected_window)
        .map(|window| (window.width, window.height));

    v_flex()
        .w_full()
        .gap_1p5()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(selector)
                .child(refresh_windows_button(can_select, cx)),
        )
        .when_some(resolution, |this, (width, height)| {
            this.child(resolution_label(width, height, cx))
        })
        .when_some(view.window_error.clone(), |this, error| {
            this.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .truncate()
                    .child(error),
            )
        })
        .into_any_element()
}

fn render_status_row(
    status: &str,
    output: &str,
    is_error: bool,
    status_color: Hsla,
    cx: &Context<RecorderView>,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .min_w_0()
        .child(
            div()
                .size(px(6.))
                .flex_shrink_0()
                .rounded_full()
                .bg(status_color),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .when(is_error, |label| label.text_color(cx.theme().danger))
                .truncate()
                .child(status.to_string()),
        )
        .child(
            div()
                .flex_shrink_0()
                .max_w(px(280.))
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .whitespace_nowrap()
                .truncate()
                .child(format!("Output: {output}")),
        )
}

fn section_label(text: &'static str) -> impl IntoElement {
    div().text_sm().font_weight(FontWeight::MEDIUM).child(text)
}

fn muted_hint(text: &'static str, cx: &Context<RecorderView>) -> AnyElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

fn status_label(state: RecorderState) -> &'static str {
    match state {
        RecorderState::Idle => "Ready",
        RecorderState::Starting => "Starting",
        RecorderState::Recording => "Recording",
        RecorderState::Stopping => "Finishing",
    }
}

fn status_color(state: RecorderState, cx: &Context<RecorderView>) -> Hsla {
    match state {
        RecorderState::Idle => cx.theme().success,
        RecorderState::Starting | RecorderState::Stopping => cx.theme().primary,
        RecorderState::Recording => cx.theme().danger,
    }
}
