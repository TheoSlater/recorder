use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;

use super::super::motion_blur::MotionBlurMode;
use super::FrameTiming;

const REPORT_INTERVAL: Duration = Duration::from_secs(1);
const FPS_WINDOW: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub(crate) struct PlaybackMetrics {
    state: Arc<Mutex<State>>,
}

impl PlaybackMetrics {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        Self {
            state: Arc::new(Mutex::new(State::new(now))),
        }
    }

    pub(crate) fn presented_fps(&self) -> f32 {
        let now = Instant::now();
        let mut state = self.state.lock();
        state.prune_presented(now);
        state.presented_fps(now)
    }

    pub(crate) fn reset_presented(&self) {
        let mut state = self.state.lock();
        state.presented_times.clear();
        state.last_presented = None;
    }

    pub(crate) fn decoded(
        &self,
        decode: Duration,
        buffer_copy: Duration,
        allocation: Duration,
        conversion: Duration,
        image_total: Duration,
        image_buffer: Duration,
        render_image: Duration,
        source_bytes: u64,
        output_bytes: u64,
    ) {
        self.update(|state| {
            state.decoded += 1;
            state.bgra_allocations += 1;
            state.render_images += 1;
            state.source_bytes += source_bytes;
            state.output_bytes += output_bytes;
            state.decode_times.push(decode);
            state.buffer_copy_times.push(buffer_copy);
            state.allocation_times.push(allocation);
            state.conversion_times.push(conversion);
            state.image_times.push(image_total);
            state.image_buffer_times.push(image_buffer);
            state.render_image_times.push(render_image);
        });
    }

    pub(crate) fn clock_frames_dropped(&self, count: u64) {
        if count == 0 {
            return;
        }
        self.update(|state| {
            state.dropped += count;
            state.clock_dropped += count;
        });
    }

    pub(crate) fn frame_events_dropped(&self, count: u64) {
        if count == 0 {
            return;
        }
        self.update(|state| {
            state.dropped += count;
            state.queue_dropped += count;
        });
    }

    pub(crate) fn frame_coalesced(&self) {
        self.update(|state| {
            state.dropped += 1;
            state.coalesced += 1;
        });
    }

    pub(crate) fn queue_depth(&self, depth: usize) {
        self.update(|state| {
            state.queue_depth_sum += depth as u64;
            state.queue_depth_samples += 1;
            state.queue_depth_max = state.queue_depth_max.max(depth);
        });
    }

    pub(crate) fn frame_received(&self, timing: &FrameTiming, received_at: Instant) {
        self.update(|state| {
            state
                .event_latencies
                .push(received_at.saturating_duration_since(timing.queued_at));
            state.sample_to_buffer_times.push(
                timing
                    .buffer_ready_at
                    .saturating_duration_since(timing.sample_ready_at),
            );
            state.buffer_to_conversion_times.push(
                timing
                    .conversion_completed_at
                    .saturating_duration_since(timing.buffer_ready_at),
            );
            state.conversion_to_image_times.push(
                timing
                    .ready_at
                    .saturating_duration_since(timing.conversion_completed_at),
            );
            state
                .ready_to_event_times
                .push(received_at.saturating_duration_since(timing.ready_at));
        });
    }

    pub(crate) fn frame_queued(&self, timing: &FrameTiming) {
        self.update(|state| {
            state
                .ready_to_queue_times
                .push(timing.queued_at.saturating_duration_since(timing.ready_at));
            if let Some(deadline) = timing.scheduled_at
                && timing.queued_at > deadline
            {
                state.worker_late += 1;
                state
                    .worker_late_times
                    .push(timing.queued_at.saturating_duration_since(deadline));
            }
        });
    }

    pub(crate) fn frame_invalidated(
        &self,
        timing: &FrameTiming,
        received_at: Instant,
        invalidated_at: Instant,
    ) {
        self.update(|state| {
            state
                .event_to_invalidate_times
                .push(invalidated_at.saturating_duration_since(received_at));
            state
                .ready_to_invalidate_times
                .push(invalidated_at.saturating_duration_since(timing.ready_at));
        });
    }

    pub(crate) fn gpui_update(&self, duration: Duration) {
        self.update(|state| state.update_times.push(duration));
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn presented(
        &self,
        frame_id: u64,
        ready_at: Instant,
        invalidated_at: Instant,
        scheduled_at: Option<Instant>,
        paint_started_at: Instant,
        canvas_paint_duration: Duration,
        composition_paint_duration: Duration,
        image_submission_duration: Duration,
        now: Instant,
    ) {
        let report = {
            let mut state = self.state.lock();
            if state.last_presented_id == Some(frame_id) {
                return;
            }

            state.presented += 1;
            if let Some(last) = state.last_presented {
                state.frame_times.push(now.saturating_duration_since(last));
            }
            state.last_presented = Some(now);
            state.last_presented_id = Some(frame_id);
            state.presented_times.push_back(now);
            state
                .presentation_latencies
                .push(now.saturating_duration_since(ready_at));
            state
                .invalidate_to_paint_times
                .push(paint_started_at.saturating_duration_since(invalidated_at));
            state.canvas_paint_times.push(canvas_paint_duration);
            state
                .composition_paint_times
                .push(composition_paint_duration);
            state.image_submission_times.push(image_submission_duration);
            if let Some(deadline) = scheduled_at
                && now > deadline
            {
                state.late += 1;
                state
                    .late_times
                    .push(now.saturating_duration_since(deadline));
            }
            state.take_report(now)
        };
        log_report(report);
    }

    pub(crate) fn image_released(&self, duration: Duration) {
        self.update(|state| state.release_times.push(duration));
    }

    pub(crate) fn stale_event_dropped(&self) {
        self.update(|state| {
            state.dropped += 1;
            state.stale_events += 1;
        });
    }

    pub(crate) fn paint_failed(&self) {
        self.update(|state| state.paint_failures += 1);
    }

    pub(crate) fn cursor_painted(&self, duration: Duration) {
        self.update(|state| state.cursor_paint_times.push(duration));
    }

    pub(crate) fn cursor_updated(&self, duration: Duration) {
        self.update(|state| state.cursor_update_times.push(duration));
    }

    /// Time spent classifying motion and building the smeared cursor sprite for
    /// one presented frame.
    pub(crate) fn motion_blur_prepared(&self, duration: Duration) {
        self.update(|state| state.motion_blur_times.push(duration));
    }

    /// Counts how each presented frame was classified, so blurred frames can be
    /// compared against sharp ones in the same report.
    pub(crate) fn motion_blur_classified(&self, mode: MotionBlurMode) {
        self.update(|state| match mode {
            MotionBlurMode::Movement => state.movement_blur_frames += 1,
            MotionBlurMode::Zoom => state.zoom_blur_frames += 1,
            MotionBlurMode::None => {}
        });
    }

    pub(crate) fn timeline_painted(&self, duration: Duration) {
        self.update(|state| state.timeline_paint_times.push(duration));
    }

    pub(crate) fn seek_completed(&self, latency: Duration) {
        self.update(|state| state.seek_latencies.push(latency));
    }

    pub(crate) fn seek_requested(&self, replaced: bool) {
        self.update(|state| {
            state.seek_requests += 1;
            state.seek_replaced += u64::from(replaced);
        });
    }

    pub(crate) fn seek_skipped(&self) {
        self.update(|state| state.seek_skipped += 1);
    }

    pub(crate) fn seek_decode_cancelled(&self) {
        self.update(|state| state.seek_decode_cancelled += 1);
    }

    pub(crate) fn scrub_pointer_moved(&self) {
        self.update(|state| state.scrub_pointer_moves += 1);
    }

    pub(crate) fn scrub_seek_published(&self, latency: Duration) {
        self.update(|state| {
            state.scrub_seek_requests += 1;
            state.scrub_pointer_to_seek_times.push(latency);
        });
    }

    pub(crate) fn flush_report(&self) {
        let report = self.state.lock().take_report(Instant::now());
        log_report(report);
    }

    fn update(&self, update: impl FnOnce(&mut State)) {
        let report = {
            let mut state = self.state.lock();
            update(&mut state);
            state.take_report(Instant::now())
        };
        log_report(report);
    }
}

