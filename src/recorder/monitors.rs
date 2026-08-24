use windows_capture::monitor::Monitor;

use super::model::MonitorInfo;

pub(crate) fn enumerate_monitors() -> Result<Vec<MonitorInfo>, String> {
    let monitors = Monitor::enumerate().map_err(|error| error.to_string())?;
    if monitors.is_empty() {
        return Err("No monitors were found".to_string());
    }

    monitors
        .into_iter()
        .enumerate()
        .map(|(index, monitor)| {
            let width = monitor.width().map_err(|error| error.to_string())?;
            let height = monitor.height().map_err(|error| error.to_string())?;
            let name = monitor
                .name()
                .unwrap_or_else(|_| format!("Monitor {}", index + 1));

            Ok(MonitorInfo {
                monitor,
                label: format!("{name} — {width} × {height}").into(),
                width,
                height,
            })
        })
        .collect()
}
