use std::{cell::RefCell, path::PathBuf, rc::Rc};

use anyhow::{Result, anyhow};
use crossbeam_channel::Receiver;
use gpui::*;
use gpui_component::{ActiveTheme as _, Root, button::*, h_flex, v_flex};
use gpui_wry::WebView;

use super::{
    cursor::CursorOverlay,
    media::{PlaybackEvent, build_webview},
};

pub(super) fn open(
    cx: &mut AsyncApp,
    video_path: PathBuf,
    telemetry_path: PathBuf,
    metadata_path: PathBuf,
) -> Result<WindowHandle<Root>> {
    let player_background = cx.update(|app| app.theme().popover);
    let options = cx.update(|app| WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(960.), px(640.)), app)),
        titlebar: Some(TitlebarOptions {
            title: Some("Recording".into()),
            ..Default::default()
        }),
        window_min_size: Some(size(px(560.), px(360.))),
        ..Default::default()
    });
    let build_error = Rc::new(RefCell::new(None));
    let build_error_for_window = build_error.clone();

    let handle = cx.open_window(options, move |window, cx| {
        let view = match PlaybackView::new(
            video_path,
            telemetry_path,
            metadata_path,
            player_background,
            window,
            cx,
        ) {
            Ok(view) => {
                let view = cx.new(|_| view);
                view.update(cx, |view, cx| view.start_event_listener(cx));
                view
            }
            Err(error) => {
                let message = error.to_string();
                *build_error_for_window.borrow_mut() = Some(message.clone());
                cx.new(|_| PlaybackView::unavailable(message))
            }
        };
        cx.new(|cx| Root::new(view, window, cx))
    })?;

    if let Some(error) = build_error.borrow_mut().take() {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
        return Err(anyhow!("could not create playback view: {error}"));
    }

    Ok(handle)
}

struct PlaybackView {
    webview: Option<Entity<WebView>>,
    time_events: Option<Receiver<PlaybackEvent>>,
    cursor_overlay: CursorOverlay,
    video_path: PathBuf,
    playing: bool,
    error: Option<SharedString>,
}

impl PlaybackView {
    fn new(
        video_path: PathBuf,
        telemetry_path: PathBuf,
        metadata_path: PathBuf,
        player_background: Hsla,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Self> {
        let cursor_overlay = CursorOverlay::load(&telemetry_path, &metadata_path);
        let (webview, time_events) = build_webview(
            &video_path,
            player_background,
            cursor_overlay.asset(),
            window,
            cx,
        )?;
        Ok(Self {
            webview: Some(webview),
            time_events: Some(time_events),
            cursor_overlay,
            video_path,
            playing: false,
            error: None,
        })
    }

    fn unavailable(error: String) -> Self {
        Self {
            webview: None,
            time_events: None,
            cursor_overlay: CursorOverlay::disabled("Cursor overlay unavailable"),
            video_path: PathBuf::new(),
            playing: false,
            error: Some(error.into()),
        }
    }

    fn start_event_listener(&mut self, cx: &mut Context<Self>) {
        let Some(events) = self.time_events.take() else {
            return;
        };

        cx.spawn(async move |view, cx| {
            loop {
                let events_for_wait = events.clone();
                let event = cx
                    .background_spawn(async move { events_for_wait.recv().ok() })
                    .await;
                let Some(event) = event else {
                    break;
                };
                if view
                    .update(cx, |view, cx| view.apply_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_event(&mut self, event: PlaybackEvent, cx: &mut Context<Self>) {
        match event {
            PlaybackEvent::Time { seconds, playing } => {
                let state_changed = self.playing != playing;
                self.playing = playing;
                self.update_cursor(seconds, cx);
                if state_changed {
                    cx.notify();
                }
            }
            PlaybackEvent::State(playing) => {
                if self.playing != playing {
                    self.playing = playing;
                    cx.notify();
                }
            }
        }
    }

    fn update_cursor(&mut self, seconds: f64, cx: &mut Context<Self>) {
        let Some(script) = self.cursor_overlay.script_at(seconds) else {
            return;
        };
        let Some(webview) = self.webview.as_ref() else {
            return;
        };
        let result = webview.update(cx, |webview, _| {
            webview
                .raw()
                .evaluate_script(&script)
                .map_err(|error| error.to_string())
        });
        if let Err(error) = result {
            self.report_error(format!("Cursor overlay failed: {error}"), cx);
        }
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        let Some(webview) = self.webview.as_ref() else {
            return;
        };

        let script = if self.playing {
            "document.getElementById('video').pause();"
        } else {
            "document.getElementById('video').play();"
        };
        match webview.update(cx, |webview, _| {
            webview
                .raw()
                .evaluate_script(script)
                .map_err(|error| error.to_string())
        }) {
            Ok(()) => {
                self.playing = !self.playing;
                self.error = None;
                cx.notify();
            }
            Err(error) => self.report_error(error.to_string(), cx),
        }
    }

    fn report_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error.into());
        cx.notify();
    }
}

impl Render for PlaybackView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let video_area = if let Some(webview) = self.webview.clone() {
            div()
                .flex_1()
                .min_h(px(0.))
                .bg(cx.theme().popover)
                .child(webview)
        } else {
            div()
                .flex_1()
                .min_h(px(0.))
                .bg(cx.theme().popover)
                .flex()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child("Recording could not be loaded")
        };

        let file_name = self
            .video_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Recording");
        let caption = self.error.clone().unwrap_or_else(|| {
            format!("Loaded {file_name} · {}", self.cursor_overlay.status()).into()
        });
        let button_label = if self.playing { "Pause" } else { "Play" };

        v_flex()
            .size_full()
            .gap_3()
            .p_4()
            .bg(cx.theme().background)
            .child(video_area)
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("play-pause")
                            .primary()
                            .label(button_label)
                            .on_click(cx.listener(|view, _, _, cx| view.toggle(cx))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_color(cx.theme().muted_foreground)
                            .child(caption),
                    ),
            )
    }
}