impl Default for PlaybackMetrics {
    fn default() -> Self {
        Self::new()
    }
}

struct State {
    last_report: Instant,
    decoded: u64,
    bgra_allocations: u64,
    render_images: u64,
    source_bytes: u64,
    output_bytes: u64,
    presented: u64,
    dropped: u64,
    clock_dropped: u64,
    queue_dropped: u64,
    stale_events: u64,
    coalesced: u64,
    paint_failures: u64,
    late: u64,
    worker_late: u64,
    last_presented: Option<Instant>,
    last_presented_id: Option<u64>,
    presented_times: VecDeque<Instant>,
    frame_times: Vec<Duration>,
    presentation_latencies: Vec<Duration>,
    decode_times: Vec<Duration>,
    buffer_copy_times: Vec<Duration>,
    allocation_times: Vec<Duration>,
    conversion_times: Vec<Duration>,
    image_times: Vec<Duration>,
    image_buffer_times: Vec<Duration>,
    render_image_times: Vec<Duration>,
    event_latencies: Vec<Duration>,
    sample_to_buffer_times: Vec<Duration>,
    buffer_to_conversion_times: Vec<Duration>,
    conversion_to_image_times: Vec<Duration>,
    ready_to_event_times: Vec<Duration>,
    ready_to_queue_times: Vec<Duration>,
    event_to_invalidate_times: Vec<Duration>,
    ready_to_invalidate_times: Vec<Duration>,
    invalidate_to_paint_times: Vec<Duration>,
    update_times: Vec<Duration>,
    canvas_paint_times: Vec<Duration>,
    composition_paint_times: Vec<Duration>,
    image_submission_times: Vec<Duration>,
    release_times: Vec<Duration>,
    late_times: Vec<Duration>,
    worker_late_times: Vec<Duration>,
    cursor_paint_times: Vec<Duration>,
    cursor_update_times: Vec<Duration>,
    motion_blur_times: Vec<Duration>,
    movement_blur_frames: u64,
    zoom_blur_frames: u64,
    timeline_paint_times: Vec<Duration>,
    seek_latencies: Vec<Duration>,
    scrub_pointer_to_seek_times: Vec<Duration>,
    seek_requests: u64,
    seek_replaced: u64,
    seek_skipped: u64,
    seek_decode_cancelled: u64,
    scrub_pointer_moves: u64,
    scrub_seek_requests: u64,
    queue_depth_sum: u64,
    queue_depth_samples: u64,
    queue_depth_max: usize,
}

