use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use crossbeam_channel::{Receiver, RecvError, Sender, TryRecvError, TrySendError, bounded};
use windows::Win32::{
    Media::MediaFoundation::{MF_VERSION, MFSTARTUP_FULL, MFShutdown, MFStartup},
    System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
};

use self::native_decoder::Decoder;
use super::{FrameTiming, PlaybackEvent, PlaybackMetrics, discard_frame_events, queue_event};

#[path = "native_decoder.rs"]
mod native_decoder;

pub(in crate::recorder) struct NativePlayer {
    commands: Sender<PlayerCommand>,
    pending_seek: Arc<PendingSeek>,
    metrics: PlaybackMetrics,
}

impl NativePlayer {
    pub(in crate::recorder) fn open(
        path: &std::path::Path,
    ) -> Result<(Self, Receiver<PlaybackEvent>)> {
        let (commands, command_receiver) = bounded(16);
        let (events, event_receiver) = bounded(4);
        let worker_events = event_receiver.clone();
        let pending_seek = Arc::new(PendingSeek::default());
        let worker_pending_seek = pending_seek.clone();
        let metrics = PlaybackMetrics::new();
        let worker_metrics = metrics.clone();
        let path = path.to_path_buf();
        let worker_path = path.clone();

        thread::Builder::new()
            .name("recorder-playback".to_string())
            .spawn(move || {
                run_worker(
                    worker_path,
                    command_receiver,
                    events,
                    worker_events,
                    worker_pending_seek,
                    worker_metrics,
                )
            })
            .map_err(|error| {
                tracing::error!(
                    target: "recorder::playback",
                    path = %path.display(),
                    error = %error,
                    "could not start playback worker"
                );
                anyhow!("could not start playback worker: {error}")
            })?;

        Ok((
            Self {
                commands,
                pending_seek,
                metrics,
            },
            event_receiver,
        ))
    }

    pub(in crate::recorder) fn set_playing(&self, playing: bool) -> Result<()> {
        tracing::info!(target: "recorder::playback", playing, "playback state requested");
        self.send(PlayerCommand::SetPlaying(playing))
    }

    pub(in crate::recorder) fn seek(&self, seconds: f64) -> Result<u64> {
        tracing::debug!(target: "recorder::playback", seconds, "playback seek requested");
        let (generation, replaced) = self.pending_seek.request(seconds);
        self.metrics.seek_requested(replaced);
        if !self.pending_seek.wake_queued.swap(true, Ordering::AcqRel) {
            match self.commands.try_send(PlayerCommand::Wake) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    // The worker is already active and checks pending seeks before blocking.
                    self.pending_seek
                        .wake_queued
                        .store(false, Ordering::Release);
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.pending_seek
                        .wake_queued
                        .store(false, Ordering::Release);
                    return Err(anyhow!("playback worker stopped"));
                }
            }
        }
        Ok(generation)
    }

    pub(in crate::recorder) fn metrics(&self) -> PlaybackMetrics {
        self.metrics.clone()
    }

    fn send(&self, command: PlayerCommand) -> Result<()> {
        match self.commands.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(anyhow!("playback worker is busy")),
            Err(TrySendError::Disconnected(_)) => Err(anyhow!("playback worker stopped")),
        }
    }
}

impl Drop for NativePlayer {
    fn drop(&mut self) {
        let _ = self.commands.try_send(PlayerCommand::Shutdown);
    }
}

enum PlayerCommand {
    SetPlaying(bool),
    Wake,
    Shutdown,
}

/// Keeps timeline seeks latest-wins without making the GPUI caller wait for decoding.
///
/// The request itself is entirely atomic. The GPUI thread must not contend with the
/// decoder while publishing a new scrub target; the worker can observe the newest
/// generation before it starts another expensive seek or conversion.
struct PendingSeek {
    epoch: Instant,
    next_generation: AtomicU64,
    published_generation: AtomicU64,
    consumed_generation: AtomicU64,
    target_bits: AtomicU64,
    requested_at_nanos: AtomicU64,
    wake_queued: AtomicBool,
}

impl Default for PendingSeek {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            next_generation: AtomicU64::new(0),
            published_generation: AtomicU64::new(0),
            consumed_generation: AtomicU64::new(0),
            target_bits: AtomicU64::new(0.0f64.to_bits()),
            requested_at_nanos: AtomicU64::new(0),
            wake_queued: AtomicBool::new(false),
        }
    }
}

