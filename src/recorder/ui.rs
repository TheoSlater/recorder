use crossbeam_channel::bounded;
use gpui::*;
use gpui_component::Root;
use std::sync::Arc;
use std::time::Instant;

use super::alerts::{AlertQueue, AppAlert};
use super::capture::spawn_capture_worker;
use super::home_ui;
use super::hooks::watch_worker;
use super::lifecycle::{RecordingControl, ShutdownCoordinator};
use super::model::{
    CaptureSource, CaptureSourceKind, MonitorInfo, RecorderState, WindowInfo, WorkerEvent,
};
use super::overlay;
use super::playback;
use super::project;
use super::session::{SessionPaths, SessionSource};
use super::windows::enumerate_windows;

pub(crate) struct RecorderView {
    pub(super) monitors: Vec<MonitorInfo>,
    pub(super) selected_monitor: usize,
    pub(super) windows: Vec<WindowInfo>,
    pub(super) selected_window: usize,
    pub(super) source_kind: CaptureSourceKind,
    pub(super) window_error: Option<SharedString>,
    pub(super) state: RecorderState,
    pub(super) status: SharedString,
    /// Whether the current status message reports an error.
    pub(super) status_error: bool,
    shutdown: Arc<ShutdownCoordinator>,
    recording: Option<RecordingControl>,
    recording_source: Option<CaptureSource>,
    pub(super) session: Option<SessionPaths>,
    pub(super) last_output: Option<SharedString>,
    pub(super) pending_alerts: AlertQueue,
    overlay_window: Option<WindowHandle<Root>>,
    pub(super) projects: Vec<project::SavedProject>,
    pub(super) project_refresh_generation: u64,
}

impl RecorderView {
    pub(crate) fn new(
        monitors: Result<Vec<MonitorInfo>, String>,
        windows: Result<Vec<WindowInfo>, String>,
        shutdown: Arc<ShutdownCoordinator>,
    ) -> Self {
        let projects = project::load_projects();
        let (windows, window_error) = match windows {
            Ok(windows) => (windows, None),
            Err(error) => {
                let message = format!("Window enumeration failed: {error}");
                tracing::error!(
                    target: "recorder",
                    error = %message,
                    "window enumeration failed"
                );
                (Vec::new(), Some(message.into()))
            }
        };
        let mut pending_alerts = AlertQueue::default();
        if let Some(error) = window_error.clone() {
            pending_alerts.push(AppAlert::error(error));
        }
        match monitors {
            Ok(monitors) => Self {
                monitors,
                selected_monitor: 0,
                windows,
                selected_window: 0,
                source_kind: CaptureSourceKind::Monitor,
                window_error,
                state: RecorderState::Idle,
                status: "Ready to record".into(),
                status_error: false,
                shutdown,
                recording: None,
                recording_source: None,
                session: None,
                last_output: None,
                pending_alerts,
                overlay_window: None,
                projects,
                project_refresh_generation: 0,
            },
            Err(error) => {
                let message = format!("Monitor enumeration failed: {error}");
                tracing::error!(
                    target: "recorder",
                    error = %message,
                    "monitor enumeration failed"
                );
                pending_alerts.push(AppAlert::error(message.clone()));
                Self {
                    monitors: Vec::new(),
                    selected_monitor: 0,
                    windows,
                    selected_window: 0,
                    source_kind: CaptureSourceKind::Monitor,
                    window_error,
                    state: RecorderState::Idle,
                    status: message.clone().into(),
                    status_error: true,
                    shutdown,
                    recording: None,
                    recording_source: None,
                    session: None,
                    last_output: None,
                    pending_alerts,
                    overlay_window: None,
                    projects,
                    project_refresh_generation: 0,
                }
            }
        }
    }

    pub(crate) fn select_source(&mut self, kind: CaptureSourceKind, cx: &mut Context<Self>) {
        if self.state != RecorderState::Idle {
            return;
        }

        self.source_kind = kind;
        self.set_status(self.source_status());
        cx.notify();
    }

    pub(crate) fn select_monitor(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.state != RecorderState::Idle || index >= self.monitors.len() {
            return;
        }

        self.selected_monitor = index;
        self.source_kind = CaptureSourceKind::Monitor;
        self.set_status(self.source_status());
        cx.notify();
    }

    pub(crate) fn select_window(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.state != RecorderState::Idle || index >= self.windows.len() {
            return;
        }

        self.selected_window = index;
        self.source_kind = CaptureSourceKind::Window;
        self.set_status(self.source_status());
        cx.notify();
    }

    pub(crate) fn refresh_windows(&mut self, cx: &mut Context<Self>) {
        if self.state != RecorderState::Idle {
            return;
        }

        match enumerate_windows() {
            Ok(windows) => {
                let no_windows = windows.is_empty();
                self.windows = windows;
                self.selected_window = self
                    .selected_window
                    .min(self.windows.len().saturating_sub(1));
                self.window_error = None;
                if no_windows {
                    self.report_warning("No capturable windows were found after refresh.", cx);
                }
            }
            Err(error) => {
                let message = format!("Window enumeration failed: {error}");
                self.windows.clear();
                self.selected_window = 0;
                self.window_error = Some(message.clone().into());
                self.report_error(message, cx);
            }
        }
        if self.source_kind == CaptureSourceKind::Window && self.window_error.is_none() {
            self.set_status(self.source_status());
        }
        cx.notify();
    }

