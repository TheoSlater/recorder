use std::time::Duration;

use gpui::SharedString;
use windows_capture::monitor::Monitor;

pub(crate) const RECORDINGS_DIR: &str = "recordings";
pub(crate) const VIDEO_FILE: &str = "recording.mp4";
pub(crate) const TELEMETRY_FILE: &str = "telemetry.jsonl";
pub(crate) const SESSION_FILE: &str = "session.json";
pub(crate) const RECORDING_TIMEBASE: &str = "windows_qpc";
pub(crate) const RECORDING_ZERO: &str = "first_accepted_video_frame";
pub(crate) const CURSOR_CAPTURE: &str = "excluded";
pub(crate) const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);

#[derive(Clone)]
pub(crate) struct MonitorInfo {
    pub(crate) monitor: Monitor,
    pub(crate) label: SharedString,
    pub(crate) width: u32,
    pub(crate) height: u32,
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
