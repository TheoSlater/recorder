use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::anyhow;
use crossbeam_channel::{Receiver, Sender, bounded};
use windows_capture::capture::{Context as CaptureContext, GraphicsCaptureApiHandler};
use windows_capture::encoder::VideoEncoder;
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use super::encoder::{EncoderPump, FrameTask, QueueStats, new_encoder};
use super::input::{CursorTracker, RecordingClock};
use super::model::{FRAME_INTERVAL, WorkerEvent};
use super::session::SessionPaths;

const WORKER_POLL: Duration = Duration::from_millis(100);

type CaptureError = Box<dyn std::error::Error + Send + Sync>;

struct CaptureFlags {
    width: u32,
    height: u32,
    video_path: PathBuf,
    clock: Arc<RecordingClock>,
    start_sender: mpsc::Sender<()>,
    encoder_sender: Sender<VideoEncoder>,
    frame_sender: Sender<FrameTask>,
    stopping: Arc<AtomicBool>,
    stats: Arc<QueueStats>,
}

struct Capture {
    clock: Arc<RecordingClock>,
    start_sender: mpsc::Sender<()>,
    frame_sender: Sender<FrameTask>,
    stopping: Arc<AtomicBool>,
    stats: Arc<QueueStats>,
    video_started: bool,
}

impl GraphicsCaptureApiHandler for Capture {
    type Flags = CaptureFlags;
    type Error = CaptureError;

    fn new(ctx: CaptureContext<Self::Flags>) -> Result<Self, Self::Error> {
        let encoder = new_encoder(ctx.flags.width, ctx.flags.height, &ctx.flags.video_path)
            .map_err(|error| anyhow!(error))?;
        ctx.flags
            .encoder_sender
            .send(encoder)
            .map_err(|_| anyhow!("encoder owner stopped before startup"))?;

        Ok(Self {
            clock: ctx.flags.clock,
            start_sender: ctx.flags.start_sender,
            frame_sender: ctx.flags.frame_sender,
            stopping: ctx.flags.stopping,
            stats: ctx.flags.stats,
            video_started: false,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.stopping.load(Ordering::Acquire) {
            return Ok(());
        }

        let accepted = EncoderPump::submit(&self.frame_sender, frame, &self.stats)
            .map_err(|error| anyhow!(error))?;
        if accepted && !self.video_started {
            self.clock.mark_video_start();
            let _ = self.start_sender.send(());
            self.video_started = true;
        }

        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        self.stopping.store(true, Ordering::Release);
        Ok(())
    }
}

type CaptureHandle = windows_capture::capture::CaptureControl<Capture, CaptureError>;

struct CaptureResources {
    control: Option<CaptureHandle>,
    cursor_tracker: Option<CursorTracker>,
    pump: Option<EncoderPump>,
    stopping: Arc<AtomicBool>,
}

impl CaptureResources {
    fn new(control: CaptureHandle, stopping: Arc<AtomicBool>) -> Self {
        Self {
            control: Some(control),
            cursor_tracker: None,
            pump: None,
            stopping,
        }
    }

    fn set_pump(&mut self, pump: EncoderPump) {
        self.pump = Some(pump);
    }

    fn set_cursor_tracker(&mut self, cursor_tracker: CursorTracker) {
        self.cursor_tracker = Some(cursor_tracker);
    }

    fn control(&self) -> &CaptureHandle {
        self.control
            .as_ref()
            .expect("capture control should be active")
    }

    fn pump(&self) -> &EncoderPump {
        self.pump.as_ref().expect("encoder pump should be active")
    }

    fn finish(mut self) -> Result<(), String> {
        combine_results(self.stop_all())
    }

    fn stop_all(&mut self) -> [Result<(), String>; 3] {
        self.stopping.store(true, Ordering::Release);
        let capture = self
            .control
            .take()
            .map(|control| control.stop().map_err(|error| error.to_string()))
            .unwrap_or(Ok(()));
        let telemetry = self
            .cursor_tracker
            .take()
            .map(|tracker| tracker.stop())
            .unwrap_or(Ok(()));
        let encoder = self.pump.take().map(|pump| pump.finish()).unwrap_or(Ok(()));
        [telemetry, capture, encoder]
    }
}

impl Drop for CaptureResources {
    fn drop(&mut self) {
        let _ = self.stop_all();
    }
}

pub(crate) fn spawn_capture_worker(
    monitor: Monitor,
    width: u32,
    height: u32,
    session: SessionPaths,
    stop_receiver: Receiver<()>,
    event_sender: Sender<WorkerEvent>,
    done_sender: Sender<()>,
) {
    let fallback_events = event_sender.clone();
    let fallback_done = done_sender.clone();
    let fallback_session = session.clone();
    let worker = thread::Builder::new()
        .name("capture-worker".to_string())
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                run_capture_worker(
                    monitor,
                    width,
                    height,
                    session.clone(),
                    stop_receiver,
                    &event_sender,
                )
            }))
            .unwrap_or_else(|_| Err("capture worker panicked".to_string()));

            let (result, dropped_frames) = match outcome {
                Ok(outcome) => (outcome.result, outcome.dropped_frames),
                Err(error) => (Err(error), 0),
            };
            let result = finish_session(&session, result, dropped_frames);
            let _ = event_sender.send(WorkerEvent::Finished(result));
            let _ = done_sender.send(());
        });

    if let Err(error) = worker {
        let result = finish_session(
            &fallback_session,
            Err(format!("failed to start capture worker: {error}")),
            0,
        );
        let _ = fallback_events.send(WorkerEvent::Finished(result));
        let _ = fallback_done.send(());
    }
}

