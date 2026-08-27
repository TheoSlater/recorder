use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

use anyhow::{Result, bail};
use crossbeam_channel::Sender;

use super::super::{
    composition::{self, OutputSize},
    cursor::CursorOverlay,
    motion_blur::{
        MotionBlurDescriptor, MotionBlurMode, RecordingTransform, compute_display_motion_blur,
    },
    project_settings::ProjectSettings,
    zoom::cursor_scale_at,
};
use super::{
    ExportEvent, ExportRequest, finalize_temporary, is_cancelled, remove_temporary, send_terminal,
    temporary_path,
};
use super::{
    decoder::{self, Decoder},
    encoder::Encoder,
    frames::Renderer,
};

pub(crate) fn run(
    request: ExportRequest,
    output_path: PathBuf,
    cancel: Arc<AtomicBool>,
    events: Sender<ExportEvent>,
) {
    let temporary = temporary_path(&output_path);
    let result = run_inner(&request, &temporary, &cancel, &events);
    match result {
        Ok(true) => match finalize_temporary(&temporary, &output_path) {
            Ok(()) => send_terminal(&events, ExportEvent::Finished(output_path)),
            Err(error) => {
                remove_temporary(&temporary);
                send_terminal(&events, ExportEvent::Error(format!("{error:#}")));
            }
        },
        Ok(false) => {
            remove_temporary(&temporary);
            send_terminal(&events, ExportEvent::Cancelled);
        }
        Err(error) => {
            remove_temporary(&temporary);
            send_terminal(&events, ExportEvent::Error(format!("{error:#}")));
        }
    }
}

fn run_inner(
    request: &ExportRequest,
    temporary: &std::path::Path,
    cancel: &AtomicBool,
    events: &Sender<ExportEvent>,
) -> Result<bool> {
    let _media = decoder::initialize_media()?;
    let settings = request.settings.clone().normalized();
    let cursor = CursorOverlay::load(&request.telemetry_path, &request.metadata_path);
    let mut decoder = Decoder::open(&request.video_path).map_err(|e| e.context("decoder open"))?;
    let source = decoder.source;
    let output = OutputSize::for_source(source, settings.canvas_composition.aspect_ratio);
    let mut renderer = Renderer::new(
        decoder.device_context(),
        output,
        source,
        &settings.canvas_composition,
        composition::evaluate(&settings, source, 0, None),
    )?;
    let encoder = Encoder::open(
        temporary,
        decoder.device_context(),
        output.width,
        output.height,
        decoder.frame_rate,
    )?;
    let total = decoder.frame_rate.frame_count(decoder.duration_100ns);
    if decoder.duration_100ns == 0 || total == 0 {
        bail!("recording duration is unavailable");
    }
    let _ = events.try_send(ExportEvent::Progress {
        completed: 0,
        total,
    });

    // Export renders every frame in order, so the previous frame is always the
    // one before this in the finished video: exactly the pair display motion
    // blur must measure between.
    let mut previous: Option<PreviousFrame> = None;
    let mut cost = RenderCost::default();

    for index in 0..total {
        if is_cancelled(cancel) {
            return Ok(false);
        }
        let timestamp = decoder.frame_rate.timestamp(index);
        let seconds = timestamp as f64 / 10_000_000.0;
        let source_frame = decoder
            .frame_at(timestamp)
            .map_err(|e| e.context("frame_at"))?;
        let cursor_frame = cursor_frame(&cursor, &settings, timestamp);
        let composition = composition::evaluate(&settings, source, timestamp / 10, cursor_frame);
        let transform = composition.recording_transform();
        let motion = previous
            .zip(transform)
            .map(|(previous, current)| {
                compute_display_motion_blur(
                    previous.transform,
                    current,
                    previous.seconds,
                    seconds,
                    previous.mode,
                    composition.zoom_center(),
                    settings.motion_blur,
                )
            })
            .unwrap_or_else(MotionBlurDescriptor::inactive);
        previous = transform.map(|transform| PreviousFrame {
            transform,
            seconds,
            mode: motion.mode,
        });

        let render_started = std::time::Instant::now();
        let rendered = renderer
            .render(&source_frame.texture, composition, motion)
            .map_err(|e| e.context("render"))?;
        cost.record(motion.mode, render_started.elapsed());
        encoder
            .write(
                &rendered,
                timestamp,
                decoder.frame_rate.frame_duration(index),
            )
            .map_err(|e| e.context("encoder write"))?;
        if is_cancelled(cancel) {
            return Ok(false);
        }
        let _ = events.try_send(ExportEvent::Progress {
            completed: index + 1,
            total,
        });
    }
    encoder.finish()?;
    cost.log();
    Ok(true)
}

#[derive(Clone, Copy)]
struct PreviousFrame {
    transform: RecordingTransform,
    seconds: f64,
    mode: MotionBlurMode,
}

/// Render time split by how each frame was classified, so the cost of the
/// directional and radial passes can be read against sharp frames from the
/// same export rather than a separate benchmark.
#[derive(Default)]
struct RenderCost {
    sharp: ModeCost,
    movement: ModeCost,
    zoom: ModeCost,
}

#[derive(Default)]
struct ModeCost {
    frames: u64,
    total: std::time::Duration,
}

impl ModeCost {
    fn record(&mut self, elapsed: std::time::Duration) {
        self.frames += 1;
        self.total += elapsed;
    }

    fn average_ms(&self) -> f64 {
        if self.frames == 0 {
            0.0
        } else {
            self.total.as_secs_f64() * 1_000.0 / self.frames as f64
        }
    }
}

impl RenderCost {
    fn record(&mut self, mode: MotionBlurMode, elapsed: std::time::Duration) {
        match mode {
            MotionBlurMode::None => self.sharp.record(elapsed),
            MotionBlurMode::Movement => self.movement.record(elapsed),
            MotionBlurMode::Zoom => self.zoom.record(elapsed),
        }
    }

    fn log(&self) {
        tracing::info!(
            target: "recorder::export",
            sharp_frames = self.sharp.frames,
            sharp_ms = self.sharp.average_ms(),
            movement_frames = self.movement.frames,
            movement_ms = self.movement.average_ms(),
            zoom_frames = self.zoom.frames,
            zoom_ms = self.zoom.average_ms(),
            "export render cost by motion blur mode"
        );
    }
}

fn cursor_frame(
    overlay: &CursorOverlay,
    settings: &ProjectSettings,
    timestamp_100ns: u64,
) -> Option<super::super::cursor::CursorFrame> {
    let timestamp_us = timestamp_100ns / 10;
    let mut cursor = settings.cursor;
    cursor.scale = cursor_scale_at(&settings.cursor_size_regions, timestamp_us, cursor.scale);
    overlay.frame_at_timestamp(timestamp_100ns, cursor)
}
