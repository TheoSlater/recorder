use std::path::PathBuf;

use super::{ExportEvent, ExportRequest, mp4_path, same_path};
use crate::recorder::project;
use crate::recorder::project_settings::CanvasBackgroundKind;

#[test]
fn requires_an_mp4_destination() {
    assert_eq!(
        mp4_path(PathBuf::from("clip")).ok(),
        Some(PathBuf::from("clip.mp4"))
    );
    assert_eq!(
        mp4_path(PathBuf::from("clip.MP4")).ok(),
        Some(PathBuf::from("clip.MP4"))
    );
    assert!(mp4_path(PathBuf::from("clip.mov")).is_err());
    assert!(mp4_path(PathBuf::new()).is_err());
}

/// Exporting over the recording would destroy the source, so the check has to
/// hold for a path spelled differently as well as one spelled the same.
#[test]
fn recognizes_the_same_destination_as_the_recording() {
    let directory = std::env::temp_dir();
    let recording = directory.join("recorder-same-path.mp4");
    std::fs::write(&recording, b"").expect("a file to compare against");

    assert!(same_path(&recording, &recording));
    assert!(same_path(
        &recording,
        &directory.join("nested/../recorder-same-path.MP4")
    ));
    assert!(!same_path(
        &recording,
        &directory.join("recorder-other.mp4")
    ));

    let _ = std::fs::remove_file(&recording);
}

/// Runs a real export end to end.
///
/// Ignored by default: it needs a GPU, Media Foundation, and a recording under
/// `recordings/`, none of which exist on a build machine. Run it after touching
/// the composition renderer, which the editor preview and the exporter share —
/// the preview exercises that code every frame, but only this reaches the
/// encoder.
///
/// `cargo test -- --ignored exports_a_recording`
#[test]
#[ignore = "needs a GPU and a recording under recordings/"]
fn exports_a_recording() {
    let project = newest_recording().expect("a recording under recordings/ to export");
    let video_path = project.join("recording.mp4");
    let settings = project
        .file_name()
        .map(|name| project.join(format!("{}.recproj", name.to_string_lossy())))
        .map(|path| project::load_settings(&path))
        .unwrap_or_default();
    assert_ne!(
        settings.canvas_composition.background.kind,
        CanvasBackgroundKind::Image,
        "an image background needs the file to still exist; \
         re-run against a project with a solid or gradient background"
    );

    let output = std::env::temp_dir().join("recorder-export-test.mp4");
    let _ = std::fs::remove_file(&output);
    let handle = super::start(
        ExportRequest {
            video_path,
            telemetry_path: project.join("telemetry.jsonl"),
            metadata_path: project.join("session.json"),
            settings: settings.normalized(),
        },
        output.clone(),
    )
    .expect("the export worker starts");

    // Bounded, because a stalled Media Foundation call would otherwise hang the
    // whole test run rather than reporting where it stopped.
    const QUIET: std::time::Duration = std::time::Duration::from_secs(60);
    let mut progressed = 0;
    loop {
        let event = match handle.events.recv_timeout(QUIET) {
            Ok(event) => event,
            Err(error) => panic!(
                "the export stopped reporting after {progressed} frames \
                 ({}s of silence): {error}",
                QUIET.as_secs()
            ),
        };
        match event {
            ExportEvent::Progress { completed, .. } => progressed = completed,
            ExportEvent::Finished(path) => {
                let written = std::fs::read(&path).expect("the export exists");
                let _ = std::fs::remove_file(&path);
                assert!(progressed > 0, "no frames were composed");
                // An MP4 opens with a file-type box, so a truncated or
                // header-only write is caught rather than passing on length.
                assert_eq!(&written[4..8], b"ftyp", "not an MP4");
                assert!(
                    written.len() > 64 * 1024,
                    "the export is {} bytes for {progressed} frames",
                    written.len()
                );
                return;
            }
            ExportEvent::Cancelled => panic!("the export cancelled itself"),
            ExportEvent::Error(error) => panic!("export failed: {error}"),
        }
    }
}

fn newest_recording() -> Option<PathBuf> {
    let mut directories: Vec<PathBuf> = std::fs::read_dir("recordings")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("recording.mp4").is_file())
        .collect();
    directories.sort();
    directories.pop()
}