struct WorkerOutcome {
    result: Result<(), String>,
    dropped_frames: u64,
}

fn run_capture_worker(
    monitor: Monitor,
    width: u32,
    height: u32,
    session: SessionPaths,
    stop_receiver: Receiver<()>,
    event_sender: &Sender<WorkerEvent>,
) -> Result<WorkerOutcome, String> {
    let clock = Arc::new(RecordingClock::new());
    let (start_sender, start_receiver) = mpsc::channel();
    let (frame_sender, frame_receiver) = super::encoder::channel();
    let (encoder_sender, encoder_receiver) = bounded(1);
    let stopping = Arc::new(AtomicBool::new(false));
    let stats = Arc::new(QueueStats::default());
    let flags = CaptureFlags {
        width,
        height,
        video_path: session.video_path().to_path_buf(),
        clock: clock.clone(),
        start_sender,
        encoder_sender,
        frame_sender,
        stopping: stopping.clone(),
        stats: stats.clone(),
    };
    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Custom(FRAME_INTERVAL),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    );
    let control = Capture::start_free_threaded(settings).map_err(|error| error.to_string())?;
    let mut resources = CaptureResources::new(control, stopping.clone());
    let encoder = encoder_receiver
        .recv()
        .map_err(|_| "encoder did not start".to_string())?;
    let pump = EncoderPump::new(encoder, frame_receiver)?;
    resources.set_pump(pump);
    let cursor_tracker = CursorTracker::spawn(
        monitor,
        clock,
        start_receiver,
        session.telemetry_path().to_path_buf(),
    )?;
    resources.set_cursor_tracker(cursor_tracker);
    let _ = event_sender.send(WorkerEvent::Started);

    let stop_reason = wait_for_stop(resources.control(), resources.pump(), stop_receiver);
    if stop_reason == StopReason::Unexpected {
        let _ = event_sender.send(WorkerEvent::CaptureStopped);
    }

    let result = resources.finish();
    let result = if stop_reason == StopReason::Unexpected {
        mark_unexpected(result)
    } else {
        result
    };
    Ok(WorkerOutcome {
        result,
        dropped_frames: stats.dropped(),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StopReason {
    Requested,
    Unexpected,
}

fn wait_for_stop(
    control: &CaptureHandle,
    pump: &EncoderPump,
    stop_receiver: Receiver<()>,
) -> StopReason {
    loop {
        match stop_receiver.recv_timeout(WORKER_POLL) {
            Ok(()) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                return StopReason::Requested;
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout)
                if control.is_finished() || pump.is_finished() =>
            {
                return StopReason::Unexpected;
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn mark_unexpected(result: Result<(), String>) -> Result<(), String> {
    match result {
        Ok(()) => Err("capture worker stopped unexpectedly".to_string()),
        Err(error) => Err(format!("capture worker stopped unexpectedly: {error}")),
    }
}

fn combine_results(results: [Result<(), String>; 3]) -> Result<(), String> {
    let errors: Vec<_> = results.into_iter().filter_map(Result::err).collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn finish_session(
    session: &SessionPaths,
    result: Result<(), String>,
    dropped_frames: u64,
) -> Result<(), String> {
    match session.complete(&result, dropped_frames) {
        Ok(()) => result,
        Err(error) => match result {
            Ok(()) => Err(error),
            Err(recording_error) => Err(format!("{recording_error}; {error}")),
        },
    }
}
