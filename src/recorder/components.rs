use gpui::Context;
use gpui_component::{Disableable, Selectable, button::*};

use super::model::MonitorInfo;
use super::ui::RecorderView;

pub(super) fn monitor_button(
    index: usize,
    monitor: &MonitorInfo,
    selected: bool,
    enabled: bool,
    cx: &mut Context<RecorderView>,
) -> Button {
    Button::new(format!("monitor-{index}"))
        .outline()
        .selected(selected)
        .disabled(!enabled)
        .label(monitor.label.clone())
        .on_click(cx.listener(move |view, _, _, cx| view.select_monitor(index, cx)))
}

pub(super) fn start_button(enabled: bool, cx: &mut Context<RecorderView>) -> Button {
    Button::new("start-recording")
        .primary()
        .disabled(!enabled)
        .label("Start Recording")
        .on_click(cx.listener(|view, _, _, cx| view.start_recording(cx)))
}

pub(super) fn stop_button(enabled: bool, cx: &mut Context<RecorderView>) -> Button {
    Button::new("stop-recording")
        .danger()
        .disabled(!enabled)
        .label("Stop Recording")
        .on_click(cx.listener(|view, _, _, cx| view.stop_recording(cx)))
}
