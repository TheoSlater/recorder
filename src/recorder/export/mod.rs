pub(crate) mod decoder;
mod encoder;
mod frames;
mod native;
mod path;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
};

use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, Sender, bounded};

use super::project_settings::ProjectSettings;

static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    let output_path = mp4_path(output_path)?;
    if same_path(&request.video_path, &output_path) {
        return Err(anyhow!(
            "export destination must be different from the recording"
        ));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let (sender, events) = bounded(16);
    let worker_cancel = cancel.clone();
    let panic_events = sender.clone();
    thread::Builder::new()
        .name("recorder-export".to_string())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                native::run(request, output_path, worker_cancel, sender);
            }));
            if result.is_err() {
                send_terminal(
                    &panic_events,
                    ExportEvent::Error("export worker panicked".to_string()),
                );
            }
        })
        .map_err(|error| anyhow!("could not start export worker: {error}"))?;
    Ok(ExportHandle { cancel, events })
}

fn temporary_path(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("export.mp4");
    let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    output.with_file_name(format!(".{name}.{}-{sequence}.tmp.mp4", std::process::id()))
}

fn mp4_path(mut path: PathBuf) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(anyhow!("export destination is empty"));
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("mp4") => Ok(path),
        None => {
            path.set_extension("mp4");
            Ok(path)
        }
        Some(_) => Err(anyhow!("export destination must use the .mp4 extension")),
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| absolute_path(left));
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| absolute_path(right));
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
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
    use windows::{
        Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        core::PCWSTR,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| anyhow!("could not finalize exported MP4: {error}"))
}

#[cfg(not(windows))]
fn finalize_temporary(source: &Path, target: &Path) -> Result<()> {
    std::fs::rename(source, target)
        .map_err(|error| anyhow!("could not finalize exported MP4: {error}"))
}

fn send_terminal(sender: &Sender<ExportEvent>, event: ExportEvent) {
    let _ = sender.send(event);
}

fn is_cancelled(cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::Acquire)
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