impl State {
    fn new(now: Instant) -> Self {
        Self {
            last_report: now,
            decoded: 0,
            bgra_allocations: 0,
            render_images: 0,
            source_bytes: 0,
            output_bytes: 0,
            presented: 0,
            dropped: 0,
            clock_dropped: 0,
            queue_dropped: 0,
            stale_events: 0,
            coalesced: 0,
            paint_failures: 0,
            late: 0,
            worker_late: 0,
            last_presented: None,
            last_presented_id: None,
            presented_times: VecDeque::new(),
            frame_times: Vec::new(),
            presentation_latencies: Vec::new(),
            decode_times: Vec::new(),
            buffer_copy_times: Vec::new(),
            allocation_times: Vec::new(),
            conversion_times: Vec::new(),
            image_times: Vec::new(),
            image_buffer_times: Vec::new(),
            render_image_times: Vec::new(),
            event_latencies: Vec::new(),
            sample_to_buffer_times: Vec::new(),
            buffer_to_conversion_times: Vec::new(),
            conversion_to_image_times: Vec::new(),
            ready_to_event_times: Vec::new(),
            ready_to_queue_times: Vec::new(),
            event_to_invalidate_times: Vec::new(),
            ready_to_invalidate_times: Vec::new(),
            invalidate_to_paint_times: Vec::new(),
            update_times: Vec::new(),
            canvas_paint_times: Vec::new(),
            composition_paint_times: Vec::new(),
            image_submission_times: Vec::new(),
            release_times: Vec::new(),
            late_times: Vec::new(),
            worker_late_times: Vec::new(),
            cursor_paint_times: Vec::new(),
            cursor_update_times: Vec::new(),
            motion_blur_times: Vec::new(),
            movement_blur_frames: 0,
            zoom_blur_frames: 0,
            timeline_paint_times: Vec::new(),
            seek_latencies: Vec::new(),
            scrub_pointer_to_seek_times: Vec::new(),
            seek_requests: 0,
            seek_replaced: 0,
            seek_skipped: 0,
            seek_decode_cancelled: 0,
            scrub_pointer_moves: 0,
            scrub_seek_requests: 0,
            queue_depth_sum: 0,
            queue_depth_samples: 0,
            queue_depth_max: 0,
        }
    }

