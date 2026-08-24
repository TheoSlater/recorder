use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
    VideoSettingsSubType,
};
use windows_capture::frame::Frame;

pub(crate) const QUEUE_CAPACITY: usize = 2;

type FrameResult = Result<(), String>;

/// A borrowed GPU frame is handed to the encoder thread and acknowledged before
/// the capture callback returns. The queue never owns pixel data.
pub(crate) struct FrameTask {
    frame: *const (),
    complete: Sender<FrameResult>,
}

// The capture callback waits for completion, so the pointer cannot outlive the
// callback's Frame. The encoder only reads the frame while that wait is active.
unsafe impl Send for FrameTask {}

impl FrameTask {
    pub(crate) fn new(frame: &Frame<'_>, complete: Sender<FrameResult>) -> Self {
        Self {
            frame: frame as *const Frame<'_> as *const (),
            complete,
        }
    }
}

#[derive(Default)]
pub(crate) struct QueueStats {
    dropped: AtomicU64,
}

impl QueueStats {
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub(crate) fn record_drop(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) struct EncoderPump {
    join: Option<JoinHandle<Result<(), String>>>,
}

impl EncoderPump {
    pub(crate) fn new(
        encoder: VideoEncoder,
        receiver: Receiver<FrameTask>,
    ) -> Result<Self, String> {
        let join = thread::Builder::new()
            .name("gpu-encoder-handoff".to_string())
            .spawn(move || {
                catch_unwind(AssertUnwindSafe(|| run(encoder, receiver)))
                    .unwrap_or_else(|_| Err("encoder pump panicked".to_string()))
            })
            .map_err(|error| format!("failed to start encoder pump: {error}"))?;
        Ok(Self { join: Some(join) })
    }

    pub(crate) fn submit(
        sender: &Sender<FrameTask>,
        frame: &Frame<'_>,
        stats: &QueueStats,
    ) -> Result<bool, String> {
        let (complete_sender, complete_receiver) = bounded(1);
        let task = FrameTask::new(frame, complete_sender);
        match sender.try_send(task) {
            Ok(()) => complete_receiver
                .recv()
                .map(|result| result.map(|()| true))
                .map_err(|_| "encoder pump stopped".to_string())?,
            Err(TrySendError::Full(_)) => {
                stats.record_drop();
                Ok(false)
            }
            Err(TrySendError::Disconnected(_)) => Err("encoder queue stopped".to_string()),
        }
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.join
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
    }

    pub(crate) fn finish(mut self) -> Result<(), String> {
        self.join
            .take()
            .expect("encoder pump join handle missing")
            .join()
            .map_err(|_| "encoder pump panicked".to_string())?
    }
}

impl Drop for EncoderPump {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub(crate) fn new_encoder(width: u32, height: u32, path: &Path) -> Result<VideoEncoder, String> {
    VideoEncoder::new(
        VideoSettingsBuilder::new(width, height)
            .sub_type(VideoSettingsSubType::H264)
            .frame_rate(60),
        AudioSettingsBuilder::default().disabled(true),
        ContainerSettingsBuilder::default(),
        path,
    )
    .map_err(|error| format!("failed to create encoder: {error}"))
}

pub(crate) fn channel() -> (Sender<FrameTask>, Receiver<FrameTask>) {
    bounded(QUEUE_CAPACITY)
}

fn run(mut encoder: VideoEncoder, receiver: Receiver<FrameTask>) -> Result<(), String> {
    let mut first_error: Option<String> = None;
    while let Ok(task) = receiver.recv() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            if let Some(error) = &first_error {
                Err(error.clone())
            } else {
                let frame = unsafe { &*task.frame.cast::<Frame<'_>>() };
                encoder
                    .send_frame(frame)
                    .map_err(|error| format!("encoder rejected frame: {error}"))
            }
        }))
        .unwrap_or_else(|_| Err("encoder panicked while accepting a frame".to_string()));

        if let Err(error) = &result {
            first_error.get_or_insert_with(|| error.clone());
        }
        let _ = task.complete.send(result);
    }

    let finish_result = encoder
        .finish()
        .map_err(|error| format!("failed to finalize encoder: {error}"));
    match (first_error, finish_result) {
        (Some(error), _) => Err(error),
        (None, Err(error)) => Err(error),
        (None, Ok(())) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameTask, QUEUE_CAPACITY, channel};
    use crossbeam_channel::{TrySendError, bounded};

    fn task() -> FrameTask {
        let (complete, _) = bounded(1);
        FrameTask {
            frame: std::ptr::null(),
            complete,
        }
    }

    #[test]
    fn queue_rejects_overflow() {
        let (sender, receiver) = channel();
        for _ in 0..QUEUE_CAPACITY {
            sender.try_send(task()).unwrap();
        }

        assert!(matches!(
            sender.try_send(task()),
            Err(TrySendError::Full(_))
        ));
        drop(receiver);
    }
}
