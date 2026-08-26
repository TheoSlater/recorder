use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

use anyhow::{Result, anyhow};
use gpui::*;
use gpui_component::{IndexPath, Root, select::SelectState, slider::SliderState};

#[path = "playback/editor_canvas.rs"]
mod editor_canvas;
#[path = "playback/editor_canvas_controls.rs"]
mod editor_canvas_controls;
#[path = "playback/editor_canvas_cursor.rs"]
mod editor_canvas_cursor;
#[path = "playback/editor_canvas_cursor_blur.rs"]
mod editor_canvas_cursor_blur;
#[path = "playback/editor_canvas_geometry.rs"]
mod editor_canvas_geometry;
#[path = "playback/editor_canvas_paint.rs"]
mod editor_canvas_paint;
#[path = "playback/editor_cursor.rs"]
mod editor_cursor;
#[path = "playback/editor_inspector.rs"]
mod editor_inspector;
#[path = "playback/editor_motion_blur.rs"]
mod editor_motion_blur;
#[path = "playback/editor_motion_state.rs"]
mod editor_motion_state;
#[path = "playback/editor_preview.rs"]
mod editor_preview;
#[path = "playback/editor_shell.rs"]
mod editor_shell;
#[path = "playback/editor_timeline.rs"]
mod editor_timeline;
#[path = "playback/editor_timeline_canvas.rs"]
mod editor_timeline_canvas;
#[path = "playback/editor_toolbar.rs"]
mod editor_toolbar;
#[path = "playback/editor_zoom.rs"]
mod editor_zoom;
#[path = "playback_ui.rs"]
mod playback_ui;
#[path = "playback/preview_rate.rs"]
mod preview_rate;
mod view;

pub(super) use view::PlaybackView;

type CursorControls = (
    Entity<SliderState>,
    Entity<SliderState>,
    Entity<SelectState<Vec<&'static str>>>,
);

use super::{
    cursor_settings::{
        CURSOR_STYLE_LABELS, CursorSettings, MAX_CURSOR_SCALE, MIN_CURSOR_SCALE, cursor_assets,
    },
    motion_blur::MotionBlurSettings,
    project_settings::ProjectSettings,
};

pub(super) fn open(
    cx: &mut AsyncApp,
    video_path: PathBuf,
    telemetry_path: PathBuf,
    metadata_path: PathBuf,
    project_path: PathBuf,
    project_settings: ProjectSettings,
    generate_auto_zooms: bool,
    autoplay: bool,
) -> Result<WindowHandle<Root>> {
    let options = cx.update(|app| {
        let project_name = video_path
            .parent()
            .and_then(|directory| directory.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("Recording");
        WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(720.)), app)),
            titlebar: Some(TitlebarOptions {
                title: Some(project_name.to_string().into()),
                ..Default::default()
            }),
            window_min_size: Some(size(px(720.), px(460.))),
            ..Default::default()
        }
    });
    let build_error = Rc::new(RefCell::new(None));
    let build_error_for_window = build_error.clone();

    let handle = cx.open_window(options, move |window, cx| {
        let telemetry_path_for_load = telemetry_path.clone();
        let metadata_path_for_load = metadata_path.clone();
        let view = match PlaybackView::new(
            video_path,
            project_path.clone(),
            project_settings.clone(),
            generate_auto_zooms,
            window,
            cx,
        ) {
            Ok(view) => cx.new(|_| view),
            Err(error) => {
                let message = error.to_string();
                tracing::error!(
                    target: "recorder::playback",
                    error = %message,
                    "could not create playback view"
                );
                *build_error_for_window.borrow_mut() = Some(message.clone());
                let view =
                    PlaybackView::unavailable(message, project_path, project_settings, window, cx);
                cx.new(|_| view)
            }
        };
        view.update(cx, |view, cx| {
            view.subscribe_cursor_controls(cx);
            view.subscribe_canvas_controls(cx);
            view.start_event_listener(cx);
            if view.player.is_some() {
                view.start_background_tasks(telemetry_path_for_load, metadata_path_for_load, cx);
                if autoplay {
                    view.toggle(cx);
                }
            }
        });
        cx.new(|cx| Root::new(view, window, cx))
    })?;

    if let Some(error) = build_error.borrow_mut().take() {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
        tracing::error!(
            target: "recorder::playback",
            error = %error,
            "playback window initialization failed"
        );
        return Err(anyhow!("could not create playback view: {error}"));
    }

    Ok(handle)
}

fn cursor_controls(settings: &CursorSettings, window: &mut Window, cx: &mut App) -> CursorControls {
    let cursor_size_slider = cx.new(|_| {
        SliderState::new()
            .min(MIN_CURSOR_SCALE)
            .max(MAX_CURSOR_SCALE)
            .step(0.1)
            .default_value(settings.scale)
    });
    let cursor_smoothing_slider = cx.new(|_| {
        SliderState::new()
            .min(0.0)
            .max(1.0)
            .step(0.05)
            .default_value(settings.smoothing)
    });
    let cursor_style_select = cx.new(|cx| {
        SelectState::new(
            CURSOR_STYLE_LABELS.to_vec(),
            Some(IndexPath::default().row(settings.style.index())),
            window,
            cx,
        )
    });
    (
        cursor_size_slider,
        cursor_smoothing_slider,
        cursor_style_select,
    )
}

/// The single authored motion-blur amount, shown as a percentage.
fn motion_blur_control(settings: MotionBlurSettings, cx: &mut App) -> Entity<SliderState> {
    cx.new(|_| {
        SliderState::new()
            .min(0.0)
            .max(1.0)
            .step(0.05)
            .default_value(settings.amount)
    })
}

fn load_cursor_images(cx: &App) -> Result<[Arc<RenderImage>; 2]> {
    let renderer = cx.svg_renderer();
    let assets = cursor_assets();
    let first = renderer
        .render_single_frame(assets[0].svg().as_bytes(), 1.0)
        .map_err(|error| anyhow!("could not render default cursor: {error}"))?;
    let second = renderer
        .render_single_frame(assets[1].svg().as_bytes(), 1.0)
        .map_err(|error| anyhow!("could not render circle cursor: {error}"))?;
    Ok([first, second])
}
