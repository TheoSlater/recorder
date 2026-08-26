use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use serde::Deserialize;
use serde_json::Value;

use super::super::input::{ButtonState, MouseEventKind};
use super::super::model::{RECORDING_TIMEBASE, RECORDING_ZERO};

const TIMESTAMP_UNIT: &str = "microseconds";
const SMOOTHING_WINDOW_MIN_SECONDS: f64 = 0.04;
const SMOOTHING_WINDOW_MAX_SECONDS: f64 = 0.3;
const SMOOTHING_TAPS: usize = 9;
const SMOOTHING_WEIGHTS: [f64; SMOOTHING_TAPS] = [1.0, 2.0, 3.0, 4.0, 5.0, 4.0, 3.0, 2.0, 1.0];
const SMOOTHING_WEIGHT_TOTAL: f64 = 25.0;
const CLICK_BOUNCE_DURATION_SECONDS: f64 = 0.24;
const CLICK_BOUNCE_AMPLITUDE: f64 = 0.18;
const CLICK_BOUNCE_CYCLES: f64 = 1.25;

#[derive(Clone)]
pub(crate) struct CursorTrack {
    pub(crate) samples: Vec<CursorSample>,
    pub(crate) events: Vec<CursorEvent>,
    pub(crate) warning: Option<String>,
}

impl CursorTrack {
    pub(super) fn load(path: &Path) -> Result<Self, String> {
        let file =
            File::open(path).map_err(|error| format!("Cursor telemetry unavailable: {error}"))?;
        let reader = BufReader::new(file);
        let mut header_seen = false;
        let mut samples = Vec::new();
        let mut events = Vec::new();
        let mut invalid_samples = 0;

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
                        invalid_samples += 1;
                        continue;
                    }
                    samples.push(sample);
                }
                "event" => {
                    let event: CursorEvent = serde_json::from_value(value).map_err(|error| {
                        format!("Invalid cursor telemetry event on line {line_number}: {error}")
                    })?;
                    events.push(event);
                }
                "footer" => {}
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
        let warning = if invalid_samples > 0 {
            tracing::warn!(
                target: "recorder::playback",
                invalid_samples,
                path = %path.display(),
                "ignored invalid cursor telemetry samples"
            );
            Some(format!(
                "Ignored {invalid_samples} invalid cursor telemetry sample(s)."
            ))
        } else {
            None
        };
        if samples.is_empty() {
            return Err("Cursor telemetry has no samples".to_string());
        }

        samples.sort_by_key(|sample| sample.timestamp_us);
        events.sort_by_key(|event| event.timestamp_us);
        Ok(Self {
            samples,
            events,
            warning,
        })
    }

    pub(super) fn position_at(&self, seconds: f64, smoothing: f32) -> CursorPosition {
        let center = self.raw_position_at(seconds);
        if smoothing <= 0.0 {
            return center;
        }

        let window = SMOOTHING_WINDOW_MIN_SECONDS
            + f64::from(smoothing) * (SMOOTHING_WINDOW_MAX_SECONDS - SMOOTHING_WINDOW_MIN_SECONDS);
        let step = window / (SMOOTHING_TAPS - 1) as f64;
        let start = seconds - window / 2.0;
        let mut x = 0.0;
        let mut y = 0.0;
        for (tap, weight) in SMOOTHING_WEIGHTS.iter().enumerate() {
            let position = self.raw_position_at(start + step * tap as f64);
            x += f64::from(position.x) * *weight;
            y += f64::from(position.y) * *weight;
        }
        CursorPosition {
            x: (x / SMOOTHING_WEIGHT_TOTAL) as f32,
            y: (y / SMOOTHING_WEIGHT_TOTAL) as f32,
            visible: center.visible,
        }
    }

    pub(super) fn click_bounce_at(&self, seconds: f64) -> f64 {
        if !seconds.is_finite() || seconds < 0.0 {
            return 1.0;
        }

        let playhead_us = seconds_to_micros(seconds);
        let Some(press) = self
            .events
            .iter()
            .filter(|event| event.timestamp_us <= playhead_us)
            .filter(|event| {
                matches!(
                    event.kind,
                    MouseEventKind::LeftDown
                        | MouseEventKind::RightDown
                        | MouseEventKind::MiddleDown
                )
            })
            .max_by_key(|event| event.timestamp_us)
        else {
            return 1.0;
        };

        let elapsed = playhead_us.saturating_sub(press.timestamp_us) as f64 / 1_000_000.0;
        if elapsed >= CLICK_BOUNCE_DURATION_SECONDS {
            return 1.0;
        }

        let progress = (elapsed / CLICK_BOUNCE_DURATION_SECONDS).clamp(0.0, 1.0);
        let settle = (1.0 - progress).powi(2);
        let phase = std::f64::consts::TAU * CLICK_BOUNCE_CYCLES * progress;
        1.0 - CLICK_BOUNCE_AMPLITUDE * settle * phase.cos()
    }

    fn raw_position_at(&self, seconds: f64) -> CursorPosition {
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

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct CursorSample {
    pub(crate) timestamp_us: u64,
    pub(crate) normalized_x: f32,
    pub(crate) normalized_y: f32,
    pub(crate) visible: bool,
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) buttons: ButtonState,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct CursorEvent {
    pub(crate) timestamp_us: u64,
    pub(crate) normalized_x: f32,
    pub(crate) normalized_y: f32,
    pub(crate) kind: MouseEventKind,
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

#[derive(Deserialize)]
struct TelemetryHeader {
    timebase: String,
    zero: String,
    timestamp_unit: String,
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
