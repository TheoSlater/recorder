use std::time::Duration;

use gpui::SharedString;
use windows_capture::monitor::Monitor;

pub(crate) const OUTPUT_PATH: &str = "recording.mp4";
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
    Finished(Result<(), String>),
}