impl PendingSeek {
    fn request(&self, seconds: f64) -> (u64, bool) {
        let generation = self.next_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let replaced = self.published_generation.load(Ordering::Acquire)
            > self.consumed_generation.load(Ordering::Acquire);
        let now = Instant::now();
        self.target_bits.store(seconds.to_bits(), Ordering::Relaxed);
        self.requested_at_nanos.store(
            now.saturating_duration_since(self.epoch)
                .as_nanos()
                .min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
        self.published_generation
            .store(generation, Ordering::Release);
        (generation, replaced)
    }

    fn take(&self) -> Option<SeekRequest> {
        loop {
            let generation = self.published_generation.load(Ordering::Acquire);
            if generation <= self.consumed_generation.load(Ordering::Acquire) {
                return None;
            }
            let seconds = f64::from_bits(self.target_bits.load(Ordering::Relaxed));
            let requested_at_nanos = self.requested_at_nanos.load(Ordering::Relaxed);
            if self.published_generation.load(Ordering::Acquire) != generation {
                continue;
            }
            self.consumed_generation
                .store(generation, Ordering::Release);
            return Some(SeekRequest {
                seconds,
                generation,
                requested_at: self.epoch + Duration::from_nanos(requested_at_nanos),
            });
        }
    }

    fn is_current(&self, generation: u64) -> bool {
        self.published_generation.load(Ordering::Acquire) == generation
    }

    fn wake_received(&self) {
        self.wake_queued.store(false, Ordering::Release);
    }
}

struct SeekRequest {
    seconds: f64,
    generation: u64,
    requested_at: Instant,
}

fn run_worker(
    path: std::path::PathBuf,
    commands: Receiver<PlayerCommand>,
    events: Sender<PlaybackEvent>,
    event_receiver: Receiver<PlaybackEvent>,
    pending_seek: Arc<PendingSeek>,
    metrics: PlaybackMetrics,
) {
    tracing::info!(
        target: "recorder::playback",
        path = %path.display(),
        "native playback worker started"
    );
    let result = run(
        &path,
        &commands,
        &events,
        &event_receiver,
        &pending_seek,
        &metrics,
    );
    if let Err(error) = result {
        tracing::error!(
            target: "recorder::playback",
            path = %path.display(),
            error = %error,
            "native playback worker failed"
        );
        let dropped = queue_event(
            &events,
            &event_receiver,
            PlaybackEvent::Error(format!("Native playback failed: {error}")),
        );
        if dropped.dropped_events > 0 {
            tracing::warn!(
                target: "recorder::playback",
                dropped = dropped.dropped_events,
                "playback error event displaced queued events"
            );
        }
        let dropped = queue_event(&events, &event_receiver, PlaybackEvent::State(false));
        if dropped.dropped_events > 0 {
            tracing::warn!(
                target: "recorder::playback",
                dropped = dropped.dropped_events,
                "playback stopped event displaced queued events"
            );
        }
    } else {
        tracing::info!(
            target: "recorder::playback",
            path = %path.display(),
            "native playback worker stopped"
        );
    }
    metrics.flush_report();
}

fn run(
    path: &std::path::Path,
    commands: &Receiver<PlayerCommand>,
    events: &Sender<PlaybackEvent>,
    event_receiver: &Receiver<PlaybackEvent>,
    pending_seek: &PendingSeek,
    metrics: &PlaybackMetrics,
) -> Result<()> {
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if com.is_err() {
        bail!("could not initialize COM: {com:?}");
    }
    let _com_guard = ComGuard;

    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
        .map_err(|error| anyhow!("could not initialize Media Foundation: {error}"))?;
    let _media_foundation_guard = MediaFoundationGuard;

    let mut decoder = Decoder::open(path)?;
    tracing::info!(
        target: "recorder::playback",
        path = %path.display(),
        width = decoder.width,
        height = decoder.height,
        duration_seconds = decoder.duration,
        "native playback source opened"
    );
    emit(
        events,
        event_receiver,
        metrics,
        PlaybackEvent::Ready {
            duration: decoder.duration,
            width: decoder.width,
            height: decoder.height,
        },
    );

    let mut position = 0.0;
    let mut sequence = 0;
    let mut seek_generation = 0;
    if let Some(frame) = decoder.next_frame()? {
        position = frame.seconds;
        emit_frame(
            events,
            event_receiver,
            metrics,
            &mut sequence,
            frame,
            seek_generation,
            None,
        );
    }
    emit(events, event_receiver, metrics, PlaybackEvent::State(false));

    let mut playing = false;
    let mut play_anchor = Instant::now();

    'playback: loop {
        if let Some(request) = pending_seek.take() {
            seek_generation = request.generation;
            match apply_seek(
                &mut decoder,
                request.seconds,
                request.generation,
                Some(request.requested_at),
                events,
                event_receiver,
                metrics,
                &mut sequence,
                || pending_seek.is_current(request.generation),
            ) {
                Ok(Some(new_position)) => {
                    position = new_position;
                    if playing {
                        play_anchor = Instant::now() - media_duration(position);
                    }
                }
                Ok(None) => {
                    metrics.seek_skipped();
                }
                Err(error) if !pending_seek.is_current(request.generation) => {
                    metrics.seek_skipped();
                    tracing::debug!(
                        target: "recorder::playback",
                        generation = request.generation,
                        error = %error,
                        "discarding error from stale seek"
                    );
                }
                // A failed seek keeps the worker alive so playback can be retried.
                Err(error) => playback_stopped(
                    &mut playing,
                    events,
                    event_receiver,
                    metrics,
                    format!("Seeking failed: {error}"),
                ),
            }
            continue;
        }

        if !playing {
            match commands.recv() {
                Ok(PlayerCommand::Shutdown) | Err(RecvError) => return Ok(()),
                Ok(PlayerCommand::Wake) => {
                    pending_seek.wake_received();
                }
                Ok(PlayerCommand::SetPlaying(next)) => {
                    if next && position >= decoder.duration && decoder.duration > 0.0 {
                        // Replay from the start when the recording finished.
                        match apply_seek(
                            &mut decoder,
                            0.0,
                            seek_generation,
                            None,
                            events,
                            event_receiver,
                            metrics,
                            &mut sequence,
                            || true,
                        ) {
                            Ok(Some(new_position)) => position = new_position,
                            Ok(None) => {}
                            Err(error) => {
                                playback_stopped(
                                    &mut playing,
                                    events,
                                    event_receiver,
                                    metrics,
                                    format!("Replaying failed: {error}"),
                                );
                                continue;
                            }
                        }
                    }
                    if next {
                        play_anchor = Instant::now() - media_duration(position);
                        playing = true;
                        emit(events, event_receiver, metrics, PlaybackEvent::State(true));
                    }
                }
            }
            continue;
        }

        match commands.try_recv() {
            Ok(PlayerCommand::Shutdown) => return Ok(()),
            Ok(PlayerCommand::SetPlaying(next)) if !next => {
                playing = false;
                emit(events, event_receiver, metrics, PlaybackEvent::State(false));
                continue;
            }
            Ok(PlayerCommand::SetPlaying(true))
                if position >= decoder.duration && decoder.duration > 0.0 =>
            {
                match apply_seek(
                    &mut decoder,
                    0.0,
                    seek_generation,
                    None,
                    events,
                    event_receiver,
                    metrics,
                    &mut sequence,
                    || true,
                ) {
                    Ok(Some(new_position)) => {
                        position = new_position;
                        play_anchor = Instant::now() - media_duration(position);
                        continue 'playback;
                    }
                    Ok(None) => continue 'playback,
                    Err(error) => {
                        playback_stopped(
                            &mut playing,
                            events,
                            event_receiver,
                            metrics,
                            format!("Replaying failed: {error}"),
                        );
                        continue 'playback;
                    }
                }
            }
            Ok(PlayerCommand::SetPlaying(_)) => {}
            Ok(PlayerCommand::Wake) => {
                pending_seek.wake_received();
                continue 'playback;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => return Ok(()),
        }

        let decoded = match decoder.next_frame_for_clock(media_clock(play_anchor)) {
            Ok(decoded) => decoded,
            // A decode hiccup stops playback but must not kill the worker.
            Err(error) => {
                playback_stopped(
                    &mut playing,
                    events,
                    event_receiver,
                    metrics,
                    format!("Decoding failed: {error}"),
                );
                continue;
            }
        };
        let (frame, skipped) = decoded;
        metrics.clock_frames_dropped(skipped);
        let Some(frame) = frame else {
            position = decoder.duration;
            playing = false;
            emit(
                events,
                event_receiver,
                metrics,
                PlaybackEvent::Time {
                    seconds: position,
                    seek_generation,
                },
            );
            emit(events, event_receiver, metrics, PlaybackEvent::State(false));
            continue;
        };

        let deadline = play_anchor + media_duration(frame.seconds);
        while Instant::now() < deadline {
            match commands.try_recv() {
                Ok(PlayerCommand::Shutdown) => return Ok(()),
                Ok(PlayerCommand::SetPlaying(next)) if !next => {
                    playing = false;
                    emit(events, event_receiver, metrics, PlaybackEvent::State(false));
                    continue 'playback;
                }
                Ok(PlayerCommand::SetPlaying(_)) => {}
                Ok(PlayerCommand::Wake) => {
                    pending_seek.wake_received();
                    continue 'playback;
                }
                Err(TryRecvError::Empty) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    thread::sleep(remaining.min(Duration::from_millis(8)));
                }
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        position = frame.seconds;
        emit_frame(
            events,
            event_receiver,
            metrics,
            &mut sequence,
            frame,
            seek_generation,
            Some(deadline),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_seek(
    decoder: &mut Decoder,
    requested: f64,
    seek_generation: u64,
    requested_at: Option<Instant>,
    events: &Sender<PlaybackEvent>,
    event_receiver: &Receiver<PlaybackEvent>,
    metrics: &PlaybackMetrics,
    sequence: &mut u64,
    should_continue: impl Fn() -> bool + Sync,
) -> Result<Option<f64>> {
    let target = if requested.is_finite() {
        requested.max(0.0).min(decoder.duration)
    } else {
        0.0
    };
    if !should_continue() {
        return Ok(None);
    }
    let dropped = discard_frame_events(events, event_receiver);
    metrics.frame_events_dropped(dropped.dropped_frames as u64);
    metrics.queue_depth(events.len());

    if let Some(frame) = decoder.seek(target, &should_continue)? {
        if !should_continue() {
            metrics.seek_decode_cancelled();
            return Ok(None);
        }
        emit_frame(
            events,
            event_receiver,
            metrics,
            sequence,
            frame,
            seek_generation,
            None,
        );
        if let Some(requested_at) = requested_at {
            metrics.seek_completed(requested_at.elapsed());
        }
        // The decoded frame is usually the keyframe before the requested time.
        // Keep the worker's media clock at the requested target so resuming does
        // not replay the entire keyframe-to-target interval.
        Ok(Some(target))
    } else {
        if !should_continue() {
            metrics.seek_decode_cancelled();
            return Ok(None);
        }
        emit(
            events,
            event_receiver,
            metrics,
            PlaybackEvent::Time {
                seconds: target,
                seek_generation,
            },
        );
        if let Some(requested_at) = requested_at {
            metrics.seek_completed(requested_at.elapsed());
        }
        Ok(Some(target))
    }
}

fn emit(
    events: &Sender<PlaybackEvent>,
    event_receiver: &Receiver<PlaybackEvent>,
    metrics: &PlaybackMetrics,
    event: PlaybackEvent,
) {
    let dropped = queue_event(events, event_receiver, event);
    metrics.frame_events_dropped(dropped.dropped_frames as u64);
    metrics.queue_depth(events.len());
}

/// Stops playback on a transient error while keeping the worker alive for retries.
fn playback_stopped(
    playing: &mut bool,
    events: &Sender<PlaybackEvent>,
    event_receiver: &Receiver<PlaybackEvent>,
    metrics: &PlaybackMetrics,
    error: String,
) {
    tracing::error!(
        target: "recorder::playback",
        error = %error,
        "native playback stopped after an error"
    );
    *playing = false;
    emit(events, event_receiver, metrics, PlaybackEvent::Error(error));
    emit(events, event_receiver, metrics, PlaybackEvent::State(false));
}

fn emit_frame(
    events: &Sender<PlaybackEvent>,
    event_receiver: &Receiver<PlaybackEvent>,
    metrics: &PlaybackMetrics,
    sequence: &mut u64,
    frame: native_decoder::DecodedFrame,
    seek_generation: u64,
    scheduled_at: Option<Instant>,
) {
    *sequence += 1;
    metrics.decoded(
        frame.decode_time,
        frame.buffer_copy_time,
        frame.allocation_time,
        frame.conversion_time,
        frame.image_time,
        frame.image_buffer_time,
        frame.render_image_time,
        frame.source_bytes,
        frame.output_bytes,
    );
    let timing = FrameTiming {
        sequence: *sequence,
        seek_generation,
        sample_ready_at: frame.sample_ready_at,
        buffer_ready_at: frame.buffer_ready_at,
        conversion_completed_at: frame.conversion_completed_at,
        ready_at: frame.ready_at,
        queued_at: Instant::now(),
        scheduled_at,
    };
    metrics.frame_queued(&timing);
    let dropped = queue_event(
        events,
        event_receiver,
        PlaybackEvent::Frame {
            seconds: frame.seconds,
            image: frame.image,
            timing,
        },
    );
    metrics.frame_events_dropped(dropped.dropped_frames as u64);
    metrics.queue_depth(events.len());
}

fn media_clock(play_anchor: Instant) -> f64 {
    Instant::now()
        .saturating_duration_since(play_anchor)
        .as_secs_f64()
}

fn media_duration(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds.max(0.0))
}

struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct MediaFoundationGuard;

impl Drop for MediaFoundationGuard {
    fn drop(&mut self) {
        let _ = unsafe { MFShutdown() };
    }
}

#[cfg(test)]
mod tests {
    use super::PendingSeek;

    #[test]
    fn pending_seek_keeps_only_latest_target() {
        let pending = PendingSeek::default();
        let (first_generation, first_replaced) = pending.request(1.2);
        assert!(!first_replaced);

        let (second_generation, second_replaced) = pending.request(1.8);
        assert!(second_replaced);
        assert!(second_generation > first_generation);

        let request = pending.take().expect("latest request should be available");
        assert_eq!(request.generation, second_generation);
        assert_eq!(request.seconds, 1.8);
        assert!(pending.take().is_none());
    }
}
