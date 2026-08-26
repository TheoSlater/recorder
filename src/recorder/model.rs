use std::convert::Infallible;
use std::time::Duration;

use gpui::SharedString;
use windows_capture::{monitor::Monitor, settings::GraphicsCaptureItemType, window::Window};

pub(crate) const RECORDINGS_DIR: &str = "recordings";
pub(crate) const VIDEO_FILE: &str = "recording.mp4";
pub(crate) const TELEMETRY_FILE: &str = "telemetry.jsonl";
pub(crate) const SESSION_FILE: &str = "session.json";
pub(crate) const PROJECT_FILE_EXTENSION: &str = "recproj";
pub(crate) const LEGACY_PROJECT_FILE: &str = "project.json";
pub(crate) const RECORDING_TIMEBASE: &str = "windows_qpc";
pub(crate) const RECORDING_ZERO: &str = "first_accepted_video_frame";
pub(crate) const CURSOR_CAPTURE: &str = "excluded";
pub(crate) const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);

#[derive(Clone, Copy)]
pub(crate) enum CaptureSource {
    Monitor(Monitor),
    Window(Window),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureSourceKind {
    Monitor,
    Window,
}

impl CaptureSourceKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Monitor => "Monitor",
            Self::Window => "Window",
        }
    }
}

impl CaptureSource {
    pub(crate) fn capture_item(self) -> Result<CaptureItem, String> {
        self.try_into()
            .map(CaptureItem)
            .map_err(|error| format!("failed to create capture item: {error}"))
    }

    pub(crate) fn overlay_monitor(self) -> Option<Monitor> {
        match self {
            Self::Monitor(monitor) => Some(monitor),
            Self::Window(window) => window.monitor(),
        }
    }
}

/// Owns the exact WinRT item used to create the frame pool.
///
/// Keeping the item together with the source dimensions avoids using a DPI-
/// virtualized `GetWindowRect` size as the encoder target on mixed-DPI setups.
///
/// # Thread safety
///
/// `windows-capture` leaves this type `!Send` out of caution, but handing it to
/// the capture thread is the intended flow: `start_free_threaded` moves the
/// settings onto its own worker. Handles stay valid process-wide (the crate
/// itself marks `Monitor`/`Window` `Send` for the same reason),
/// `GraphicsCaptureItem` is an agile WinRT object used and released on that one
/// thread, and the picker-backed `Unknown` variant is never constructed here.
pub(crate) struct CaptureItem(GraphicsCaptureItemType);

unsafe impl Send for CaptureItem {}

impl CaptureItem {
    pub(crate) fn dimensions(&self) -> Result<(u32, u32), String> {
        let item = match &self.0 {
            GraphicsCaptureItemType::Monitor((item, _))
            | GraphicsCaptureItemType::Window((item, _))
            | GraphicsCaptureItemType::Unknown((item, _)) => item,
        };
        let size = item
            .Size()
            .map_err(|error| format!("failed to query capture dimensions: {error}"))?;
        let width =
            u32::try_from(size.Width).map_err(|_| "capture width is invalid".to_string())?;
        let height =
            u32::try_from(size.Height).map_err(|_| "capture height is invalid".to_string())?;
        if width == 0 || height == 0 {
            return Err("capture dimensions are invalid".to_string());
        }
        Ok((width, height))
    }
}

impl TryInto<GraphicsCaptureItemType> for CaptureItem {
    type Error = Infallible;

    fn try_into(self) -> Result<GraphicsCaptureItemType, Self::Error> {
        Ok(self.0)
    }
}

impl TryInto<GraphicsCaptureItemType> for CaptureSource {
    type Error = windows::core::Error;

    fn try_into(self) -> Result<GraphicsCaptureItemType, Self::Error> {
        match self {
            Self::Monitor(monitor) => monitor.try_into(),
            Self::Window(window) => window.try_into(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct MonitorInfo {
    pub(crate) monitor: Monitor,
    pub(crate) label: SharedString,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Clone)]
pub(crate) struct WindowInfo {
    pub(crate) window: Window,
    pub(crate) title: SharedString,
    pub(crate) app_name: Option<SharedString>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl WindowInfo {
    pub(crate) fn label(&self) -> SharedString {
        match &self.app_name {
            Some(app_name) if !app_name.is_empty() => {
                format!("{} — {}", self.title, app_name).into()
            }
            _ => self.title.clone(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecorderState {
    Idle,
    Starting,
    Recording,
    Stopping,
}

pub(crate) enum WorkerEvent {
    Started,
    CaptureStopped,
    Finished(Result<(), String>),
}
