use std::path::PathBuf;

use anyhow::Result;
use gpui::AsyncApp;

mod alerts;
mod auto_zoom;
mod capture;
mod components;
mod composition;
mod cursor;
mod cursor_settings;
mod encoder;
mod export;
mod home_ui;
mod hooks;
mod input;
mod lifecycle;
mod media;
mod model;
mod monitors;
mod motion_blur;
mod overlay;
mod playback;
mod project;
mod project_actions;
mod project_save;
mod project_settings;
mod project_ui;
mod rendering;
mod session;
mod thumbnails;
mod ui;
mod windows;
mod zoom;

pub(crate) use lifecycle::ShutdownCoordinator;
pub(crate) use monitors::enumerate_monitors;
pub(crate) use ui::RecorderView;
pub(crate) use windows::enumerate_windows;

/// Opens a video directly for local playback profiling, without needing a
/// home-screen project entry.
///
/// A video that happens to sit in a recording directory brings that recording's
/// telemetry, manifest, and saved settings with it. Ignoring them produced an
/// editor with no cursor and no zoom regions, which looks like a rendering
/// fault rather than a missing input; only a video from somewhere else falls
/// back to defaults.
pub(crate) fn open_debug_video(cx: &mut AsyncApp, video_path: PathBuf) -> Result<()> {
    let directory = video_path.parent().map(std::path::Path::to_path_buf);
    let beside = |name: &str| {
        directory
            .as_ref()
            .map(|directory| directory.join(name))
            .filter(|path| path.is_file())
            .unwrap_or_default()
    };
    let settings_path = directory
        .as_ref()
        .and_then(|directory| {
            let name = directory.file_name()?.to_str()?;
            Some(directory.join(format!("{name}.recproj")))
        })
        .filter(|path| path.is_file());
    let settings = settings_path
        .as_deref()
        .map(project::load_settings)
        .unwrap_or_default();

    playback::open(
        cx,
        video_path,
        beside("telemetry.jsonl"),
        beside("session.json"),
        settings_path.unwrap_or_else(|| PathBuf::from("target/recorder-debug.recproj")),
        settings,
        false,
        true,
    )
    .map(|_| ())
}
