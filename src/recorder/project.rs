use std::fs::{self, File};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::model::{
    LEGACY_PROJECT_FILE, PROJECT_FILE_EXTENSION, RECORDINGS_DIR, SESSION_FILE, TELEMETRY_FILE,
    VIDEO_FILE,
};
use super::project_settings::{self, ProjectSettings};

/// A completed recording that can be reopened from the recorder home screen.
#[derive(Clone, Debug)]
pub(crate) struct SavedProject {
    directory: PathBuf,
    metadata: PathBuf,
    settings_path: PathBuf,
    video: PathBuf,
    telemetry: PathBuf,
    created_at_utc: String,
    source_summary: String,
    width: u32,
    height: u32,
}

impl SavedProject {
    fn load(directory: PathBuf) -> Option<Self> {
        let metadata = directory.join(SESSION_FILE);
        let settings_path = settings_path_for(&metadata);
        let manifest = File::open(&metadata)
            .ok()
            .and_then(|file| serde_json::from_reader::<_, ProjectManifest>(file).ok())?;

        if manifest.status != "completed" {
            return None;
        }

        let video = directory.join(VIDEO_FILE);
        if !video.is_file() {
            return None;
        }

        let monitor = manifest.monitor.unwrap_or_default();
        let fallback_name = if monitor.name.is_empty() {
            "Unknown source".to_string()
        } else {
            monitor.name
        };
        let source_summary = manifest
            .source
            .and_then(ProjectSource::summary)
            .unwrap_or(fallback_name);

        Some(Self {
            telemetry: directory.join(TELEMETRY_FILE),
            directory,
            metadata,
            settings_path,
            video,
            created_at_utc: manifest.created_at_utc,
            source_summary,
            width: monitor.width,
            height: monitor.height,
        })
    }

    pub(crate) fn label(&self) -> String {
        format!(
            "{} · {} · {} × {}",
            self.created_at_utc.replace('_', " "),
            self.source_summary,
            self.width,
            self.height
        )
    }

    /// Human-readable capture source, e.g. a monitor name or `App · Title`.
    pub(crate) fn source_summary(&self) -> &str {
        &self.source_summary
    }

    /// Capture dimensions in pixels; `(0, 0)` when the manifest lacked them.
    pub(crate) fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Unix timestamp (seconds) for the session's UTC creation time.
    pub(crate) fn created_at_epoch(&self) -> Option<u64> {
        timestamp_to_epoch(&self.created_at_utc)
    }

    pub(crate) fn video_path(&self) -> &std::path::Path {
        &self.video
    }

    pub(crate) fn telemetry_path(&self) -> &std::path::Path {
        &self.telemetry
    }

    pub(crate) fn metadata_path(&self) -> &std::path::Path {
        &self.metadata
    }

    pub(crate) fn settings_path(&self) -> &std::path::Path {
        &self.settings_path
    }
}

/// Reads the autosaved project manifests below the recorder's recordings folder.
pub(crate) fn load_projects() -> Vec<SavedProject> {
    load_projects_from(Path::new(RECORDINGS_DIR))
}

pub(crate) fn load_settings(path: &Path) -> ProjectSettings {
    project_settings::load(path)
}

/// Resolves the editable project save file next to a session manifest.
///
/// Prefers the named `<folder>.recproj` file and falls back to the legacy
/// `project.json` so older recordings keep their saved settings; new projects
/// always use the named file.
pub(crate) fn settings_path_for(metadata_path: &Path) -> PathBuf {
    let directory = metadata_path.parent().unwrap_or(Path::new(""));
    let preferred = settings_file_name(directory);
    if preferred.is_file() {
        return preferred;
    }

    let legacy = directory.join(LEGACY_PROJECT_FILE);
    if legacy.is_file() {
        return legacy;
    }

    preferred
}

/// The `<folder>.recproj` save file inside a project directory.
pub(crate) fn settings_file_name(directory: &Path) -> PathBuf {
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("project");
    directory.join(format!("{name}.{PROJECT_FILE_EXTENSION}"))
}

fn load_projects_from(root: &Path) -> Vec<SavedProject> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut projects: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(SavedProject::load)
        .collect();
    projects.sort_by(|left, right| {
        right
            .created_at_utc
            .cmp(&left.created_at_utc)
            .then_with(|| right.directory.cmp(&left.directory))
    });
    projects
}

#[derive(Deserialize)]
struct ProjectManifest {
    status: String,
    #[serde(default)]
    created_at_utc: String,
    #[serde(default)]
    monitor: Option<ProjectMonitor>,
    #[serde(default)]
    source: Option<ProjectSource>,
}

