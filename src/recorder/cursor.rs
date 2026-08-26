use std::{fs::File, path::Path};

use serde_json::Value;

use super::cursor_settings::{CursorAsset, CursorSettings};
use super::model::CURSOR_CAPTURE;
use super::{auto_zoom, zoom::ZoomRegion};

mod track;

pub(super) use track::{CursorEvent, CursorSample, CursorTrack};

#[derive(Clone)]
pub(super) struct CursorOverlay {
    evaluator: CursorEvaluator,
    status: String,
    warning: Option<String>,
}

impl CursorOverlay {
    pub(super) fn loading() -> Self {
        Self {
            evaluator: CursorEvaluator { track: None },
            status: "Loading cursor telemetry…".to_string(),
            warning: None,
        }
    }

    pub(super) fn load(telemetry_path: &Path, metadata_path: &Path) -> Self {
        let track = CursorTrack::load(telemetry_path);
        let native_cursor = native_cursor_status(metadata_path);

        match (native_cursor, track) {
            (Ok(()), Ok(track)) => Self {
                warning: track.warning.clone(),
                evaluator: CursorEvaluator { track: Some(track) },
                status: "Cursor reconstructed from telemetry; native cursor excluded".to_string(),
            },
            (Err(error), _) | (Ok(()), Err(error)) => Self {
                evaluator: CursorEvaluator { track: None },
                status: error,
                warning: None,
            },
        }
    }

    pub(super) fn disabled(status: impl Into<String>) -> Self {
        Self {
            evaluator: CursorEvaluator { track: None },
            status: status.into(),
            warning: None,
        }
    }

    pub(super) fn status(&self) -> &str {
        &self.status
    }

    pub(super) fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    pub(super) fn frame_at(&self, seconds: f64, settings: CursorSettings) -> Option<CursorFrame> {
        self.evaluator.frame_at(seconds, settings)
    }

    pub(super) fn auto_zoom_regions_with_report(
        &self,
        duration_us: u64,
        existing: &[ZoomRegion],
    ) -> (Vec<ZoomRegion>, auto_zoom::GenerationReport) {
        self.evaluator
            .track
            .as_ref()
            .map(|track| {
                auto_zoom::generate_with_report(
                    &track.samples,
                    &track.events,
                    duration_us,
                    existing,
                )
            })
            .unwrap_or_default()
    }

    pub(super) fn has_telemetry(&self) -> bool {
        self.evaluator.track.is_some()
    }
}

#[derive(Clone)]
pub(super) struct CursorEvaluator {
    track: Option<CursorTrack>,
}

impl CursorEvaluator {
    pub(super) fn frame_at(&self, seconds: f64, settings: CursorSettings) -> Option<CursorFrame> {
        let settings = settings.normalized();
        let track = self.track.as_ref()?;
        let position = track.position_at(seconds, settings.smoothing);
        Some(CursorFrame {
            x: position.x,
            y: position.y,
            visible: settings.visible && position.visible,
            scale: settings.scale * (track.click_bounce_at(seconds) as f32),
            asset: settings.style.asset(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CursorFrame {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) visible: bool,
    pub(super) scale: f32,
    pub(super) asset: CursorAsset,
}

fn native_cursor_status(path: &Path) -> Result<(), String> {
    let file = File::open(path).map_err(|error| {
        format!(
            "Native cursor capture status is unknown ({error}); reconstructed cursor disabled to avoid a double cursor"
        )
    })?;
    let metadata: Value = serde_json::from_reader(file).map_err(|error| {
        format!("Native cursor capture status is invalid ({error}); reconstructed cursor disabled")
    })?;
    match metadata.get("cursor_capture").and_then(Value::as_str) {
        Some(CURSOR_CAPTURE) => Ok(()),
        Some(value) => Err(format!(
            "Native cursor capture is {value}; reconstructed cursor disabled to avoid a double cursor"
        )),
        None => Err(
            "Native cursor capture status is unknown; reconstructed cursor disabled to avoid a double cursor"
                .to_string(),
        ),
    }
}

#[cfg(test)]
#[path = "cursor_tests.rs"]
mod tests;
