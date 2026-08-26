use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::cursor_settings::CursorSettings;
use super::zoom::{CursorSizeRegion, ZoomRegion};

const SCHEMA_VERSION: u32 = 4;

pub(super) const MIN_CANVAS_ZOOM: f64 = 0.25;
pub(super) const MAX_CANVAS_ZOOM: f64 = 4.0;
pub(super) const MIN_COMPOSITION_SCALE: f64 = 0.25;
pub(super) const MAX_COMPOSITION_SCALE: f64 = 2.0;
pub(super) const MAX_COMPOSITION_PADDING: f64 = 0.45;
pub(super) const MAX_COMPOSITION_RADIUS: f64 = 0.25;

pub(super) const ASPECT_RATIO_LABELS: [&str; 5] = ["16:9", "4:3", "1:1", "4:5", "9:16"];
pub(super) const BACKGROUND_KIND_LABELS: [&str; 3] = ["Colour", "Gradient", "Image"];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ProjectSettings {
    #[serde(default = "schema_version")]
    pub(super) schema_version: u32,
    #[serde(default)]
    pub(super) cursor: CursorSettings,
    #[serde(default)]
    pub(super) canvas: CanvasView,
    #[serde(default)]
    pub(super) canvas_composition: CanvasComposition,
    #[serde(default)]
    pub(super) zoom_regions: Vec<ZoomRegion>,
    #[serde(default)]
    pub(super) cursor_size_regions: Vec<CursorSizeRegion>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct CanvasView {
    #[serde(default = "default_canvas_zoom")]
    pub(super) zoom: f64,
    #[serde(default)]
    pub(super) pan_x: f64,
    #[serde(default)]
    pub(super) pan_y: f64,
}

impl Default for CanvasView {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AspectRatioPreset {
    #[default]
    Widescreen,
    Standard,
    Square,
    Portrait,
    Vertical,
}

impl AspectRatioPreset {
    pub(super) fn index(self) -> usize {
        match self {
            Self::Widescreen => 0,
            Self::Standard => 1,
            Self::Square => 2,
            Self::Portrait => 3,
            Self::Vertical => 4,
        }
    }

    pub(super) fn from_label(label: &str) -> Self {
        match label {
            "4:3" => Self::Standard,
            "1:1" => Self::Square,
            "4:5" => Self::Portrait,
            "9:16" => Self::Vertical,
            _ => Self::Widescreen,
        }
    }

    pub(super) fn ratio(self) -> f32 {
        match self {
            Self::Widescreen => 16. / 9.,
            Self::Standard => 4. / 3.,
            Self::Square => 1.,
            Self::Portrait => 4. / 5.,
            Self::Vertical => 9. / 16.,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CanvasBackgroundKind {
    #[default]
    Solid,
    Gradient,
    Image,
}

impl CanvasBackgroundKind {
    pub(super) fn index(self) -> usize {
        match self {
            Self::Solid => 0,
            Self::Gradient => 1,
            Self::Image => 2,
        }
    }

    pub(super) fn from_label(label: &str) -> Self {
        match label {
            "Gradient" => Self::Gradient,
            "Image" => Self::Image,
            _ => Self::Solid,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CanvasBackground {
    #[serde(default)]
    pub(super) kind: CanvasBackgroundKind,
    #[serde(default)]
    pub(super) solid_color: Option<String>,
    #[serde(default)]
    pub(super) gradient_start: Option<String>,
    #[serde(default)]
    pub(super) gradient_end: Option<String>,
    #[serde(default)]
    pub(super) image_path: Option<PathBuf>,
}

impl CanvasBackground {
    pub(super) fn normalized(mut self) -> Self {
        self.solid_color = normalize_color(self.solid_color);
        self.gradient_start = normalize_color(self.gradient_start);
        self.gradient_end = normalize_color(self.gradient_end);
        self.image_path = self.image_path.filter(|path| !path.as_os_str().is_empty());
        self
    }
}

/// Composition values use normalized units so the preview and future export
/// stay independent of the current window size.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct CanvasComposition {
    #[serde(default)]
    pub(super) aspect_ratio: AspectRatioPreset,
    #[serde(default)]
    pub(super) position_x: f64,
    #[serde(default)]
    pub(super) position_y: f64,
    #[serde(default = "default_composition_scale")]
    pub(super) scale: f64,
    #[serde(default = "default_composition_padding")]
    pub(super) padding: f64,
    #[serde(default)]
    pub(super) corner_radius: f64,
    #[serde(default)]
    pub(super) shadow: bool,
    #[serde(default)]
    pub(super) background: CanvasBackground,
}

impl Default for CanvasComposition {
    fn default() -> Self {
        Self {
            aspect_ratio: AspectRatioPreset::default(),
            position_x: 0.0,
            position_y: 0.0,
            scale: default_composition_scale(),
            padding: default_composition_padding(),
            corner_radius: 0.0,
            shadow: false,
            background: CanvasBackground::default(),
        }
    }
}

impl CanvasComposition {
    pub(super) fn normalized(mut self) -> Self {
        self.position_x = finite_or(self.position_x, 0.0).clamp(-1.0, 1.0);
        self.position_y = finite_or(self.position_y, 0.0).clamp(-1.0, 1.0);
        self.scale = finite_or(self.scale, default_composition_scale())
            .clamp(MIN_COMPOSITION_SCALE, MAX_COMPOSITION_SCALE);
        self.padding = finite_or(self.padding, default_composition_padding())
            .clamp(0.0, MAX_COMPOSITION_PADDING);
        self.corner_radius = finite_or(self.corner_radius, 0.0).clamp(0.0, MAX_COMPOSITION_RADIUS);
        self.background = self.background.normalized();
        self
    }
}

impl CanvasView {
    pub(super) fn normalized(mut self) -> Self {
        self.zoom = if self.zoom.is_finite() {
            self.zoom.clamp(MIN_CANVAS_ZOOM, MAX_CANVAS_ZOOM)
        } else {
            1.0
        };
        self.pan_x = if self.pan_x.is_finite() {
            self.pan_x
        } else {
            0.0
        };
        self.pan_y = if self.pan_y.is_finite() {
            self.pan_y
        } else {
            0.0
        };
        self
    }
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            cursor: CursorSettings::default(),
            canvas: CanvasView::default(),
            canvas_composition: CanvasComposition::default(),
            zoom_regions: Vec::new(),
            cursor_size_regions: Vec::new(),
        }
    }
}

impl ProjectSettings {
    pub(super) fn normalized(mut self) -> Self {
        self.schema_version = SCHEMA_VERSION;
        self.cursor = self.cursor.normalized();
        self.canvas = self.canvas.normalized();
        self.canvas_composition = self.canvas_composition.normalized();
        self.zoom_regions = self
            .zoom_regions
            .into_iter()
            .filter_map(ZoomRegion::normalized)
            .collect();
        self.cursor_size_regions = self
            .cursor_size_regions
            .into_iter()
            .filter_map(CursorSizeRegion::normalized)
            .collect();
        self
    }
}

pub(super) fn load(path: &Path) -> ProjectSettings {
    File::open(path)
        .ok()
        .and_then(|file| serde_json::from_reader(file).ok())
        .map(ProjectSettings::normalized)
        .unwrap_or_default()
}

pub(super) fn save(path: &Path, settings: &ProjectSettings) -> Result<(), String> {
    let settings = settings.clone().normalized();
    let temporary = temporary_path(path);
    let result = write_temporary(&temporary, &settings).and_then(|_| replace(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| format!("failed to save project settings: {error}"))
}

fn write_temporary(path: &Path, settings: &ProjectSettings) -> io::Result<()> {
    let _ = fs::remove_file(path);
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    serde_json::to_writer_pretty(&mut file, settings)
        .map_err(|error| io::Error::other(format!("failed to encode project settings: {error}")))?;
    file.write_all(b"\n")?;
    file.flush()?;
    file.sync_all()
}

#[cfg(windows)]
fn replace(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;

    unsafe { MoveFileExW(PCWSTR(source.as_ptr()), PCWSTR(target.as_ptr()), flags) }
        .map_err(|_| io::Error::last_os_error())
}

#[cfg(not(windows))]
fn replace(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project.json");
    path.with_file_name(format!(".{name}.tmp"))
}

fn schema_version() -> u32 {
    SCHEMA_VERSION
}

fn default_canvas_zoom() -> f64 {
    1.0
}

fn default_composition_scale() -> f64 {
    1.0
}

fn default_composition_padding() -> f64 {
    0.08
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { value } else { fallback }
}

fn normalize_color(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        let hex = value.strip_prefix('#').unwrap_or(value);
        if matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Some(format!("#{hex}"))
        } else {
            None
        }
    })
}

#[cfg(test)]
#[path = "project_settings_tests.rs"]
mod tests;