#[derive(Default, Deserialize)]
struct ProjectMonitor {
    #[serde(default)]
    name: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

/// Mirrors the `SessionSource` shape written into session manifests.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProjectSource {
    Monitor {
        #[serde(default)]
        name: String,
    },
    Window {
        #[serde(default)]
        title: String,
        #[serde(default)]
        app_name: Option<String>,
    },
}

impl ProjectSource {
    fn summary(self) -> Option<String> {
        match self {
            Self::Monitor { name } => non_empty(name),
            Self::Window { title, app_name } => app_name
                .filter(|app| !app.is_empty())
                .map(|app| format!("{app} · {title}"))
                .or_else(|| non_empty(title)),
        }
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// Seconds since the Unix epoch for a `YYYY-MM-DD_HH-MM-SS` UTC timestamp.
fn timestamp_to_epoch(timestamp: &str) -> Option<u64> {
    let (date, time) = timestamp.split_once('_')?;
    let mut date = date.split('-');
    let mut time = time.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: i64 = date.next()?.parse().ok()?;
    let day: i64 = date.next()?.parse().ok()?;
    let hour: i64 = time.next()?.parse().ok()?;
    let minute: i64 = time.next()?.parse().ok()?;
    let second: i64 = time.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    Some((days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second) as u64)
}

/// Inverse of the civil-from-days conversion used to write session timestamps.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{days_from_civil, load_projects_from, settings_path_for, timestamp_to_epoch};

    #[test]
    fn converts_timestamps_to_epoch_seconds() {
        assert_eq!(
            timestamp_to_epoch("2026-08-24_18-00-00"),
            Some(1_787_594_400)
        );
        assert_eq!(timestamp_to_epoch("1970-01-01_00-00-00"), Some(0));
        assert_eq!(timestamp_to_epoch("2026-13-01_00-00-00"), None);
        assert_eq!(timestamp_to_epoch("not-a-timestamp"), None);
    }

    #[test]
    fn civil_conversion_matches_known_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2024, 1, 1), 19_723);
        assert_eq!(days_from_civil(2026, 1, 1), 20_454);
        assert_eq!(days_from_civil(2026, 8, 24), 20_689);
    }

    #[test]
    fn loads_completed_projects_with_video() {
        let root =
            std::env::temp_dir().join(format!("recorder-project-test-{}", std::process::id()));
        let directory = root.join("2026-08-24_18-00-00");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("recording.mp4"), b"video").unwrap();
        fs::write(
            directory.join("session.json"),
            r#"{
                "status": "completed",
                "created_at_utc": "2026-08-24_18-00-00",
                "monitor": {"name": "Primary", "width": 1920, "height": 1080},
                "source": {"kind": "window", "title": "Brave", "app_name": "Browser", "width": 1920, "height": 1080}
            }"#,
        )
        .unwrap();

        let projects = load_projects_from(&root);

        assert_eq!(projects.len(), 1);
        assert_eq!(
            projects[0].label(),
            "2026-08-24 18-00-00 · Browser · Brave · 1920 × 1080"
        );
        assert_eq!(projects[0].source_summary(), "Browser · Brave");
        assert_eq!(projects[0].created_at_epoch(), Some(1_787_594_400));
        assert_eq!(projects[0].video_path(), directory.join("recording.mp4"));
        assert_eq!(
            projects[0].settings_path(),
            directory.join("2026-08-24_18-00-00.recproj")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn falls_back_to_monitor_name_without_source_field() {
        let root =
            std::env::temp_dir().join(format!("recorder-project-legacy-{}", std::process::id()));
        let directory = root.join("2026-08-24_18-00-00");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("recording.mp4"), b"video").unwrap();
        fs::write(
            directory.join("session.json"),
            r#"{
                "status": "completed",
                "created_at_utc": "2026-08-24_18-00-00",
                "monitor": {"name": "LG IPS FULLHD", "width": 1920, "height": 1080}
            }"#,
        )
        .unwrap();

        let projects = load_projects_from(&root);

        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].source_summary(), "LG IPS FULLHD");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolves_named_settings_file_with_legacy_fallback() {
        let root = std::env::temp_dir().join(format!(
            "recorder-project-settings-path-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);

        let metadata = root.join("2026-08-24_18-00-00").join("session.json");
        assert_eq!(
            settings_path_for(&metadata),
            root.join("2026-08-24_18-00-00")
                .join("2026-08-24_18-00-00.recproj")
        );

        let directory = root.join("2026-08-24_18-00-00");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("project.json"), b"{}").unwrap();
        assert_eq!(settings_path_for(&metadata), directory.join("project.json"));

        fs::write(directory.join("2026-08-24_18-00-00.recproj"), b"{}").unwrap();
        assert_eq!(
            settings_path_for(&metadata),
            directory.join("2026-08-24_18-00-00.recproj")
        );

        let _ = fs::remove_dir_all(root);
    }
}
