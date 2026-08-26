use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

use anyhow::{Result, bail};
use crossbeam_channel::Sender;

use super::{
    ExportEvent, ExportRequest, finalize_temporary, is_cancelled, remove_temporary,
    send_terminal, temporary_path,
};
use super::super::{
    composition::{self, OutputSize},
    cursor::CursorOverlay,
    project_settings::ProjectSettings,
    zoom::cursor_scale_at,
};
use super::{decoder::{self, Decoder}, encoder::Encoder, renderer::Renderer};

const MEDIA_TIME_PER_SECOND: u64 = 10_000_000;

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
                send_terminal(&events, ExportEvent::Error(error.to_string()));
            }
        },
        Ok(false) => {
            remove_temporary(&temporary);
            send_terminal(&events, ExportEvent::Cancelled);
        }
        Err(error) => {
            remove_temporary(&temporary);
            send_terminal(&events, ExportEvent::Error(error.to_string()));
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
    let mut decoder = Decoder::open(&request.video_path)?;
    let source = decoder.source;
    let output = OutputSize::for_source(source, settings.canvas_composition.aspect_ratio);
    let renderer = Renderer::new(
        decoder.device_context(),
        output.width,
        output.height,
        &settings.canvas_composition,
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
    let _ = events.try_send(ExportEvent::Progress { completed: 0, total });

    for index in 0..total {
        if is_cancelled(cancel) {
            return Ok(false);
        }
        let timestamp = decoder.frame_rate.timestamp(index).min(
            decoder
                .duration_100ns
                .saturating_sub(decoder.frame_rate.duration_100ns()),
        );
        let source_frame = decoder.frame_at(timestamp)?;
        let cursor_frame = cursor_frame(&cursor, &settings, timestamp);
        let composition = composition::evaluate(
            &settings,
            source,
            timestamp / 10,
            cursor_frame,
        );
        let rendered = renderer.render(
            &source_frame.texture,
            &composition,
            source,
            &settings.canvas_composition,
        )?;
        encoder.write(rendered, timestamp)?;
        if is_cancelled(cancel) {
            return Ok(false);
        }
        let _ = events.try_send(ExportEvent::Progress { completed: index + 1, total });
    }
    encoder.finish()?;
    Ok(true)
}

fn cursor_frame(
    overlay: &CursorOverlay,
    settings: &ProjectSettings,
    timestamp_100ns: u64,
) -> Option<super::super::cursor::CursorFrame> {
    let timestamp_us = timestamp_100ns / 10;
    let mut cursor = settings.cursor;
    cursor.scale = cursor_scale_at(&settings.cursor_size_regions, timestamp_us, cursor.scale);
    overlay.frame_at(seconds_from_timestamp(timestamp_100ns), cursor)
}

fn seconds_from_timestamp(timestamp_100ns: u64) -> f64 {
    timestamp_100ns as f64 / MEDIA_TIME_PER_SECOND as f64
}

#[cfg(test)]
mod tests {
    use super::seconds_from_timestamp;

    #[test]
    fn cursor_timestamp_uses_media_timebase() {
        assert_eq!(seconds_from_timestamp(12_500_000), 1.25);
        assert_eq!(seconds_from_timestamp(1), 0.0000001);
    }
}
