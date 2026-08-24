use std::sync::OnceLock;

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// Recording timestamps use one process-independent Windows QPC timebase.
/// Zero is the QPC reading taken after the first frame is accepted by the encoder.
/// The video track uses Windows Graphics Capture's QPC-derived frame timestamps;
/// telemetry converts this same clock to microseconds.
#[derive(Debug)]
pub(crate) struct RecordingClock {
    video_start: OnceLock<i64>,
    frequency: i64,
}

impl RecordingClock {
    pub(crate) fn new() -> Self {
        let mut frequency = 0;
        unsafe { QueryPerformanceFrequency(&mut frequency) }
            .expect("Windows QPC frequency should be available");
        Self {
            video_start: OnceLock::new(),
            frequency,
        }
    }

    pub(crate) fn mark_video_start(&self) -> bool {
        self.video_start.set(self.now()).is_ok()
    }

    pub(crate) fn timestamp_us(&self) -> Option<u64> {
        self.video_start.get().map(|start| {
            let elapsed = self.now().saturating_sub(*start).max(0) as i128;
            (elapsed * 1_000_000 / i128::from(self.frequency)) as u64
        })
    }

    fn now(&self) -> i64 {
        let mut counter = 0;
        unsafe { QueryPerformanceCounter(&mut counter) }
            .expect("Windows QPC counter should be available");
        counter
    }
}