    fn source_status(&self) -> String {
        match self.source_kind {
            CaptureSourceKind::Monitor => self
                .monitors
                .get(self.selected_monitor)
                .map(|monitor| format!("Ready: {} × {}", monitor.width, monitor.height))
                .unwrap_or_else(|| "No monitor is available".to_string()),
            CaptureSourceKind::Window => self
                .windows
                .get(self.selected_window)
                .map(|window| format!("Ready: {} × {}", window.width, window.height))
                .or_else(|| self.window_error.as_ref().map(ToString::to_string))
                .unwrap_or_else(|| "No capturable windows found".to_string()),
        }
    }

    pub(crate) fn start_recording(&mut self, cx: &mut Context<Self>) {
        if self.state != RecorderState::Idle {
            return;
        }

        let (source, session_source, width, height) = match self.source_kind {
            CaptureSourceKind::Monitor => {
                let Some(monitor) = self.monitors.get(self.selected_monitor).cloned() else {
                    self.report_error("No monitor is available to record", cx);
                    return;
                };
                (
                    CaptureSource::Monitor(monitor.monitor),
                    SessionSource::monitor(monitor.label.as_ref(), monitor.width, monitor.height),
                    monitor.width,
                    monitor.height,
                )
            }
            CaptureSourceKind::Window => {
                let Some(window) = self.windows.get(self.selected_window).cloned() else {
                    self.report_error("No capturable window is available to record", cx);
                    return;
                };
                if !window.window.is_valid() {
                    self.report_error(
                        "Selected window is no longer available; refresh the window list",
                        cx,
                    );
                    return;
                }
                (
                    CaptureSource::Window(window.window),
                    SessionSource::window(
                        window.title.as_ref(),
                        window.app_name.map(|name| name.to_string()),
                        window.width,
                        window.height,
                    ),
                    window.width,
                    window.height,
                )
            }
        };

        let session = match SessionPaths::create(session_source) {
            Ok(session) => session,
            Err(error) => {
                self.report_error(format!("Could not create recording session: {error}"), cx);
                return;
            }
        };
        let (stop_sender, stop_receiver) = bounded(1);
        let (event_sender, event_receiver) = bounded(8);
        let (done_sender, done_receiver) = bounded(1);
        let recording = RecordingControl::new(stop_sender, done_receiver);

        self.shutdown.register(recording.clone());
        self.recording = Some(recording);
        self.recording_source = Some(source);
        self.session = Some(session.clone());
        self.last_output = None;
        self.state = RecorderState::Starting;
        self.set_status(format!(
            "Starting {} {} × {} capture…",
            self.source_kind.label(),
            width,
            height
        ));

        spawn_capture_worker(
            source,
            width,
            height,
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
            self.set_status("Finishing recording…");
            cx.notify();
        }
    }

    pub(crate) fn apply_worker_event(&mut self, event: WorkerEvent, cx: &mut Context<Self>) {
        match event {
            WorkerEvent::Started => {
                if self.state == RecorderState::Starting {
                    self.state = RecorderState::Recording;
                    self.set_status("Recording…");
                    self.open_overlay(cx);
                }
            }
            WorkerEvent::CaptureStopped => {
                if matches!(
                    self.state,
                    RecorderState::Starting | RecorderState::Recording
                ) {
                    self.state = RecorderState::Stopping;
                    self.set_status("Capture stopped unexpectedly; finalizing…");
                    self.close_overlay(cx);
                }
            }
            WorkerEvent::Finished(result) => {
                self.shutdown.clear();
                self.recording = None;
                self.recording_source = None;
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
                self.refresh_projects_in_background(cx, false);
                match result {
                    Ok(()) => {
                        self.set_status(format!(
                            "Saved {}",
                            output.as_deref().unwrap_or("recording")
                        ));
                    }
                    Err(error) => {
                        self.report_error(format!("Recording failed: {error}"), cx);
                    }
                }
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

        let Some(monitor) = self
            .recording_source
            .as_ref()
            .and_then(|source| source.overlay_monitor())
        else {
            return;
        };
        let display_id = DisplayId::new(monitor.as_raw_hmonitor() as u64);

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
                            view.report_error(
                                format!("Recording… overlay unavailable: {error}"),
                                cx,
                            );
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
        let settings_path = project::settings_path_for(&metadata_path);
        let settings_path_for_load = settings_path.clone();
        cx.spawn(async move |view, cx| {
            let settings = cx
                .background_spawn(async move { project::load_settings(&settings_path_for_load) })
                .await;
            if let Err(error) = playback::open(
                cx,
                video_path,
                telemetry_path,
                metadata_path,
                settings_path,
                settings,
                true,
                false,
            ) {
                view.update(cx, |view, cx| {
                    view.report_error(format!("Recording saved, but playback failed: {error}"), cx);
                })
                .ok();
            }
        })
        .detach();
    }
}

impl RecorderView {
    /// Updates the informational status message, clearing any error emphasis.
    pub(super) fn set_status(&mut self, message: impl Into<SharedString>) {
        self.status = message.into();
        self.status_error = false;
    }

    pub(super) fn report_error(
        &mut self,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let message: SharedString = message.into();
        tracing::error!(
            target: "recorder",
            error = %message,
            "recorder error"
        );
        self.status = message.clone();
        self.status_error = true;
        self.pending_alerts.push(AppAlert::error(message));
        cx.notify();
    }

    pub(super) fn report_warning(
        &mut self,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let message: SharedString = message.into();
        tracing::warn!(
            target: "recorder",
            warning = %message,
            "recorder warning"
        );
        self.pending_alerts.push(AppAlert::warning(message));
        cx.notify();
    }
}

impl Render for RecorderView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        home_ui::render(self, cx)
    }
}
