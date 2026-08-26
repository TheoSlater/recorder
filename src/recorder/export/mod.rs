mod decoder;
mod encoder;
mod native;
mod path;
mod renderer;
mod shaders;

use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    thread,
};

use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, Sender, bounded};

use super::project_settings::ProjectSettings;

#[derive(Clone)]
pub(crate) struct ExportRequest {
    pub(crate) video_path: PathBuf,
    pub(crate) telemetry_path: PathBuf,
    pub(crate) metadata_path: PathBuf,
    pub(crate) settings: ProjectSettings,
}

#[derive(Clone, Debug)]
pub(crate) enum ExportEvent {
    Progress { completed: u64, total: u64 },
    Finished(PathBuf),
    Cancelled,
    Error(String),
}

pub(crate) struct ExportHandle {
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) events: Receiver<ExportEvent>,
}

pub(crate) fn choose_output_path(suggested_name: &str) -> Result<Option<PathBuf>> {
    path::choose(suggested_name)
}

pub(crate) fn start(request: ExportRequest, output_path: PathBuf) -> Result<ExportHandle> {
    if request.video_path.as_os_str().is_empty() {
        return Err(anyhow!("recording video path is empty"));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let (sender, events) = bounded(16);
    let worker_cancel = cancel.clone();
    thread::Builder::new()
        .name("recorder-export".to_string())
        .spawn(move || native::run(request, output_path, worker_cancel, sender))
        .map_err(|error| anyhow!("could not start export worker: {error}"))?;
    Ok(ExportHandle { cancel, events })
}

fn temporary_path(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("export.mp4");
    output.with_file_name(format!(".{name}.{}.tmp.mp4", std::process::id()))
}

fn remove_temporary(path: &Path) {
    if let Err(error) = std::fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(target: "recorder::export", path = %path.display(), %error, "could not remove incomplete export");
    }
}

#[cfg(windows)]
fn finalize_temporary(source: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW}, core::PCWSTR};

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe { MoveFileExW(PCWSTR(source.as_ptr()), PCWSTR(target.as_ptr()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) }
        .map_err(|error| anyhow!("could not finalize exported MP4: {error}"))
}

#[cfg(not(windows))]
fn finalize_temporary(source: &Path, target: &Path) -> Result<()> {
    std::fs::rename(source, target).map_err(|error| anyhow!("could not finalize exported MP4: {error}"))
}

fn send_terminal(sender: &Sender<ExportEvent>, event: ExportEvent) {
    let _ = sender.send(event);
}

fn is_cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Acquire)
}
