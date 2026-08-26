use windows_capture::window::Window;

use super::model::WindowInfo;

pub(crate) fn enumerate_windows() -> Result<Vec<WindowInfo>, String> {
    let current_process = std::process::id();
    let mut windows = Window::enumerate()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter_map(|window| {
            if !window.is_valid() || window.process_id().ok() == Some(current_process) {
                return None;
            }

            let title = window.title().ok()?.trim().to_string();
            if title.is_empty() {
                return None;
            }

            let rect = window.rect().ok()?;
            let width = u32::try_from(rect.right.saturating_sub(rect.left)).ok()?;
            let height = u32::try_from(rect.bottom.saturating_sub(rect.top)).ok()?;
            if width == 0 || height == 0 {
                return None;
            }

            let app_name = window
                .process_name()
                .ok()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty());

            Some(WindowInfo {
                window,
                title: title.into(),
                app_name: app_name.map(Into::into),
                width,
                height,
            })
        })
        .collect::<Vec<_>>();

    windows.sort_by_key(WindowInfo::label);
    Ok(windows)
}
