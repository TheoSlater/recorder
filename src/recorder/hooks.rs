use crossbeam_channel::Receiver;
use gpui::{AppContext, Context};

use super::model::WorkerEvent;
use super::ui::RecorderView;

pub(super) fn watch_worker(events: Receiver<WorkerEvent>, cx: &mut Context<RecorderView>) {
    cx.spawn(async move |view, cx| {
        loop {
            let events = events.clone();
            let event = cx.background_spawn(async move { events.recv().ok() }).await;
            let Some(event) = event else {
                view.update(cx, |view, cx| {
                    view.apply_worker_event(
                        WorkerEvent::Finished(Err(
                            "capture worker event channel closed".to_string()
                        )),
                        cx,
                    );
                })
                .ok();
                break;
            };

            let finished = matches!(&event, WorkerEvent::Finished(_));
            if view
                .update(cx, |view, cx| view.apply_worker_event(event, cx))
                .is_err()
            {
                break;
            }

            if finished {
                break;
            }
        }
    })
    .detach();
}
