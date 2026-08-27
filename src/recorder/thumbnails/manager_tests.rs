use std::path::PathBuf;

use crossbeam_channel::bounded;
use parking_lot::Mutex;

use super::{ThumbnailEvent, ThumbnailManager};
use crate::recorder::thumbnails::{ThumbnailSize, TimelineViewport, plan, thumbnail_size};

fn manager() -> ThumbnailManager {
    let (requests, command_receiver) = bounded(64);
    let (_, results) = bounded(128);
    ThumbnailManager {
        source: PathBuf::from("recording.mp4"),
        requests,
        results,
        stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        latest_generation: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        state: Mutex::new(super::State::new(super::Status::Active)),
        metrics: super::ThumbnailMetrics::default(),
        worker: None,
        _test_commands: Some(command_receiver),
    }
}

fn test_plan(scroll_us: u64) -> super::super::layout::ThumbnailPlan {
    plan(
        TimelineViewport {
            duration_us: 120_000_000,
            scroll_us,
            pixels_per_second: 80.,
            width_px: 800.,
        },
        ThumbnailSize {
            width: 114,
            height: 64,
        },
    )
}

#[test]
fn duplicate_requests_are_deduplicated() {
    let manager = manager();
    let plan = test_plan(30_000_000);

    manager.request(&plan);
    manager.request(&plan);

    assert_eq!(manager.requests.len(), plan.targets.len());
    assert_eq!(manager.state.lock().generation, 1);
}

#[test]
fn stale_event_does_not_clear_newer_request() {
    let manager = manager();
    let first = test_plan(30_000_000);
    let second = test_plan(30_500_000);
    let key = manager.key(&first.targets[0]);

    manager.request(&first);
    let old_generation = manager.state.lock().generation;
    manager.request(&second);
    let new_generation = manager.state.lock().generation;
    manager
        .state
        .lock()
        .in_flight
        .insert(key.clone(), new_generation);

    assert!(manager.apply_events([ThumbnailEvent::Stale {
        key: key.clone(),
        generation: old_generation,
    }]));
    assert_eq!(
        manager.state.lock().in_flight.get(&key),
        Some(&new_generation)
    );
}

#[test]
fn worker_shutdown_joins_without_decoding() {
    let mut manager = ThumbnailManager::new(PathBuf::from("target/missing-thumbnail.mp4"))
        .expect("worker thread should start");
    manager.shutdown_and_join();
    assert!(manager.worker.is_none());
}

#[test]
fn test_plan_uses_small_output() {
    assert_eq!(
        thumbnail_size(1920, 1080),
        ThumbnailSize {
            width: 114,
            height: 64
        }
    );
}
