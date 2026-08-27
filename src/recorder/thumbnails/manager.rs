use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
};

use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use gpui::RenderImage;
use parking_lot::Mutex;

use super::{
    ExtractionRequest, ThumbnailEvent, ThumbnailPlan, ThumbnailSlot, ThumbnailStrip, WorkerCommand,
    cache::ThumbnailKey,
    extractor,
    layout::ThumbnailTarget,
    metrics::{self, ThumbnailMetrics},
    state::{State, Status},
};

const REQUEST_CAPACITY: usize = 64;
const RESULT_CAPACITY: usize = 128;

pub(crate) struct ThumbnailManager {
    pub(super) source: PathBuf,
    pub(super) requests: Sender<WorkerCommand>,
    pub(super) results: Receiver<ThumbnailEvent>,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) latest_generation: Arc<AtomicU64>,
    pub(super) state: Mutex<State>,
    pub(super) metrics: ThumbnailMetrics,
    pub(super) worker: Option<JoinHandle<()>>,
    #[cfg(test)]
    _test_commands: Option<Receiver<WorkerCommand>>,
}

impl ThumbnailManager {
    pub(crate) fn new(source: PathBuf) -> Result<Self> {
        let (requests, command_receiver) = bounded(REQUEST_CAPACITY);
        let (events, results) = bounded(RESULT_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let latest_generation = Arc::new(AtomicU64::new(0));
        let worker_stop = stop.clone();
        let worker_generation = latest_generation.clone();
        let worker_source = source.clone();
        let worker = thread::Builder::new()
            .name("recorder-thumbnails".to_string())
            .spawn(move || {
                extractor::run(
                    super::WorkerContext {
                        commands: command_receiver,
                        events,
                        stop: worker_stop,
                        latest_generation: worker_generation,
                    },
                    worker_source,
                );
            })
            .map_err(|error| anyhow!("could not start thumbnail worker: {error}"))?;

        Ok(Self {
            source,
            requests,
            results,
            stop,
            latest_generation,
            state: Mutex::new(State::new(Status::Active)),
            metrics: ThumbnailMetrics::default(),
            worker: Some(worker),
            #[cfg(test)]
            _test_commands: None,
        })
    }

    pub(crate) fn disabled(source: PathBuf) -> Self {
        let (requests, _) = bounded(1);
        let (_, results) = bounded(1);
        Self {
            source,
            requests,
            results,
            stop: Arc::new(AtomicBool::new(true)),
            latest_generation: Arc::new(AtomicU64::new(0)),
            state: Mutex::new(State::new(Status::Unavailable)),
            metrics: ThumbnailMetrics::default(),
            worker: None,
            #[cfg(test)]
            _test_commands: None,
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        self.worker.is_some() && self.state.lock().status == Status::Active
    }

    pub(crate) fn events(&self) -> Receiver<ThumbnailEvent> {
        self.results.clone()
    }

    pub(crate) fn request(&self, plan: &ThumbnailPlan) -> ThumbnailStrip {
        let mut state = self.state.lock();
        if state.status != Status::Active {
            return ThumbnailStrip::default();
        }

        if state.signature != Some(plan.signature) {
            state.signature = Some(plan.signature);
            state.generation = state.generation.saturating_add(1);
            self.latest_generation
                .store(state.generation, Ordering::Release);
        }
        let generation = state.generation;
        state.current_keys.clear();
        let mut slots = Vec::with_capacity(plan.targets.len());
        for target in &plan.targets {
            let key = self.key(target);
            state.current_keys.insert(key.clone());
            let image = state.cache.get(&key);
            if image.is_some() {
                self.metrics.cache_hits.fetch_add(1, Ordering::Relaxed);
            } else {
                self.metrics.cache_misses.fetch_add(1, Ordering::Relaxed);
                self.enqueue(&mut state, key.clone(), target, generation);
            }
            slots.push(ThumbnailSlot {
                start_us: target.start_us,
                end_us: target.end_us,
                image,
            });
        }
        ThumbnailStrip { slots }
    }

    pub(crate) fn take_image_releases(&self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.state.lock().pending_releases)
    }

    fn enqueue(
        &self,
        state: &mut State,
        key: ThumbnailKey,
        target: &ThumbnailTarget,
        generation: u64,
    ) {
        if state.failed.contains(&key) || state.in_flight.contains_key(&key) {
            return;
        }
        let request = ExtractionRequest {
            key: key.clone(),
            timestamp_us: target.timestamp_us,
            size: target.size,
            generation,
        };
        match self.requests.try_send(WorkerCommand::Request(request)) {
            Ok(()) => {
                state.in_flight.insert(key, generation);
                self.metrics.requested.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Full(_)) => {
                self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(target: "recorder::thumbnails", "thumbnail request queue full");
            }
            Err(TrySendError::Disconnected(_)) => {
                state.status = Status::Unavailable;
            }
        }
    }

    fn key(&self, target: &ThumbnailTarget) -> ThumbnailKey {
        ThumbnailKey::new(
            self.source.clone(),
            target.bucket,
            target.interval_us,
            target.size,
        )
    }

    #[cfg(test)]
    fn shutdown_and_join(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.requests.try_send(WorkerCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("thumbnail worker should stop");
        }
    }
}

impl Drop for ThumbnailManager {
    fn drop(&mut self) {
        metrics::report(&self.source, &self.metrics, &self.state.lock().cache);
        self.stop.store(true, Ordering::Release);
        let _ = self.requests.try_send(WorkerCommand::Shutdown);
        let Some(worker) = self.worker.take() else {
            return;
        };
        let _ = thread::Builder::new()
            .name("recorder-thumbnails-reaper".to_string())
            .spawn(move || {
                let _ = worker.join();
            });
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
