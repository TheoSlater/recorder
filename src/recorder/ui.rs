use crossbeam_channel::{Sender, bounded, unbounded};
use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex, v_flex};

use super::capture::spawn_capture_worker;
use super::components::{monitor_button, start_button, stop_button};
use super::hooks::watch_worker;
use super::model::{MonitorInfo, OUTPUT_PATH, RecorderState, WorkerEvent};

pub(crate) struct RecorderView {
    monitors: Vec<MonitorInfo>,
    selected_monitor: usize,
    state: RecorderState,
    status: SharedString,
    stop_sender: Option<Sender<()>>,
}

impl RecorderView {
    pub(crate) fn new(monitors: Result<Vec<MonitorInfo>, String>) -> Self {
        match monitors {
            Ok(monitors) => Self {
                monitors,
                selected_monitor: 0,
                state: RecorderState::Idle,
                status: "Ready to record".into(),
                stop_sender: None,
            },
            Err(error) => Self {
                monitors: Vec::new(),
                selected_monitor: 0,
                state: RecorderState::Idle,
                status: format!("Monitor enumeration failed: {error}").into(),
                stop_sender: None,
            },
        }
    }

    pub(crate) fn select_monitor(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.state != RecorderState::Idle || index >= self.monitors.len() {
            return;
        }

        self.selected_monitor = index;
        let monitor = &self.monitors[index];
        self.status = format!("Ready: {} × {}", monitor.width, monitor.height).into();
        cx.notify();
    }

    pub(crate) fn start_recording(&mut self, cx: &mut Context<Self>) {
        if self.state != RecorderState::Idle {
            return;
        }

        let Some(monitor) = self.monitors.get(self.selected_monitor).cloned() else {
            self.status = "No monitor is available to record".into();
            cx.notify();
            return;
        };

        let (stop_sender, stop_receiver) = bounded(1);
        let (event_sender, event_receiver) = unbounded();

        self.stop_sender = Some(stop_sender);
        self.state = RecorderState::Starting;
        self.status = format!("Starting {} × {} capture…", monitor.width, monitor.height).into();

        spawn_capture_worker(
            monitor.monitor,
            monitor.width,
            monitor.height,
            stop_receiver,
            event_sender,
        );
        watch_worker(event_receiver, cx);
        cx.notify();
    }

    pub(crate) fn stop_recording(&mut self, cx: &mut Context<Self>) {
        if !matches!(
            self.state,
            RecorderState::Starting | RecorderState::Recording
        ) {
            return;
        }

        if let Some(sender) = &self.stop_sender {
            let _ = sender.send(());
            self.state = RecorderState::Stopping;
            self.status = "Finishing recording…".into();
            cx.notify();
        }
    }

    pub(crate) fn apply_worker_event(&mut self, event: WorkerEvent, cx: &mut Context<Self>) {
        match event {
            WorkerEvent::Started => {
                if self.state == RecorderState::Starting {
                    self.state = RecorderState::Recording;
                    self.status = "Recording…".into();
                }
            }
            WorkerEvent::Finished(result) => {
                self.stop_sender = None;
                self.state = RecorderState::Idle;
                self.status = match result {
                    Ok(()) => format!("Saved {OUTPUT_PATH}").into(),
                    Err(error) => format!("Recording failed: {error}").into(),
                };
            }
        }

        cx.notify();
    }
}

impl Render for RecorderView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_select_monitor = self.state == RecorderState::Idle;
        let can_start = can_select_monitor && !self.monitors.is_empty();
        let can_stop = matches!(
            self.state,
            RecorderState::Starting | RecorderState::Recording
        );

        let monitor_buttons: Vec<_> = self
            .monitors
            .iter()
            .enumerate()
            .map(|(index, monitor)| {
                monitor_button(
                    index,
                    monitor,
                    index == self.selected_monitor,
                    can_select_monitor,
                    cx,
                )
            })
            .collect();

        let selected_dimensions = self
            .monitors
            .get(self.selected_monitor)
            .map(|monitor| format!("{} × {}", monitor.width, monitor.height))
            .unwrap_or_else(|| "Unavailable".to_string());

        div().size_full().bg(cx.theme().background).p_6().child(
            v_flex()
                .gap_4()
                .size_full()
                .child(div().text_xl().child("Screen Recorder"))
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .child("Monitor"),
                        )
                        .child(h_flex().gap_2().children(monitor_buttons))
                        .child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("Selected resolution: {selected_dimensions}")),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(start_button(can_start, cx))
                        .child(stop_button(can_stop, cx)),
                )
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.status.clone()),
                )
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("Output: {OUTPUT_PATH}")),
                ),
        )
    }
}
