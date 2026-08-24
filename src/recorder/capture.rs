use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::thread;

use anyhow::anyhow;
use crossbeam_channel::{Receiver, Sender};
use windows_capture::capture::{Context as CaptureContext, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
    VideoSettingsSubType,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use super::model::{FRAME_INTERVAL, OUTPUT_PATH, WorkerEvent};

type CaptureError = Box<dyn std::error::Error + Send + Sync>;

struct Capture {
    encoder: Option<VideoEncoder>,
}

impl Capture {
    fn finish(&mut self) -> Result<(), CaptureError> {
        if let Some(encoder) = self.encoder.take() {
            encoder.finish()?;
        }

        Ok(())
    }
}

impl GraphicsCaptureApiHandler for Capture {
    type Flags = (u32, u32);
    type Error = CaptureError;

    fn new(ctx: CaptureContext<Self::Flags>) -> Result<Self, Self::Error> {
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(ctx.flags.0, ctx.flags.1)
                .sub_type(VideoSettingsSubType::H264)
                .frame_rate(60),
            AudioSettingsBuilder::default().disabled(true),
            ContainerSettingsBuilder::default(),
            Path::new(OUTPUT_PATH),
        )?;

        Ok(Self {
            encoder: Some(encoder),
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        self.encoder
            .as_mut()
            .ok_or_else(|| anyhow!("encoder was already finalized"))?
            .send_frame(frame)?;

        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        self.finish()
    }
}

pub(crate) fn spawn_capture_worker(
    monitor: Monitor,
    width: u32,
    height: u32,
    stop_receiver: Receiver<()>,
    event_sender: Sender<WorkerEvent>,
) {
    thread::spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| {
            run_capture_worker(monitor, width, height, stop_receiver, &event_sender)
        }))
        .unwrap_or_else(|_| Err("capture worker panicked".to_string()));

        let _ = event_sender.send(WorkerEvent::Finished(result));
    });
}

fn run_capture_worker(
    monitor: Monitor,
    width: u32,
    height: u32,
    stop_receiver: Receiver<()>,
    event_sender: &Sender<WorkerEvent>,
) -> Result<(), String> {
    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::Default,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Custom(FRAME_INTERVAL),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        (width, height),
    );

    let control = Capture::start_free_threaded(settings).map_err(|error| error.to_string())?;
    let callback = control.callback();
    let _ = event_sender.send(WorkerEvent::Started);

    let _ = stop_receiver.recv();

    let stop_error = control.stop().err().map(|error| error.to_string());
    let finish_error = callback
        .lock()
        .finish()
        .err()
        .map(|error| error.to_string());

    match stop_error.or(finish_error) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
