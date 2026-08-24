use std::mem::size_of;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::{CURSOR_SHOWING, CURSORINFO, GetCursorInfo};
use windows_capture::monitor::Monitor;

use super::clock::RecordingClock;
use super::model::{
    ButtonState, CursorSample, MonitorMetadata, MouseEvent, MouseEventKind, SAMPLE_INTERVAL_US,
};
use super::telemetry::TelemetryWriter;

const START_WAIT: Duration = Duration::from_millis(20);

#[derive(Clone, Copy)]
struct CursorState {
    screen_x: i32,
    screen_y: i32,
    cursor_id: Option<u64>,
    visible: bool,
    buttons: ButtonState,
}

pub(crate) struct CursorTracker {
    stop_sender: mpsc::Sender<()>,
    join: Option<JoinHandle<Result<(), String>>>,
}

impl CursorTracker {
    pub(crate) fn spawn(
        monitor: Monitor,
        clock: Arc<RecordingClock>,
        start_receiver: mpsc::Receiver<()>,
        telemetry_path: PathBuf,
    ) -> Result<Self, String> {
        let metadata = monitor_metadata(&monitor)?;
        let (stop_sender, stop_receiver) = mpsc::channel();
        let join = thread::Builder::new()
            .name("cursor-telemetry".to_string())
            .spawn(move || {
                track(
                    metadata,
                    clock,
                    start_receiver,
                    stop_receiver,
                    telemetry_path,
                )
            })
            .map_err(|error| format!("failed to start cursor tracker: {error}"))?;

        Ok(Self {
            stop_sender,
            join: Some(join),
        })
    }

    pub(crate) fn stop(mut self) -> Result<(), String> {
        let _ = self.stop_sender.send(());
        self.join
            .take()
            .expect("cursor tracker join handle missing")
            .join()
            .map_err(|_| "cursor tracker panicked".to_string())?
    }
}

impl Drop for CursorTracker {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = self.stop_sender.send(());
            let _ = join.join();
        }
    }
}

