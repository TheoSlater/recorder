use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use serde::Serialize;

use super::model::{
    ButtonState, CursorSample, MonitorMetadata, MouseEvent, TelemetryEvent, TelemetryFooter,
    TelemetryHeader, TelemetrySample,
};

const FLUSH_LINES: u64 = 64;

pub(crate) struct TelemetryWriter {
    writer: BufWriter<File>,
    samples: u64,
    events: u64,
    initial_button_state: Option<ButtonState>,
    lines_since_flush: u64,
    path: String,
}

impl TelemetryWriter {
    pub(crate) fn new(path: &Path, monitor: MonitorMetadata) -> Result<Self, String> {
        let file = File::create(path)
            .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
        let mut writer = Self {
            writer: BufWriter::new(file),
            samples: 0,
            events: 0,
            initial_button_state: None,
            lines_since_flush: 0,
            path: path.display().to_string(),
        };
        writer.write_line(&TelemetryHeader::new(monitor))?;
        Ok(writer)
    }

    pub(crate) fn write_sample(&mut self, sample: CursorSample) -> Result<(), String> {
        if self.initial_button_state.is_none() {
            self.initial_button_state = Some(sample.buttons);
        }
        self.write_line(&TelemetrySample::new(sample))?;
        self.samples += 1;
        Ok(())
    }

    pub(crate) fn write_event(&mut self, event: MouseEvent) -> Result<(), String> {
        self.write_line(&TelemetryEvent::new(event))?;
        self.events += 1;
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<(), String> {
        self.write_line(&TelemetryFooter::new(
            self.samples,
            self.events,
            self.initial_button_state,
        ))?;
        self.writer
            .flush()
            .map_err(|error| format!("failed to flush {}: {error}", self.path))?;
        self.writer
            .get_ref()
            .sync_all()
            .map_err(|error| format!("failed to sync {}: {error}", self.path))
    }

    fn write_line<T: Serialize>(&mut self, line: &T) -> Result<(), String> {
        serde_json::to_writer(&mut self.writer, line)
            .map_err(|error| format!("failed to encode {}: {error}", self.path))?;
        self.writer
            .write_all(b"\n")
            .map_err(|error| format!("failed to write {}: {error}", self.path))?;
        self.lines_since_flush += 1;
        if self.lines_since_flush >= FLUSH_LINES {
            self.writer
                .flush()
                .map_err(|error| format!("failed to flush {}: {error}", self.path))?;
            self.lines_since_flush = 0;
        }
        Ok(())
    }
}