    fn prune_presented(&mut self, now: Instant) {
        while self
            .presented_times
            .front()
            .is_some_and(|time| now.saturating_duration_since(*time) > FPS_WINDOW)
        {
            self.presented_times.pop_front();
        }
    }

    fn presented_fps(&self, now: Instant) -> f32 {
        let Some(first) = self.presented_times.front() else {
            return 0.0;
        };
        let elapsed = now.saturating_duration_since(*first).as_secs_f32();
        if self.presented_times.len() < 2 || elapsed <= 0.0 {
            0.0
        } else {
            (self.presented_times.len() - 1) as f32 / elapsed
        }
    }

    fn take_report(&mut self, now: Instant) -> Option<Report> {
        let elapsed = now.saturating_duration_since(self.last_report);
        if elapsed < REPORT_INTERVAL {
            return None;
        }

        self.prune_presented(now);
        let report = Report {
            elapsed,
            decoded: self.decoded,
            bgra_allocations: self.bgra_allocations,
            render_images: self.render_images,
            source_bytes: self.source_bytes,
            output_bytes: self.output_bytes,
            presented: self.presented,
            dropped: self.dropped,
            clock_dropped: self.clock_dropped,
            queue_dropped: self.queue_dropped,
            stale_events: self.stale_events,
            coalesced: self.coalesced,
            paint_failures: self.paint_failures,
            late: self.late,
            worker_late: self.worker_late,
            fps: self.presented_fps(now),
            frame_times: Summary::from(&mut self.frame_times),
            presentation_latencies: Summary::from(&mut self.presentation_latencies),
            decode_times: Summary::from(&mut self.decode_times),
            buffer_copy_times: Summary::from(&mut self.buffer_copy_times),
            allocation_times: Summary::from(&mut self.allocation_times),
            conversion_times: Summary::from(&mut self.conversion_times),
            image_times: Summary::from(&mut self.image_times),
            image_buffer_times: Summary::from(&mut self.image_buffer_times),
            render_image_times: Summary::from(&mut self.render_image_times),
            event_latencies: Summary::from(&mut self.event_latencies),
            sample_to_buffer_times: Summary::from(&mut self.sample_to_buffer_times),
            buffer_to_conversion_times: Summary::from(&mut self.buffer_to_conversion_times),
            conversion_to_image_times: Summary::from(&mut self.conversion_to_image_times),
            ready_to_event_times: Summary::from(&mut self.ready_to_event_times),
            ready_to_queue_times: Summary::from(&mut self.ready_to_queue_times),
            event_to_invalidate_times: Summary::from(&mut self.event_to_invalidate_times),
            ready_to_invalidate_times: Summary::from(&mut self.ready_to_invalidate_times),
            invalidate_to_paint_times: Summary::from(&mut self.invalidate_to_paint_times),
            update_times: Summary::from(&mut self.update_times),
            canvas_paint_times: Summary::from(&mut self.canvas_paint_times),
            composition_paint_times: Summary::from(&mut self.composition_paint_times),
            image_submission_times: Summary::from(&mut self.image_submission_times),
            release_times: Summary::from(&mut self.release_times),
            late_times: Summary::from(&mut self.late_times),
            worker_late_times: Summary::from(&mut self.worker_late_times),
            cursor_paint_times: Summary::from(&mut self.cursor_paint_times),
            cursor_update_times: Summary::from(&mut self.cursor_update_times),
            motion_blur_times: Summary::from(&mut self.motion_blur_times),
            movement_blur_frames: self.movement_blur_frames,
            zoom_blur_frames: self.zoom_blur_frames,
            timeline_paint_times: Summary::from(&mut self.timeline_paint_times),
            seek_latencies: Summary::from(&mut self.seek_latencies),
            scrub_pointer_to_seek_times: Summary::from(&mut self.scrub_pointer_to_seek_times),
            seek_requests: self.seek_requests,
            seek_replaced: self.seek_replaced,
            seek_skipped: self.seek_skipped,
            seek_decode_cancelled: self.seek_decode_cancelled,
            scrub_pointer_moves: self.scrub_pointer_moves,
            scrub_seek_requests: self.scrub_seek_requests,
            queue_depth: if self.queue_depth_samples == 0 {
                0.0
            } else {
                self.queue_depth_sum as f64 / self.queue_depth_samples as f64
            },
            queue_depth_max: self.queue_depth_max,
        };
        self.last_report = now;
        self.decoded = 0;
        self.bgra_allocations = 0;
        self.render_images = 0;
        self.source_bytes = 0;
        self.output_bytes = 0;
        self.presented = 0;
        self.dropped = 0;
        self.clock_dropped = 0;
        self.queue_dropped = 0;
        self.stale_events = 0;
        self.coalesced = 0;
        self.movement_blur_frames = 0;
        self.zoom_blur_frames = 0;
        self.paint_failures = 0;
        self.late = 0;
        self.worker_late = 0;
        self.seek_requests = 0;
        self.seek_replaced = 0;
        self.seek_skipped = 0;
        self.seek_decode_cancelled = 0;
        self.scrub_pointer_moves = 0;
        self.scrub_seek_requests = 0;
        self.queue_depth_sum = 0;
        self.queue_depth_samples = 0;
        self.queue_depth_max = 0;
        Some(report)
    }
}

