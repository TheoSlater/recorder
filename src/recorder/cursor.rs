use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use serde::Deserialize;
use serde_json::Value;

use super::model::{CURSOR_CAPTURE, RECORDING_TIMEBASE, RECORDING_ZERO};

const TIMESTAMP_UNIT: &str = "microseconds";

#[derive(Clone, Copy)]
pub(super) enum CursorAsset {
    Arrow,
}

impl CursorAsset {
    pub(super) fn svg(self) -> &'static str {
        match self {
            Self::Arrow => {
                r#"<svg viewBox="0 0 24 32" aria-hidden="true"><path d="M2 1v27l7-7 5 10 4-2-5-10h9L2 1z" fill="white" stroke="black" stroke-width="2" stroke-linejoin="round"/></svg>"#
            }
        }
    }
}

#[derive(Clone, Copy)]
struct CursorSettings {
    scale: f32,
    visible: bool,
    asset: CursorAsset,
}

impl Default for CursorSettings {
    fn default() -> Self {
        Self {
            scale: 1.0,
            visible: true,
            asset: CursorAsset::Arrow,
        }
    }
}

#[derive(Clone)]
pub(super) struct CursorOverlay {
    track: Option<CursorTrack>,
    settings: CursorSettings,
    status: String,
}

impl CursorOverlay {
    pub(super) fn load(telemetry_path: &Path, metadata_path: &Path) -> Self {
        let settings = CursorSettings::default();
        let track = CursorTrack::load(telemetry_path);
        let native_cursor = native_cursor_status(metadata_path);

        match (native_cursor, track) {
            (Ok(()), Ok(track)) => Self {
                track: Some(track),
                settings,
                status: "Cursor reconstructed from telemetry; native cursor excluded".to_string(),
            },
            (Err(error), _) | (Ok(()), Err(error)) => Self {
                track: None,
                settings,
                status: error,
            },
        }
    }

    pub(super) fn disabled(status: impl Into<String>) -> Self {
        Self {
            track: None,
            settings: CursorSettings::default(),
            status: status.into(),
        }
    }

    pub(super) fn asset(&self) -> CursorAsset {
        self.settings.asset
    }

    pub(super) fn status(&self) -> &str {
        &self.status
    }

    pub(super) fn script_at(&self, seconds: f64) -> Option<String> {
        let position = self.track.as_ref()?.position_at(seconds);
        let visible = self.settings.visible && position.visible && position.in_bounds();
        Some(format!(
            "window.setCursorPosition({:.8}, {:.8}, {}, {:.4});",
            position.x, position.y, visible, self.settings.scale
        ))
    }
}

#[derive(Clone)]
struct CursorTrack {
    samples: Vec<CursorSample>,
}

impl CursorTrack {
    fn load(path: &Path) -> Result<Self, String> {
        let file =
            File::open(path).map_err(|error| format!("Cursor telemetry unavailable: {error}"))?;
        let reader = BufReader::new(file);
        let mut header_seen = false;
        let mut samples = Vec::new();

        for (line_number, line) in reader.lines().enumerate() {
            let line_number = line_number + 1;
            let line = line.map_err(|error| format!("Cursor telemetry read failed: {error}"))?;
            let value: Value = serde_json::from_str(&line).map_err(|error| {
                format!("Invalid cursor telemetry on line {line_number}: {error}")
            })?;
            let record_type = value
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("Cursor telemetry line {line_number} has no type"))?;

            match record_type {
                "header" => {
                    let header: TelemetryHeader = serde_json::from_value(value)
                        .map_err(|error| format!("Invalid cursor telemetry header: {error}"))?;
                    if header.timebase != RECORDING_TIMEBASE
                        || header.zero != RECORDING_ZERO
                        || header.timestamp_unit != TIMESTAMP_UNIT
                    {
                        return Err("Cursor telemetry uses an unsupported timebase".to_string());
                    }
                    header_seen = true;
                }
                "sample" => {
                    let sample: CursorSample = serde_json::from_value(value).map_err(|error| {
                        format!("Invalid cursor telemetry sample on line {line_number}: {error}")
                    })?;
                    if !sample.normalized_x.is_finite() || !sample.normalized_y.is_finite() {
                        return Err(format!(
                            "Cursor telemetry sample on line {line_number} has invalid coordinates"
                        ));
                    }
                    samples.push(sample);
                }
                "event" | "footer" => {}
                other => {
                    return Err(format!(
                        "Cursor telemetry line {line_number} has unknown type {other}"
                    ));
                }
            }
        }

        if !header_seen {
            return Err("Cursor telemetry has no header".to_string());
        }
        if samples.is_empty() {
            return Err("Cursor telemetry has no samples".to_string());
        }

        samples.sort_by_key(|sample| sample.timestamp_us);
        Ok(Self { samples })
    }

    fn position_at(&self, seconds: f64) -> CursorPosition {
        let micros = seconds_to_micros(seconds);
        let right = self
            .samples
            .partition_point(|sample| sample.timestamp_us <= micros);

        if right == 0 {
            return self.samples[0].position();
        }
        if right == self.samples.len() {
            return self.samples[right - 1].position();
        }

        let before = &self.samples[right - 1];
        let after = &self.samples[right];
        let span = after.timestamp_us.saturating_sub(before.timestamp_us);
        let amount = if span == 0 {
            1.0
        } else {
            (micros.saturating_sub(before.timestamp_us) as f32 / span as f32).clamp(0.0, 1.0)
        };
        CursorPosition {
            x: lerp(before.normalized_x, after.normalized_x, amount),
            y: lerp(before.normalized_y, after.normalized_y, amount),
            visible: before.visible,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
struct CursorSample {
    timestamp_us: u64,
    normalized_x: f32,
    normalized_y: f32,
    visible: bool,
}

impl CursorSample {
    fn position(self) -> CursorPosition {
        CursorPosition {
            x: self.normalized_x,
            y: self.normalized_y,
            visible: self.visible,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CursorPosition {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) visible: bool,
}

impl CursorPosition {
    fn in_bounds(self) -> bool {
        (0.0..=1.0).contains(&self.x) && (0.0..=1.0).contains(&self.y)
    }
}

#[derive(Deserialize)]
struct TelemetryHeader {
    timebase: String,
    zero: String,
    timestamp_unit: String,
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

fn seconds_to_micros(seconds: f64) -> u64 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    let micros = seconds * 1_000_000.0;
    if micros >= u64::MAX as f64 {
        u64::MAX
    } else {
        micros.round() as u64
    }
}

fn lerp(start: f32, end: f32, amount: f32) -> f32 {
    start + (end - start) * amount
}

#[cfg(test)]
mod tests {
    use super::{CursorSample, CursorTrack};

    fn track() -> CursorTrack {
        CursorTrack {
            samples: vec![
                CursorSample {
                    timestamp_us: 0,
                    normalized_x: 0.0,
                    normalized_y: 0.2,
                    visible: true,
                },
                CursorSample {
                    timestamp_us: 1_000_000,
                    normalized_x: 1.0,
                    normalized_y: 0.8,
                    visible: true,
                },
            ],
        }
    }

    #[test]
    fn interpolates_between_samples() {
        let position = track().position_at(0.5);
        assert_eq!(position.x, 0.5);
        assert_eq!(position.y, 0.5);
    }

    #[test]
    fn clamps_before_and_after_track() {
        let track = track();
        assert_eq!(track.position_at(-1.0).x, 0.0);
        assert_eq!(track.position_at(2.0).x, 1.0);
    }
}
