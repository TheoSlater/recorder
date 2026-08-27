use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{AppContext, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, Theme};

use crate::recorder::{RecorderView, ShutdownCoordinator, enumerate_monitors, enumerate_windows};

pub(crate) fn run() {
    let _ = tracing_subscriber::fmt().with_target(true).try_init();
    let monitors = enumerate_monitors();
    let windows = enumerate_windows();
    let debug_video = std::env::var_os("RECORDER_DEBUG_OPEN_VIDEO").map(std::path::PathBuf::from);
    let shutdown = Arc::new(ShutdownCoordinator::new());
    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);
        Theme::sync_system_appearance(None, cx);

        let shutdown_for_quit = shutdown.clone();
        cx.on_app_quit(move |cx| {
            let wait = shutdown_for_quit.stop_and_wait().map(|done| {
                cx.background_executor().spawn(async move {
                    let _ = done.recv();
                })
            });

            async move {
                if let Some(task) = wait {
                    task.await;
                }
            }
        })
        .detach();

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(760.), px(600.)), cx)),
            window_min_size: Some(size(px(520.), px(440.))),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Screen Recorder".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let allow_close = Arc::new(AtomicBool::new(false));
                let close_started = Arc::new(AtomicBool::new(false));
                let allow_close_for_handler = allow_close.clone();
                let close_started_for_handler = close_started.clone();
                let shutdown_for_close = shutdown.clone();
                window.on_window_should_close(cx, move |_, cx| {
                    if allow_close_for_handler.load(Ordering::Acquire) {
                        return true;
                    }
                    if close_started_for_handler.swap(true, Ordering::AcqRel) {
                        return false;
                    }

                    let Some(done) = shutdown_for_close.stop_and_wait() else {
                        close_started_for_handler.store(false, Ordering::Release);
                        return true;
                    };
                    let allow_close = allow_close_for_handler.clone();
                    cx.spawn(async move |cx| {
                        let wait = cx.background_spawn(async move {
                            let _ = done.recv();
                        });
                        wait.await;
                        allow_close.store(true, Ordering::Release);
                        cx.update(|cx| cx.quit());
                    })
                    .detach();
                    false
                });

                let view = cx.new(|cx| {
                    cx.observe_window_appearance(window, |_, window, cx| {
                        Theme::sync_system_appearance(Some(window), cx);
                        cx.refresh_windows();
                    })
                    .detach();

                    RecorderView::new(monitors, windows, shutdown.clone())
                });
                if std::env::var_os("RECORDER_DEBUG_OPEN_LATEST").is_some() {
                    view.update(cx, |view, cx| view.open_latest_project(cx));
                }
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open recorder window");

            if let Some(video_path) = debug_video
                && let Err(error) = crate::recorder::open_debug_video(cx, video_path)
            {
                tracing::error!(
                    target: "recorder::playback",
                    error = %error,
                    "could not open debug playback video"
                );
            }
        })
        .detach();
    });
}
