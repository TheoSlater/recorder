use crossbeam_channel::bounded;
use gpui::*;
use gpui_component::{ActiveTheme as _, Root, h_flex, v_flex};
use std::sync::Arc;
use std::time::Instant;

use super::capture::spawn_capture_worker;
use super::components::{monitor_button, start_button, stop_button};
use super::hooks::watch_worker;
use super::lifecycle::{RecordingControl, ShutdownCoordinator};
use super::model::{MonitorInfo, RecorderState, WorkerEvent};
use super::overlay;
use super::playback;
use super::session::SessionPaths;

pub(crate) struct RecorderView {
    monitors: Vec<MonitorInfo>,
    selected_monitor: usize,
    state: RecorderState,
    status: SharedString,
    shutdown: Arc<ShutdownCoordinator>,
    recording: Option<RecordingControl>,
    session: Option<SessionPaths>,
    last_output: Option<SharedString>,
    overlay_window: Option<WindowHandle<Root>>,
}

impl RecorderView {
    pub(crate) fn new(
        monitors: Result<Vec<MonitorInfo>, String>,
        shutdown: Arc<ShutdownCoordinator>,
    ) -> Self {
        match monitors {
            Ok(monitors) => Self {
                monitors,
                selected_monitor: 0,
                state: RecorderState::Idle,
                status: "Ready to record".into(),
                shutdown,
                recording: None,
                session: None,
                last_output: None,
                overlay_window: None,
            },
            Err(error) => Self {
                monitors: Vec::new(),
                selected_monitor: 0,
                state: RecorderState::Idle,
                status: format!("Monitor enumeration failed: {error}").into(),
                shutdown,
                recording: None,
                session: None,
                last_output: None,
                overlay_window: None,
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

        let session =
            match SessionPaths::create(monitor.label.as_ref(), monitor.width, monitor.height) {
                Ok(session) => session,
                Err(error) => {
                    self.status = format!("Could not create recording session: {error}").into();
                    cx.notify();
                    return;
                }
            };
        let (stop_sender, stop_receiver) = bounded(1);
        let (event_sender, event_receiver) = bounded(8);
        let (done_sender, done_receiver) = bounded(1);
        let recording = RecordingControl::new(stop_sender, done_receiver);

        self.shutdown.register(recording.clone());
        self.recording = Some(recording);
        self.session = Some(session.clone());
        self.last_output = None;
        self.state = RecorderState::Starting;
        self.status = format!("Starting {} × {} capture…", monitor.width, monitor.height).into();

        spawn_capture_worker(
            monitor.monitor,
            monitor.width,
            monitor.height,
            session,
            stop_receiver,
            event_sender,
            done_sender,
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

        if let Some(recording) = &self.recording {
            recording.request_stop();
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
                    self.open_overlay(cx);
                }
            }
            WorkerEvent::CaptureStopped => {
                if matches!(
                    self.state,
                    RecorderState::Starting | RecorderState::Recording
                ) {
                    self.state = RecorderState::Stopping;
                    self.status = "Capture stopped unexpectedly; finalizing…".into();
                    self.close_overlay(cx);
                }
            }
            WorkerEvent::Finished(result) => {
                self.shutdown.clear();
                self.recording = None;
                self.close_overlay(cx);
                self.state = RecorderState::Idle;
                let session = self.session.take();
                let video_path = session
                    .as_ref()
                    .map(|session| session.video_path().to_path_buf());
                let telemetry_path = session
                    .as_ref()
                    .map(|session| session.telemetry_path().to_path_buf());
                let metadata_path = session
                    .as_ref()
                    .map(|session| session.metadata_path().to_path_buf());
                let output = session.map(|session| {
                    let output = session.directory().display().to_string();
                    self.last_output = Some(output.clone().into());
                    output
                });
                let succeeded = result.is_ok();
                self.status = match result {
                    Ok(()) => format!("Saved {}", output.as_deref().unwrap_or("recording")).into(),
                    Err(error) => format!("Recording failed: {error}").into(),
                };
                if succeeded
                    && let (Some(video_path), Some(telemetry_path), Some(metadata_path)) =
                        (video_path, telemetry_path, metadata_path)
                {
                    self.open_playback(video_path, telemetry_path, metadata_path, cx);
                }
            }
        }

        cx.notify();
    }

    fn open_overlay(&mut self, cx: &mut Context<Self>) {
        if self.overlay_window.is_some() {
            return;
        }

        let Some(monitor) = self.monitors.get(self.selected_monitor) else {
            return;
        };
        let display_id = DisplayId::new(monitor.monitor.as_raw_hmonitor() as u64);

        cx.spawn(async move |view, cx| {
            match overlay::open(cx, view.clone(), display_id, Instant::now()) {
                Ok(handle) => {
                    let keep_open = view
                        .update(cx, |view, _| {
                            if view.state == RecorderState::Recording {
                                view.overlay_window = Some(handle);
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);

                    if !keep_open {
                        handle
                            .update(cx, |_, window, _| window.remove_window())
                            .ok();
                    }
                }
                Err(error) => {
                    view.update(cx, |view, cx| {
                        if view.state == RecorderState::Recording {
                            view.status = format!("Recording… overlay unavailable: {error}").into();
                            cx.notify();
                        }
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    fn close_overlay(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.overlay_window.take() {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
    }

    fn open_playback(
        &mut self,
        video_path: std::path::PathBuf,
        telemetry_path: std::path::PathBuf,
        metadata_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |view, cx| {
            if let Err(error) = playback::open(cx, video_path, telemetry_path, metadata_path) {
                view.update(cx, |view, cx| {
                    view.status = format!("Recording saved, but playback failed: {error}").into();
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
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

        let output = self
            .session
            .as_ref()
            .map(|session| session.directory().display().to_string())
            .or_else(|| self.last_output.as_ref().map(ToString::to_string))
            .unwrap_or_else(|| "recordings/".to_string());

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
                        .child(format!("Output: {output}")),
                ),
        )
    }
}
