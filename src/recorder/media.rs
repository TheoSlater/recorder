use std::{sync::Arc, time::Instant};

use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, Sender, TrySendError};
use gpui::RenderImage;

pub(super) use self::metrics::PlaybackMetrics;
pub(super) use self::native::NativePlayer;

#[path = "media/metrics.rs"]
mod metrics;
mod native;

#[derive(Clone, Debug)]
pub(super) struct FrameTiming {
    pub(super) sequence: u64,
    pub(super) seek_generation: u64,
    pub(super) sample_ready_at: Instant,
    pub(super) buffer_ready_at: Instant,
    pub(super) conversion_completed_at: Instant,
    pub(super) ready_at: Instant,
    pub(super) queued_at: Instant,
    pub(super) scheduled_at: Option<Instant>,
}

#[derive(Clone, Debug)]
pub(super) enum PlaybackEvent {
    Ready {
        duration: f64,
        width: u32,
        height: u32,
    },
    Frame {
        seconds: f64,
        image: Arc<RenderImage>,
        timing: FrameTiming,
    },
    Time {
        seconds: f64,
        seek_generation: u64,
    },
    State(bool),
    Error(String),
}

pub(super) fn build_player(
    path: &std::path::Path,
) -> Result<(NativePlayer, Receiver<PlaybackEvent>)> {
    if !path.is_file() {
        return Err(anyhow!("recording file does not exist: {}", path.display()));
    }

    NativePlayer::open(path)
}

#[derive(Default)]
pub(super) struct QueueStats {
    pub(super) dropped_events: usize,
    pub(super) dropped_frames: usize,
}

pub(super) fn queue_event(
    sender: &Sender<PlaybackEvent>,
    receiver: &Receiver<PlaybackEvent>,
    mut event: PlaybackEvent,
) -> QueueStats {
    let mut stats = QueueStats::default();
    loop {
        match sender.try_send(event) {
            Ok(()) => return stats,
            Err(TrySendError::Full(next)) => {
                if let Ok(dropped) = receiver.try_recv() {
                    stats.dropped_events += 1;
                    if matches!(dropped, PlaybackEvent::Frame { .. }) {
                        stats.dropped_frames += 1;
                    }
                }
                event = next;
            }
            Err(TrySendError::Disconnected(_)) => return stats,
        }
    }
}

pub(super) fn discard_frame_events(
    sender: &Sender<PlaybackEvent>,
    receiver: &Receiver<PlaybackEvent>,
) -> QueueStats {
    let mut stats = QueueStats::default();
    let mut retained = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        if matches!(event, PlaybackEvent::Frame { .. }) {
            stats.dropped_events += 1;
            stats.dropped_frames += 1;
        } else {
            retained.push(event);
        }
    }

    for event in retained {
        let queued = queue_event(sender, receiver, event);
        stats.dropped_events += queued.dropped_events;
        stats.dropped_frames += queued.dropped_frames;
    }
    stats
}
