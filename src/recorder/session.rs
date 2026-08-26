use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::model::{
    CURSOR_CAPTURE, RECORDING_TIMEBASE, RECORDING_ZERO, RECORDINGS_DIR, SESSION_FILE,
    TELEMETRY_FILE, VIDEO_FILE,
};
use super::project;
use super::project_settings::{ProjectSettings, save as save_project_settings};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SessionSource {
    Monitor {
        name: String,
        width: u32,
        height: u32,
    },
    Window {
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        app_name: Option<String>,
        width: u32,
        height: u32,
    },
}

impl SessionSource {
    pub(crate) fn monitor(name: impl Into<String>, width: u32, height: u32) -> Self {
        Self::Monitor {
            name: name.into(),
            width,
            height,
        }
    }

    pub(crate) fn window(
        title: impl Into<String>,
        app_name: Option<String>,
        width: u32,
        height: u32,
    ) -> Self {
        Self::Window {
            title: title.into(),
            app_name,
            width,
            height,
        }
    }

    fn legacy_name(&self) -> String {
        match self {
            Self::Monitor { name, .. } => name.clone(),
            Self::Window { title, .. } => format!("Window · {title}"),
        }
    }

    fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Monitor { width, height, .. } | Self::Window { width, height, .. } => {
                (*width, *height)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SessionPaths {
    directory: PathBuf,
    video: PathBuf,
    telemetry: PathBuf,
    metadata: PathBuf,
    created_at_utc: String,
    source: SessionSource,
}

impl SessionPaths {
    /// Creates a project folder and autosaves its manifest before capture starts.
    pub(crate) fn create(source: SessionSource) -> Result<Self, String> {
        let root = PathBuf::from(RECORDINGS_DIR);
        fs::create_dir_all(&root)
            .map_err(|error| format!("failed to create {}: {error}", root.display()))?;

        let created_at_utc = utc_timestamp(SystemTime::now());
        let directory = create_unique_directory(&root, &created_at_utc)?;
        save_project_settings(
            &project::settings_file_name(&directory),
            &ProjectSettings::default(),
        )
        .map_err(|error| format!("failed to initialize project settings: {error}"))?;
        let session = Self {
            video: directory.join(VIDEO_FILE),
            telemetry: directory.join(TELEMETRY_FILE),
            metadata: directory.join(SESSION_FILE),
            directory,
            created_at_utc,
            source,
        };

        session
            .write_metadata("recording", None, None)
            .map_err(|error| format!("failed to initialize session metadata: {error}"))?;
        Ok(session)
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn video_path(&self) -> &Path {
        &self.video
    }

    pub(crate) fn telemetry_path(&self) -> &Path {
        &self.telemetry
    }

    pub(crate) fn metadata_path(&self) -> &Path {
        &self.metadata
    }

    pub(crate) fn set_captured_dimensions(&mut self, width: u32, height: u32) {
        match &mut self.source {
            SessionSource::Monitor {
                width: source_width,
                height: source_height,
                ..
            }
            | SessionSource::Window {
                width: source_width,
                height: source_height,
                ..
            } => {
                *source_width = width;
                *source_height = height;
            }
        }
    }

    pub(crate) fn complete(
        &self,
        result: &Result<(), String>,
        dropped_frames: u64,
    ) -> Result<(), String> {
        let (status, error) = match result {
            Ok(()) => ("completed", None),
            Err(error) => ("failed", Some(error.as_str())),
        };
        self.write_metadata(status, error, Some(dropped_frames))
            .map_err(|error| format!("failed to write session metadata: {error}"))
    }

    fn write_metadata(
        &self,
        status: &'static str,
        error: Option<&str>,
        dropped_frames: Option<u64>,
    ) -> io::Result<()> {
        let legacy_name = self.source.legacy_name();
        let (width, height) = self.source.dimensions();
        let metadata = SessionMetadata {
            schema_version: 2,
            status,
            created_at_utc: &self.created_at_utc,
            finished_at_utc: (status != "recording").then(|| utc_timestamp(SystemTime::now())),
            timebase: RECORDING_TIMEBASE,
            zero: RECORDING_ZERO,
            cursor_capture: CURSOR_CAPTURE,
            video_timestamp_unit: "100ns_relative_to_first_video_frame",
            telemetry_timestamp_unit: "microseconds_from_first_video_frame",
            monitor: SessionMonitor {
                name: &legacy_name,
                width,
                height,
            },
            source: &self.source,
            files: SessionFiles {
                video: VIDEO_FILE,
                telemetry: TELEMETRY_FILE,
            },
            dropped_frames: dropped_frames.unwrap_or(0),
            error,
        };

        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.metadata)?;
        serde_json::to_writer_pretty(&mut file, &metadata)
            .map_err(|error| io::Error::other(format!("failed to encode metadata: {error}")))?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.sync_all()
    }
}

#[derive(Serialize)]
struct SessionMetadata<'a> {
    schema_version: u32,
    status: &'static str,
    created_at_utc: &'a str,
    finished_at_utc: Option<String>,
    timebase: &'static str,
    zero: &'static str,
    cursor_capture: &'static str,
    video_timestamp_unit: &'static str,
    telemetry_timestamp_unit: &'static str,
    monitor: SessionMonitor<'a>,
    source: &'a SessionSource,
    files: SessionFiles,
    dropped_frames: u64,
    error: Option<&'a str>,
}

#[derive(Serialize)]
struct SessionMonitor<'a> {
    name: &'a str,
    width: u32,
    height: u32,
}

#[derive(Serialize)]
struct SessionFiles {
    video: &'static str,
    telemetry: &'static str,
}

fn create_unique_directory(root: &Path, timestamp: &str) -> Result<PathBuf, String> {
    for suffix in 0..10_000 {
        let name = if suffix == 0 {
            timestamp.to_string()
        } else {
            format!("{timestamp}_{suffix:02}")
        };
        let path = root.join(name);
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("failed to create {}: {error}", path.display())),
        }
    }

    Err(format!(
        "could not create a unique recording directory in {}",
        root.display()
    ))
}

fn utc_timestamp(now: SystemTime) -> String {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let days = (seconds / 86_400) as i64;
    let seconds_today = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_today / 3_600;
    let minute = (seconds_today / 60) % 60;
    let second = seconds_today % 60;
    format!("{year:04}-{month:02}-{day:02}_{hour:02}-{minute:02}-{second:02}")
}

// Gregorian calendar conversion from a day count relative to 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::{civil_from_days, create_unique_directory, utc_timestamp};
    use std::fs;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn formats_session_timestamp() {
        assert_eq!(
            utc_timestamp(UNIX_EPOCH + Duration::from_secs(1_724_457_600)),
            "2024-08-24_00-00-00"
        );
    }

    #[test]
    fn converts_calendar_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_000), (2022, 1, 8));
    }

    #[test]
    fn adds_suffix_for_same_timestamp() {
        let root =
            std::env::temp_dir().join(format!("recorder-session-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let first = create_unique_directory(&root, "2026-08-24_17-52-31").unwrap();
        let second = create_unique_directory(&root, "2026-08-24_17-52-31").unwrap();

        assert_eq!(
            first.file_name().and_then(|name| name.to_str()),
            Some("2026-08-24_17-52-31")
        );
        assert_eq!(
            second.file_name().and_then(|name| name.to_str()),
            Some("2026-08-24_17-52-31_01")
        );
        let _ = fs::remove_dir_all(root);
    }
}