fn track(
    metadata: MonitorMetadata,
    clock: Arc<RecordingClock>,
    start_receiver: mpsc::Receiver<()>,
    stop_receiver: mpsc::Receiver<()>,
    telemetry_path: PathBuf,
) -> Result<(), String> {
    let monitor = metadata.clone();
    let mut telemetry = TelemetryWriter::new(&telemetry_path, metadata)?;
    if !wait_for_video_start(&start_receiver, &stop_receiver) {
        return telemetry.finish();
    }

    let mut previous_buttons = None;
    let tracking_result = loop {
        let state = match read_cursor_state() {
            Ok(state) => state,
            Err(error) => break Err(error),
        };
        let timestamp_us = match clock.timestamp_us() {
            Some(timestamp_us) => timestamp_us,
            None => break Err("cursor clock was not started".to_string()),
        };
        let sample = make_sample(timestamp_us, state, &monitor);

        if let Some(previous) = previous_buttons
            && let Err(error) =
                append_button_events(&mut telemetry, previous, state.buttons, &sample)
        {
            break Err(error);
        }
        previous_buttons = Some(state.buttons);
        if let Err(error) = telemetry.write_sample(sample) {
            break Err(error);
        }

        match stop_receiver.recv_timeout(Duration::from_micros(SAMPLE_INTERVAL_US)) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };

    let finish_result = telemetry.finish();
    match (tracking_result, finish_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn wait_for_video_start(
    start_receiver: &mpsc::Receiver<()>,
    stop_receiver: &mpsc::Receiver<()>,
) -> bool {
    loop {
        match stop_receiver.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => return false,
            Err(mpsc::TryRecvError::Empty) => {}
        }

        match start_receiver.recv_timeout(START_WAIT) {
            Ok(()) => return true,
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn monitor_metadata(monitor: &Monitor) -> Result<MonitorMetadata, String> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let hmonitor = HMONITOR(monitor.as_raw_hmonitor());
    if !unsafe { GetMonitorInfoW(hmonitor, &mut info).as_bool() } {
        return Err("failed to query selected monitor bounds".to_string());
    }

    let width = info.rcMonitor.right.saturating_sub(info.rcMonitor.left);
    let height = info.rcMonitor.bottom.saturating_sub(info.rcMonitor.top);
    if width <= 0 || height <= 0 {
        return Err("selected monitor has invalid bounds".to_string());
    }

    Ok(MonitorMetadata {
        device_name: monitor.device_name().map_err(|error| error.to_string())?,
        origin_x: info.rcMonitor.left,
        origin_y: info.rcMonitor.top,
        width: width as u32,
        height: height as u32,
    })
}

fn read_cursor_state() -> Result<CursorState, String> {
    let mut info = CURSORINFO {
        cbSize: size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetCursorInfo(&mut info) }.map_err(|error| error.to_string())?;

    let cursor_id = (!info.hCursor.0.is_null()).then_some(info.hCursor.0 as usize as u64);
    Ok(CursorState {
        screen_x: info.ptScreenPos.x,
        screen_y: info.ptScreenPos.y,
        cursor_id,
        visible: info.flags.0 & CURSOR_SHOWING.0 != 0,
        buttons: read_buttons(),
    })
}

fn read_buttons() -> ButtonState {
    ButtonState {
        left: key_is_down(VK_LBUTTON.0),
        right: key_is_down(VK_RBUTTON.0),
        middle: key_is_down(VK_MBUTTON.0),
    }
}

fn key_is_down(key: u16) -> bool {
    unsafe { GetAsyncKeyState(i32::from(key)) < 0 }
}

fn make_sample(timestamp_us: u64, state: CursorState, monitor: &MonitorMetadata) -> CursorSample {
    let x = state.screen_x - monitor.origin_x;
    let y = state.screen_y - monitor.origin_y;
    CursorSample {
        timestamp_us,
        x: x as f32,
        y: y as f32,
        normalized_x: x as f32 / monitor.width as f32,
        normalized_y: y as f32 / monitor.height as f32,
        screen_x: state.screen_x,
        screen_y: state.screen_y,
        cursor_id: state.cursor_id,
        visible: state.visible,
        buttons: state.buttons,
    }
}

fn append_button_events(
    telemetry: &mut TelemetryWriter,
    previous: ButtonState,
    current: ButtonState,
    sample: &CursorSample,
) -> Result<(), String> {
    if previous.left != current.left {
        push_event(
            telemetry,
            sample,
            current.left,
            MouseEventKind::LeftDown,
            MouseEventKind::LeftUp,
        )?;
    }
    if previous.right != current.right {
        push_event(
            telemetry,
            sample,
            current.right,
            MouseEventKind::RightDown,
            MouseEventKind::RightUp,
        )?;
    }
    if previous.middle != current.middle {
        push_event(
            telemetry,
            sample,
            current.middle,
            MouseEventKind::MiddleDown,
            MouseEventKind::MiddleUp,
        )?;
    }
    Ok(())
}

fn push_event(
    telemetry: &mut TelemetryWriter,
    sample: &CursorSample,
    is_down: bool,
    down_kind: MouseEventKind,
    up_kind: MouseEventKind,
) -> Result<(), String> {
    telemetry.write_event(MouseEvent {
        timestamp_us: sample.timestamp_us,
        x: sample.x,
        y: sample.y,
        normalized_x: sample.normalized_x,
        normalized_y: sample.normalized_y,
        screen_x: sample.screen_x,
        screen_y: sample.screen_y,
        kind: if is_down { down_kind } else { up_kind },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_writes_samples_after_video_start() {
        let monitor = Monitor::primary().expect("a primary monitor is required");
        let clock = Arc::new(RecordingClock::new());
        let (start_sender, start_receiver) = mpsc::channel();
        let path = std::env::temp_dir().join(format!(
            "recorder-telemetry-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let tracker = CursorTracker::spawn(monitor, clock.clone(), start_receiver, path.clone())
            .expect("cursor tracker should start");

        clock.mark_video_start();
        start_sender
            .send(())
            .expect("video start signal should send");
        thread::sleep(Duration::from_millis(30));
        tracker.stop().expect("cursor tracker should stop");

        let json = std::fs::read_to_string(&path).expect("telemetry should be written");
        let records: Vec<_> = json
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect();
        assert!(records.iter().any(|record| record["type"] == "sample"));
        assert_eq!(records.first().unwrap()["type"], "header");
        assert_eq!(records.last().unwrap()["type"], "footer");
        std::fs::remove_file(path).unwrap();
    }
}