struct Report {
    elapsed: Duration,
    decoded: u64,
    bgra_allocations: u64,
    render_images: u64,
    source_bytes: u64,
    output_bytes: u64,
    presented: u64,
    dropped: u64,
    clock_dropped: u64,
    queue_dropped: u64,
    stale_events: u64,
    coalesced: u64,
    paint_failures: u64,
    late: u64,
    worker_late: u64,
    fps: f32,
    frame_times: Summary,
    presentation_latencies: Summary,
    decode_times: Summary,
    buffer_copy_times: Summary,
    allocation_times: Summary,
    conversion_times: Summary,
    image_times: Summary,
    image_buffer_times: Summary,
    render_image_times: Summary,
    event_latencies: Summary,
    sample_to_buffer_times: Summary,
    buffer_to_conversion_times: Summary,
    conversion_to_image_times: Summary,
    ready_to_event_times: Summary,
    ready_to_queue_times: Summary,
    event_to_invalidate_times: Summary,
    ready_to_invalidate_times: Summary,
    invalidate_to_paint_times: Summary,
    update_times: Summary,
    canvas_paint_times: Summary,
    composition_paint_times: Summary,
    image_submission_times: Summary,
    release_times: Summary,
    late_times: Summary,
    worker_late_times: Summary,
    cursor_paint_times: Summary,
    cursor_update_times: Summary,
    motion_blur_times: Summary,
    movement_blur_frames: u64,
    zoom_blur_frames: u64,
    timeline_paint_times: Summary,
    seek_latencies: Summary,
    scrub_pointer_to_seek_times: Summary,
    seek_requests: u64,
    seek_replaced: u64,
    seek_skipped: u64,
    seek_decode_cancelled: u64,
    scrub_pointer_moves: u64,
    scrub_seek_requests: u64,
    queue_depth: f64,
    queue_depth_max: usize,
}

#[derive(Default)]
struct Summary {
    p50: f64,
    p95: f64,
    p99: f64,
    worst: f64,
}

impl Summary {
    fn from(values: &mut Vec<Duration>) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        values.sort_unstable();
        let summary = Self {
            p50: percentile(values, 0.50),
            p95: percentile(values, 0.95),
            p99: percentile(values, 0.99),
            worst: values.last().unwrap().as_secs_f64() * 1_000.0,
        };
        values.clear();
        summary
    }
}

fn percentile(values: &[Duration], percentile: f64) -> f64 {
    let index = ((values.len() - 1) as f64 * percentile).round() as usize;
    values[index].as_secs_f64() * 1_000.0
}

