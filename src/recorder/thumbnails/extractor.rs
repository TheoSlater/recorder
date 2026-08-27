use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};

use crossbeam_channel::{RecvTimeoutError, SendTimeoutError};

use super::{
    ExtractionRequest, ThumbnailEvent, WorkerCommand, WorkerContext,
    decoder::{Decoder, ExtractedFrame},
};

pub(super) fn run(context: WorkerContext, path: PathBuf) {
    let com = unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        )
    };
    if com.is_err() {
        send_unavailable(&context, format!("could not initialize COM: {com:?}"));
        return;
    }
    let _com_guard = ComGuard;

    if let Err(error) = unsafe {
        windows::Win32::Media::MediaFoundation::MFStartup(
            windows::Win32::Media::MediaFoundation::MF_VERSION,
            windows::Win32::Media::MediaFoundation::MFSTARTUP_FULL,
        )
    } {
        send_unavailable(
            &context,
            format!("could not initialize Media Foundation: {error}"),
        );
        return;
    }
    let _media_foundation_guard = MediaFoundationGuard;

    let mut decoder = match Decoder::open(&path) {
        Ok(decoder) => decoder,
        Err(error) => {
            send_unavailable(&context, error.to_string());
            return;
        }
    };
    tracing::info!(
        target: "recorder::thumbnails",
        path = %path.display(),
        width = decoder.width,
        height = decoder.height,
        duration_seconds = decoder.duration,
        "thumbnail source opened"
    );

    loop {
        if context.stop.load(Ordering::Acquire) {
            return;
        }
        let command = match context.commands.recv_timeout(Duration::from_millis(50)) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        match command {
            WorkerCommand::Shutdown => return,
            WorkerCommand::Request(request) => {
                if !is_current(&context, request.generation) {
                    send_event(
                        &context,
                        ThumbnailEvent::Stale {
                            key: request.key,
                            generation: request.generation,
                        },
                    );
                    continue;
                }
                process(&context, &mut decoder, request);
            }
        }
    }
}

fn process(context: &WorkerContext, decoder: &mut Decoder, request: ExtractionRequest) {
    let key = request.key.clone();
    let generation = request.generation;
    let result = decoder.extract(request.timestamp_us, request.size, &|| {
        is_current(context, generation)
    });
    match result {
        Ok(Some(ExtractedFrame {
            image,
            decode_time,
            resize_time,
        })) => send_event(
            context,
            ThumbnailEvent::Complete {
                key,
                generation,
                image,
                size: request.size,
                decode_time,
                resize_time,
            },
        ),
        Ok(None) if !is_current(context, generation) => {
            send_event(context, ThumbnailEvent::Stale { key, generation })
        }
        Ok(None) => send_event(
            context,
            ThumbnailEvent::Failed {
                key,
                generation,
                error: "no video frame was available at the requested timestamp".to_string(),
            },
        ),
        Err(error) => send_event(
            context,
            ThumbnailEvent::Failed {
                key,
                generation,
                error: error.to_string(),
            },
        ),
    }
}

fn is_current(context: &WorkerContext, generation: u64) -> bool {
    !context.stop.load(Ordering::Acquire)
        && context.latest_generation.load(Ordering::Acquire) == generation
}

fn send_unavailable(context: &WorkerContext, error: String) {
    send_event(context, ThumbnailEvent::Unavailable(error));
}

fn send_event(context: &WorkerContext, event: ThumbnailEvent) {
    let mut event = event;
    loop {
        if context.stop.load(Ordering::Acquire) {
            return;
        }
        match context
            .events
            .send_timeout(event, Duration::from_millis(50))
        {
            Ok(()) => return,
            Err(SendTimeoutError::Timeout(next)) => event = next,
            Err(SendTimeoutError::Disconnected(_)) => return,
        }
    }
}

struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoUninitialize() };
    }
}

struct MediaFoundationGuard;

impl Drop for MediaFoundationGuard {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Media::MediaFoundation::MFShutdown() };
    }
}
