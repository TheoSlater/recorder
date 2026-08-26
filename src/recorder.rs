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
mod session;
mod ui;
mod windows;
mod zoom;

pub(crate) use lifecycle::ShutdownCoordinator;
pub(crate) use monitors::enumerate_monitors;
pub(crate) use ui::RecorderView;
pub(crate) use windows::enumerate_windows;

/// Opens a video directly for local playback profiling without requiring a
/// persisted session manifest or a home-screen project entry.
pub(crate) fn open_debug_video(cx: &mut AsyncApp, video_path: PathBuf) -> Result<()> {
    let project_path = PathBuf::from("target/recorder-debug.recproj");
    playback::open(
        cx,
        video_path,
        PathBuf::new(),
        PathBuf::new(),
        project_path,
        project_settings::ProjectSettings::default(),
        false,
        true,
    )
    .map(|_| ())
}
