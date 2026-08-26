use std::{
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use parking_lot::{Condvar, Mutex};

use super::project_settings::{self, ProjectSettings};

const SAVE_DEBOUNCE: Duration = Duration::from_millis(150);

/// Keeps editor saves off the GPUI thread while retaining only the newest
/// snapshot that has not started writing yet.
pub(super) struct ProjectSaveQueue {
    inner: Arc<SaveState>,
    owner: bool,
}

struct SaveState {
    value: Mutex<SaveStateValue>,
    wake: Condvar,
}

struct SaveStateValue {
    generation: u64,
    pending: Option<SaveRequest>,
    result: Option<SaveResult>,
    stopping: bool,
}

struct SaveRequest {
    generation: u64,
    settings: ProjectSettings,
}

struct SaveResult {
    generation: u64,
    error: Option<String>,
}

impl ProjectSaveQueue {
    pub(super) fn new(path: PathBuf) -> Self {
        let inner = Arc::new(SaveState::new());
        let worker_state = inner.clone();
        if let Err(error) = thread::Builder::new()
            .name("recorder-project-save".to_string())
            .spawn(move || run_worker(path, worker_state))
        {
            inner.value.lock().result = Some(SaveResult {
                generation: 0,
                error: Some(format!("could not start project save worker: {error}")),
            });
        }
        Self { inner, owner: true }
    }

    pub(super) fn request(&self, settings: &ProjectSettings) {
        let mut value = self.inner.value.lock();
        value.generation = value.generation.saturating_add(1).max(1);
        let generation = value.generation;
        value.pending = Some(SaveRequest {
            generation,
            settings: settings.clone(),
        });
        self.inner.wake.notify_one();
    }

    pub(super) fn take_error(&self) -> Option<String> {
        let mut value = self.inner.value.lock();
        let result = value.result.take()?;
        (result.generation == value.generation)
            .then_some(result.error)
            .flatten()
    }
}

impl Drop for ProjectSaveQueue {
    fn drop(&mut self) {
        if !self.owner {
            return;
        }
        let mut value = self.inner.value.lock();
        value.stopping = true;
        self.inner.wake.notify_one();
    }
}

impl Clone for ProjectSaveQueue {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            owner: false,
        }
    }
}

impl SaveState {
    fn new() -> Self {
        Self {
            value: Mutex::new(SaveStateValue {
                generation: 0,
                pending: None,
                result: None,
                stopping: false,
            }),
            wake: Condvar::new(),
        }
    }
}

fn run_worker(path: PathBuf, state: Arc<SaveState>) {
    while let Some(mut request) = next_request(&state) {
        loop {
            let mut value = state.value.lock();
            if let Some(next) = value.pending.take() {
                request = next;
                continue;
            }
            if value.stopping {
                break;
            }
            if state.wake.wait_for(&mut value, SAVE_DEBOUNCE).timed_out() {
                break;
            }
        }

        let started = Instant::now();
        let error = project_settings::save(&path, &request.settings).err();
        tracing::debug!(
            target: "recorder::project",
            generation = request.generation,
            elapsed_ms = started.elapsed().as_secs_f64() * 1_000.,
            failed = error.is_some(),
            "project settings save completed"
        );

        let mut value = state.value.lock();
        value.result = Some(SaveResult {
            generation: request.generation,
            error,
        });
        if value.stopping && value.pending.is_none() {
            return;
        }
    }
}

fn next_request(state: &SaveState) -> Option<SaveRequest> {
    let mut value = state.value.lock();
    loop {
        if let Some(request) = value.pending.take() {
            return Some(request);
        }
        if value.stopping {
            return None;
        }
        state.wake.wait(&mut value);
    }
}

#[cfg(test)]
mod tests {
    use super::SaveState;
    use crate::recorder::project_settings::ProjectSettings;

    #[test]
    fn replaces_pending_snapshot() {
        let state = SaveState::new();
        let first = request(&state);
        let second = request(&state);

        let pending = state.value.lock().pending.take().unwrap();
        assert_eq!(first + 1, second);
        assert_eq!(pending.generation, second);
    }

    #[test]
    fn ignores_result_from_an_older_generation() {
        let state = SaveState::new();
        let first = request(&state);
        state.value.lock().result = Some(super::SaveResult {
            generation: first,
            error: Some("old".to_string()),
        });
        let _second = request(&state);

        assert!(
            super::ProjectSaveQueue {
                inner: std::sync::Arc::new(state),
                owner: false,
            }
            .take_error()
            .is_none()
        );
    }

    fn request(state: &SaveState) -> u64 {
        let mut value = state.value.lock();
        value.generation = value.generation.saturating_add(1).max(1);
        let generation = value.generation;
        value.pending = Some(super::SaveRequest {
            generation,
            settings: ProjectSettings::default(),
        });
        generation
    }
}
