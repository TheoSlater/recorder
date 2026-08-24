use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;

/// Shared control used by the view and the app-quit hook.
#[derive(Clone)]
pub(crate) struct RecordingControl {
    stop_sender: Sender<()>,
    done_receiver: Receiver<()>,
}

impl RecordingControl {
    pub(crate) fn new(stop_sender: Sender<()>, done_receiver: Receiver<()>) -> Self {
        Self {
            stop_sender,
            done_receiver,
        }
    }

    pub(crate) fn request_stop(&self) {
        let _ = self.stop_sender.try_send(());
    }

    fn done(&self) -> Receiver<()> {
        self.done_receiver.clone()
    }
}

/// Keeps the active worker reachable after GPUI starts tearing down entities.
#[derive(Clone, Default)]
pub(crate) struct ShutdownCoordinator {
    active: Arc<Mutex<Option<RecordingControl>>>,
}

impl ShutdownCoordinator {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn register(&self, control: RecordingControl) {
        *self.active.lock() = Some(control);
    }

    pub(crate) fn clear(&self) {
        *self.active.lock() = None;
    }

    pub(crate) fn stop_and_wait(&self) -> Option<Receiver<()>> {
        let control = self.active.lock().clone()?;
        control.request_stop();
        Some(control.done())
    }
}

#[cfg(test)]
mod tests {
    use super::{RecordingControl, ShutdownCoordinator};
    use crossbeam_channel::bounded;

    #[test]
    fn shutdown_requests_stop_and_waits_for_completion() {
        let (stop_sender, stop_receiver) = bounded(1);
        let (done_sender, done_receiver) = bounded(1);
        let coordinator = ShutdownCoordinator::new();
        coordinator.register(RecordingControl::new(stop_sender, done_receiver));

        let done = coordinator.stop_and_wait().expect("active recording");
        assert!(stop_receiver.try_recv().is_ok());
        done_sender.send(()).unwrap();
        assert!(done.recv().is_ok());
    }
}