fn log_report(report: Option<Report>) {
    let Some(report) = report else {
        return;
    };
    let seconds = report.elapsed.as_secs_f64().max(f64::EPSILON);
    tracing::info!(
        target: "recorder::playback",
        "playback metrics: decoded={:.1}/s presented={:.1}/s fps={:.1} dropped={} (clock={} queue={} coalesced={} stale={}) late={} worker_late={} paint_failures={} queue={:.2}/{} allocs={} render_images={} source_mb={:.1} bgra_mb={:.1} frame_ms={:.2}/{:.2}/{:.2}/{:.2} latency_ms={:.2}/{:.2}/{:.2}/{:.2} decode_ms={:.2}/{:.2}/{:.2}/{:.2} copy_ms={:.2}/{:.2}/{:.2}/{:.2} alloc_ms={:.2}/{:.2}/{:.2}/{:.2} convert_ms={:.2}/{:.2}/{:.2}/{:.2} image_ms={:.2}/{:.2}/{:.2}/{:.2} image_buffer_ms={:.2}/{:.2}/{:.2}/{:.2} render_image_ms={:.2}/{:.2}/{:.2}/{:.2} event_ms={:.2}/{:.2}/{:.2}/{:.2} pipeline_ms sample_buffer={:.2}/{:.2}/{:.2}/{:.2} buffer_convert={:.2}/{:.2}/{:.2}/{:.2} convert_image={:.2}/{:.2}/{:.2}/{:.2} ready_queue={:.2}/{:.2}/{:.2}/{:.2} ready_event={:.2}/{:.2}/{:.2}/{:.2} event_invalidate={:.2}/{:.2}/{:.2}/{:.2} ready_invalidate={:.2}/{:.2}/{:.2}/{:.2} invalidate_paint={:.2}/{:.2}/{:.2}/{:.2} update_ms={:.2}/{:.2}/{:.2}/{:.2} canvas_paint_ms={:.2}/{:.2}/{:.2}/{:.2} composition_paint_ms={:.2}/{:.2}/{:.2}/{:.2} image_submit_ms={:.2}/{:.2}/{:.2}/{:.2} release_ms={:.2}/{:.2}/{:.2}/{:.2} late_ms={:.2}/{:.2}/{:.2}/{:.2} worker_late_ms={:.2}/{:.2}/{:.2}/{:.2} cursor_paint_ms={:.2}/{:.2}/{:.2}/{:.2} cursor_update_ms={:.2}/{:.2}/{:.2}/{:.2} motion_blur_ms={:.2}/{:.2}/{:.2}/{:.2} blur_frames={}/{} timeline_paint_ms={:.2}/{:.2}/{:.2}/{:.2} seek_ms={:.2}/{:.2}/{:.2}/{:.2} scrub_moves={} scrub_seeks={} pointer_seek_ms={:.2}/{:.2}/{:.2}/{:.2} seek_requests={} seek_replaced={} seek_skipped={} seek_cancelled={}",
        report.decoded as f64 / seconds,
        report.presented as f64 / seconds,
        report.fps,
        report.dropped,
        report.clock_dropped,
        report.queue_dropped,
        report.coalesced,
        report.stale_events,
        report.late,
        report.worker_late,
        report.paint_failures,
        report.queue_depth,
        report.queue_depth_max,
        report.bgra_allocations,
        report.render_images,
        report.source_bytes as f64 / 1_000_000.,
        report.output_bytes as f64 / 1_000_000.,
        report.frame_times.p50,
        report.frame_times.p95,
        report.frame_times.p99,
        report.frame_times.worst,
        report.presentation_latencies.p50,
        report.presentation_latencies.p95,
        report.presentation_latencies.p99,
        report.presentation_latencies.worst,
        report.decode_times.p50,
        report.decode_times.p95,
        report.decode_times.p99,
        report.decode_times.worst,
        report.buffer_copy_times.p50,
        report.buffer_copy_times.p95,
        report.buffer_copy_times.p99,
        report.buffer_copy_times.worst,
        report.allocation_times.p50,
        report.allocation_times.p95,
        report.allocation_times.p99,
        report.allocation_times.worst,
        report.conversion_times.p50,
        report.conversion_times.p95,
        report.conversion_times.p99,
        report.conversion_times.worst,
        report.image_times.p50,
        report.image_times.p95,
        report.image_times.p99,
        report.image_times.worst,
        report.image_buffer_times.p50,
        report.image_buffer_times.p95,
        report.image_buffer_times.p99,
        report.image_buffer_times.worst,
        report.render_image_times.p50,
        report.render_image_times.p95,
        report.render_image_times.p99,
        report.render_image_times.worst,
        report.event_latencies.p50,
        report.event_latencies.p95,
        report.event_latencies.p99,
        report.event_latencies.worst,
        report.sample_to_buffer_times.p50,
        report.sample_to_buffer_times.p95,
        report.sample_to_buffer_times.p99,
        report.sample_to_buffer_times.worst,
        report.buffer_to_conversion_times.p50,
        report.buffer_to_conversion_times.p95,
        report.buffer_to_conversion_times.p99,
        report.buffer_to_conversion_times.worst,
        report.conversion_to_image_times.p50,
        report.conversion_to_image_times.p95,
        report.conversion_to_image_times.p99,
        report.conversion_to_image_times.worst,
        report.ready_to_queue_times.p50,
        report.ready_to_queue_times.p95,
        report.ready_to_queue_times.p99,
        report.ready_to_queue_times.worst,
        report.ready_to_event_times.p50,
        report.ready_to_event_times.p95,
        report.ready_to_event_times.p99,
        report.ready_to_event_times.worst,
        report.event_to_invalidate_times.p50,
        report.event_to_invalidate_times.p95,
        report.event_to_invalidate_times.p99,
        report.event_to_invalidate_times.worst,
        report.ready_to_invalidate_times.p50,
        report.ready_to_invalidate_times.p95,
        report.ready_to_invalidate_times.p99,
        report.ready_to_invalidate_times.worst,
        report.invalidate_to_paint_times.p50,
        report.invalidate_to_paint_times.p95,
        report.invalidate_to_paint_times.p99,
        report.invalidate_to_paint_times.worst,
        report.update_times.p50,
        report.update_times.p95,
        report.update_times.p99,
        report.update_times.worst,
        report.canvas_paint_times.p50,
        report.canvas_paint_times.p95,
        report.canvas_paint_times.p99,
        report.canvas_paint_times.worst,
        report.composition_paint_times.p50,
        report.composition_paint_times.p95,
        report.composition_paint_times.p99,
        report.composition_paint_times.worst,
        report.image_submission_times.p50,
        report.image_submission_times.p95,
        report.image_submission_times.p99,
        report.image_submission_times.worst,
        report.release_times.p50,
        report.release_times.p95,
        report.release_times.p99,
        report.release_times.worst,
        report.late_times.p50,
        report.late_times.p95,
        report.late_times.p99,
        report.late_times.worst,
        report.worker_late_times.p50,
        report.worker_late_times.p95,
        report.worker_late_times.p99,
        report.worker_late_times.worst,
        report.cursor_paint_times.p50,
        report.cursor_paint_times.p95,
        report.cursor_paint_times.p99,
        report.cursor_paint_times.worst,
        report.cursor_update_times.p50,
        report.cursor_update_times.p95,
        report.cursor_update_times.p99,
        report.cursor_update_times.worst,
        report.motion_blur_times.p50,
        report.motion_blur_times.p95,
        report.motion_blur_times.p99,
        report.motion_blur_times.worst,
        report.movement_blur_frames,
        report.zoom_blur_frames,
        report.timeline_paint_times.p50,
        report.timeline_paint_times.p95,
        report.timeline_paint_times.p99,
        report.timeline_paint_times.worst,
        report.seek_latencies.p50,
        report.seek_latencies.p95,
        report.seek_latencies.p99,
        report.seek_latencies.worst,
        report.scrub_pointer_moves,
        report.scrub_seek_requests,
        report.scrub_pointer_to_seek_times.p50,
        report.scrub_pointer_to_seek_times.p95,
        report.scrub_pointer_to_seek_times.p99,
        report.scrub_pointer_to_seek_times.worst,
        report.seek_requests,
        report.seek_replaced,
        report.seek_skipped,
        report.seek_decode_cancelled,
    );
}
