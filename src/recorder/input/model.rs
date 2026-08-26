use serde::{Deserialize, Serialize};

use super::super::model::{RECORDING_TIMEBASE, RECORDING_ZERO};

pub(crate) const SAMPLE_INTERVAL_US: u64 = 8_333;
pub(crate) const TELEMETRY_SCHEMA_VERSION: u32 = 2;
pub(crate) const COORDINATE_SPACE: &str = "virtual_desktop_pixels";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub(crate) struct ButtonState {
    pub(crate) left: bool,
    pub(crate) right: bool,
    pub(crate) middle: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct CursorSample {
    pub(crate) timestamp_us: u64,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) normalized_x: f32,
    pub(crate) normalized_y: f32,
    pub(crate) screen_x: i32,
    pub(crate) screen_y: i32,
    pub(crate) cursor_id: Option<u64>,
    pub(crate) visible: bool,
    pub(crate) buttons: ButtonState,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MouseEventKind {
    LeftDown,
    LeftUp,
    RightDown,
    RightUp,
    MiddleDown,
    MiddleUp,
}

#[derive(Debug, Serialize)]
pub(crate) struct MouseEvent {
    pub(crate) timestamp_us: u64,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) normalized_x: f32,
    pub(crate) normalized_y: f32,
    pub(crate) screen_x: i32,
    pub(crate) screen_y: i32,
    pub(crate) kind: MouseEventKind,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MonitorMetadata {
    pub(crate) device_name: String,
    pub(crate) origin_x: i32,
    pub(crate) origin_y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Serialize)]
pub(crate) struct TelemetryHeader {
    #[serde(rename = "type")]
    pub(crate) record_type: &'static str,
    pub(crate) schema_version: u32,
    pub(crate) timebase: &'static str,
    pub(crate) zero: &'static str,
    pub(crate) timestamp_unit: &'static str,
    pub(crate) coordinate_space: &'static str,
    pub(crate) monitor: MonitorMetadata,
}

impl TelemetryHeader {
    pub(crate) fn new(monitor: MonitorMetadata) -> Self {
        Self {
            record_type: "header",
            schema_version: TELEMETRY_SCHEMA_VERSION,
            timebase: RECORDING_TIMEBASE,
            zero: RECORDING_ZERO,
            timestamp_unit: "microseconds",
            coordinate_space: COORDINATE_SPACE,
            monitor,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct TelemetrySample {
    #[serde(rename = "type")]
    pub(crate) record_type: &'static str,
    #[serde(flatten)]
    pub(crate) sample: CursorSample,
}

impl TelemetrySample {
    pub(crate) fn new(sample: CursorSample) -> Self {
        Self {
            record_type: "sample",
            sample,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct TelemetryEvent {
    #[serde(rename = "type")]
    pub(crate) record_type: &'static str,
    #[serde(flatten)]
    pub(crate) event: MouseEvent,
}

impl TelemetryEvent {
    pub(crate) fn new(event: MouseEvent) -> Self {
        Self {
            record_type: "event",
            event,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct TelemetryFooter {
    #[serde(rename = "type")]
    pub(crate) record_type: &'static str,
    pub(crate) samples: u64,
    pub(crate) events: u64,
    pub(crate) initial_button_state: Option<ButtonState>,
}

impl TelemetryFooter {
    pub(crate) fn new(
        samples: u64,
        events: u64,
        initial_button_state: Option<ButtonState>,
    ) -> Self {
        Self {
            record_type: "footer",
            samples,
            events,
            initial_button_state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_records_are_flat_jsonl_objects() {
        let sample = TelemetrySample::new(CursorSample {
            timestamp_us: 42,
            x: 12.0,
            y: 34.0,
            normalized_x: 0.00625,
            normalized_y: 0.03148,
            screen_x: -1908,
            screen_y: 34,
            cursor_id: Some(7),
            visible: true,
            buttons: ButtonState::default(),
        });

        let json = serde_json::to_string(&sample).unwrap();
        assert!(json.starts_with(r#"{"type":"sample","timestamp_us":42"#));
        assert!(!json.contains("\"samples\":["));
    }
}
