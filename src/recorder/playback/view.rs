use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use crossbeam_channel::Receiver;
use gpui::*;
use gpui_component::{
    ActiveTheme,
    color_picker::ColorPickerEvent,
    select::{SelectEvent, SelectState},
    slider::{SliderEvent, SliderState},
};

use super::super::export::{self, ExportEvent, ExportRequest};
use super::super::{
    alerts::{AlertQueue, AppAlert},
    cursor::CursorOverlay,
    cursor_settings::{CursorSettings, CursorStyle, MAX_CURSOR_SCALE, MIN_CURSOR_SCALE},
    media::{FrameTiming, NativePlayer, PlaybackEvent, PlaybackMetrics, build_player},
    project_save::ProjectSaveQueue,
    project_settings::{
        CanvasBackgroundKind, CanvasComposition, CanvasView, MAX_CANVAS_ZOOM, MIN_CANVAS_ZOOM,
        ProjectSettings,
    },
    rendering::{CanvasPlacement, CompositionState, PhysicalSize},
    thumbnails::ThumbnailManager,
    zoom::{
        CursorSizeRegion, MAX_ZOOM_REGION_SCALE, MIN_CURSOR_SIZE_REGION_DURATION_US,
        MIN_ZOOM_REGION_DURATION_US, ZoomRegion, ZoomTarget, cursor_scale_at,
    },
};
use super::{
    editor_canvas, editor_canvas_controls, editor_canvas_geometry,
    editor_motion_state::{MotionBlurState, PresentedFrame},
    editor_shell, editor_timeline,
    preview_rate::PreviewRate,
};

// One 60 FPS frame is enough seek precision for an interactive drag while
// preventing pointer noise from starting more decoder work than the preview
// can display.
const SCRUB_SEEK_STEP_US: u64 = 16_667;

#[derive(Clone, Copy)]
enum CanvasInteraction {
    Move {
        last: Point<Pixels>,
    },
    Resize {
        center: Point<Pixels>,
        start_scale: f64,
        start_distance: f32,
    },
}

#[derive(Clone, Copy)]
enum CanvasColor {
    Solid,
    GradientStart,
    GradientEnd,
}

pub(crate) struct PlaybackView {
    pub(super) player: Option<NativePlayer>,
    time_events: Option<Receiver<PlaybackEvent>>,
    pub(super) video_path: PathBuf,
    pub(super) telemetry_path: PathBuf,
    pub(super) metadata_path: PathBuf,
    pub(super) project_settings: ProjectSettings,
    generate_auto_zooms_on_open: bool,
    auto_zooms_generated: bool,
    pub(super) cursor_size_slider: Entity<SliderState>,
    pub(super) cursor_smoothing_slider: Entity<SliderState>,
    pub(super) cursor_style_select: Entity<SelectState<Vec<&'static str>>>,
    pub(super) motion_blur_slider: Entity<SliderState>,
    pub(super) canvas_controls: editor_canvas_controls::CanvasControls,
    subscriptions: Vec<Subscription>,
    pub(super) cursor_overlay: CursorOverlay,
    cursor_overlay_loading: bool,
    pub(super) cursor_frame: Option<super::super::cursor::CursorFrame>,
    pub(super) cursor_images: [Arc<RenderImage>; 2],
    pub(super) motion_blur: MotionBlurState,
    pub(super) native_preview: super::native_preview::NativePreview,
    pub(super) image: Option<Arc<RenderImage>>,
    pub(super) video_width: u32,
    pub(super) video_height: u32,
    pub(super) canvas_bounds: editor_canvas::CanvasBounds,
    pub(super) background_image: Option<Arc<RenderImage>>,
    pub(super) selected_recording: bool,
    canvas_hover_hit: Option<editor_canvas_geometry::CanvasHit>,
    canvas_interaction: Option<CanvasInteraction>,
    canvas_interaction_changed: bool,
    background_load_id: u64,
    panning: bool,
    last_pan_position: Option<Point<Pixels>>,
    pub(super) playing: bool,
    preview_rate: PreviewRate,
    last_preview_slot: Option<u64>,
    pub(super) timeline: editor_timeline::TimelineState,
    pub(super) timeline_bounds: editor_timeline::TimelineBounds,
    pub(super) thumbnail_manager: ThumbnailManager,
    thumbnail_task: Option<Task<()>>,
    pub(super) selected_zoom_region: Option<usize>,
    pub(super) selected_cursor_size_region: Option<usize>,
    pub(super) hovered_zoom_hit: Option<editor_timeline::ZoomHit>,
    pub(super) hovered_cursor_size_hit: Option<editor_timeline::CursorSizeHit>,
    pub(super) timeline_focus_handle: FocusHandle,
    timeline_interaction: Option<editor_timeline::TimelineInteraction>,
    zoom_interaction_changed: bool,
    cursor_size_interaction_changed: bool,
    pending_seek_target: Option<f64>,
    pub(super) error: Option<SharedString>,
    pub(super) pending_alerts: AlertQueue,
    save_queue: ProjectSaveQueue,
    save_error_task: Option<Task<()>>,
    pub(super) metrics: PlaybackMetrics,
    pub(super) frame_timing: Option<FrameTiming>,
    pub(super) frame_invalidated_at: Option<Instant>,
    latest_seek_generation: u64,
    last_published_seek_us: Option<u64>,
    pending_image_releases: Vec<Arc<RenderImage>>,
    pub(super) export_state: Option<ExportState>,
    export_task: Option<Task<()>>,
}

pub(super) struct ExportState {
    pub(super) cancel: Arc<std::sync::atomic::AtomicBool>,
    pub(super) completed: u64,
    pub(super) total: u64,
}

impl Drop for PlaybackView {
    fn drop(&mut self) {
        if let Some(state) = &self.export_state {
            state
                .cancel
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }
}

impl PlaybackView {
    pub(super) fn new(
        video_path: PathBuf,
        project_path: PathBuf,
        project_settings: ProjectSettings,
        generate_auto_zooms_on_open: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Self> {
        let video_path_for_state = video_path.clone();
        let project_settings = project_settings.normalized();
        let (cursor_size_slider, cursor_smoothing_slider, cursor_style_select) =
            super::cursor_controls(&project_settings.cursor, window, cx);
        let motion_blur_slider = super::motion_blur_control(project_settings.motion_blur, cx);
        let pending_alerts = AlertQueue::default();
        let canvas_controls = editor_canvas_controls::CanvasControls::new(
            &project_settings.canvas_composition,
            window,
            cx,
        );
        let cursor_images = super::load_cursor_images(cx)?;
        let (player, time_events) = build_player(&video_path)?;
        let metrics = player.metrics();
        let thumbnail_manager = ThumbnailManager::new(video_path.clone()).unwrap_or_else(|error| {
            tracing::warn!(
                target: "recorder::thumbnails",
                error = %error,
                "could not start thumbnail worker"
            );
            ThumbnailManager::disabled(video_path.clone())
        });
        let save_queue = ProjectSaveQueue::new(project_path.clone());
        Ok(Self {
            player: Some(player),
            time_events: Some(time_events),
            video_path: video_path_for_state,
            telemetry_path: PathBuf::new(),
            metadata_path: PathBuf::new(),
            project_settings,
            generate_auto_zooms_on_open,
            auto_zooms_generated: false,
            cursor_size_slider,
            cursor_smoothing_slider,
            cursor_style_select,
            motion_blur_slider,
            canvas_controls,
            subscriptions: Vec::new(),
            cursor_overlay: CursorOverlay::loading(),
            cursor_overlay_loading: true,
            cursor_frame: None,
            cursor_images,
            motion_blur: MotionBlurState::default(),
            native_preview: super::native_preview::NativePreview::default(),
            image: None,
            video_width: 0,
            video_height: 0,
            canvas_bounds: Rc::new(RefCell::new(None)),
            background_image: None,
            selected_recording: false,
            canvas_hover_hit: None,
            canvas_interaction: None,
            canvas_interaction_changed: false,
            background_load_id: 0,
            panning: false,
            last_pan_position: None,
            playing: false,
            preview_rate: PreviewRate::default(),
            last_preview_slot: None,
            timeline: editor_timeline::TimelineState::default(),
            timeline_bounds: Rc::new(RefCell::new(None)),
            thumbnail_manager,
            thumbnail_task: None,
            selected_zoom_region: None,
            selected_cursor_size_region: None,
            hovered_zoom_hit: None,
            hovered_cursor_size_hit: None,
            timeline_focus_handle: cx.focus_handle(),
            timeline_interaction: None,
            zoom_interaction_changed: false,
            cursor_size_interaction_changed: false,
            pending_seek_target: None,
            error: None,
            pending_alerts,
            save_queue,
            save_error_task: None,
            metrics,
            frame_timing: None,
            frame_invalidated_at: None,
            latest_seek_generation: 0,
            last_published_seek_us: None,
            pending_image_releases: Vec::new(),
            export_state: None,
            export_task: None,
        })
    }

    pub(super) fn unavailable(
        error: String,
        project_path: PathBuf,
        project_settings: ProjectSettings,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let project_settings = project_settings.normalized();
        let error: SharedString = error.into();
        let (cursor_size_slider, cursor_smoothing_slider, cursor_style_select) =
            super::cursor_controls(&project_settings.cursor, window, cx);
        let motion_blur_slider = super::motion_blur_control(project_settings.motion_blur, cx);
        let canvas_controls = editor_canvas_controls::CanvasControls::new(
            &project_settings.canvas_composition,
            window,
            cx,
        );
        let cursor_images =
            super::load_cursor_images(cx).expect("built-in cursor SVG should render");
        let save_queue = ProjectSaveQueue::new(project_path.clone());
        Self {
            player: None,
            time_events: None,
            video_path: PathBuf::new(),
            telemetry_path: PathBuf::new(),
            metadata_path: PathBuf::new(),
            project_settings,
            generate_auto_zooms_on_open: false,
            auto_zooms_generated: false,
            cursor_size_slider,
            cursor_smoothing_slider,
            cursor_style_select,
            motion_blur_slider,
            canvas_controls,
            subscriptions: Vec::new(),
            cursor_overlay: CursorOverlay::disabled("Cursor overlay unavailable"),
            cursor_overlay_loading: false,
            cursor_frame: None,
            cursor_images,
            motion_blur: MotionBlurState::default(),
            native_preview: super::native_preview::NativePreview::default(),
            image: None,
            video_width: 0,
            video_height: 0,
            canvas_bounds: Rc::new(RefCell::new(None)),
            background_image: None,
            selected_recording: false,
            canvas_hover_hit: None,
            canvas_interaction: None,
            canvas_interaction_changed: false,
            background_load_id: 0,
            panning: false,
            last_pan_position: None,
            playing: false,
            preview_rate: PreviewRate::default(),
            last_preview_slot: None,
            timeline: editor_timeline::TimelineState::default(),
            timeline_bounds: Rc::new(RefCell::new(None)),
            thumbnail_manager: ThumbnailManager::disabled(PathBuf::new()),
            thumbnail_task: None,
            selected_zoom_region: None,
            selected_cursor_size_region: None,
            hovered_zoom_hit: None,
            hovered_cursor_size_hit: None,
            timeline_focus_handle: cx.focus_handle(),
            timeline_interaction: None,
            zoom_interaction_changed: false,
            cursor_size_interaction_changed: false,
            pending_seek_target: None,
            error: Some(error.clone()),
            pending_alerts: {
                let mut pending_alerts = AlertQueue::default();
                pending_alerts.push(AppAlert::error(error));
                pending_alerts
            },
            save_queue,
            save_error_task: None,
            metrics: PlaybackMetrics::default(),
            frame_timing: None,
            frame_invalidated_at: None,
            latest_seek_generation: 0,
            last_published_seek_us: None,
            pending_image_releases: Vec::new(),
            export_state: None,
            export_task: None,
        }
    }

    pub(super) fn start_background_tasks(
        &mut self,
        telemetry_path: PathBuf,
        metadata_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.telemetry_path = telemetry_path.clone();
        self.metadata_path = metadata_path.clone();
        self.start_thumbnail_listener(cx);
        if self.project_settings.canvas_composition.background.kind == CanvasBackgroundKind::Image {
            self.start_background_image_load(
                self.project_settings
                    .canvas_composition
                    .background
                    .image_path
                    .clone(),
                cx,
            );
        }

        let view = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let overlay = cx
                .background_spawn(
                    async move { CursorOverlay::load(&telemetry_path, &metadata_path) },
                )
                .await;
            view.update(cx, |view, cx| view.finish_cursor_load(overlay, cx))
                .ok();
        })
        .detach();

        let view = cx.entity().downgrade();
        let save_queue = self.save_queue.clone();
        self.save_error_task = Some(cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let Some(error) = save_queue.take_error() else {
                    continue;
                };
                if view
                    .update(cx, |view, cx| view.report_error(error, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn finish_cursor_load(&mut self, overlay: CursorOverlay, cx: &mut Context<Self>) {
        self.cursor_overlay = overlay;
        self.cursor_overlay_loading = false;
        if let Some(warning) = self.cursor_overlay.warning() {
            tracing::warn!(
                target: "recorder::playback",
                warning,
                "cursor telemetry warning"
            );
            self.pending_alerts
                .push(AppAlert::warning(warning.to_string()));
        } else if !self.cursor_overlay.has_telemetry() {
            tracing::error!(
                target: "recorder::playback",
                error = self.cursor_overlay.status(),
                "cursor overlay unavailable"
            );
            self.pending_alerts
                .push(AppAlert::error(self.cursor_overlay.status().to_string()));
        }
        self.maybe_generate_auto_zooms(cx);
        cx.notify();
    }

    pub(super) fn start_event_listener(&mut self, cx: &mut Context<Self>) {
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
                let mut events_to_apply = Vec::with_capacity(4);
                events_to_apply.push(event);
                events_to_apply.extend(events.try_iter());
                if view
                    .update(cx, |view, cx| view.apply_events(events_to_apply, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn start_thumbnail_listener(&mut self, cx: &mut Context<Self>) {
        if !self.thumbnail_manager.is_running() {
            return;
        }
        let events = self.thumbnail_manager.events();
        self.thumbnail_task = Some(cx.spawn(async move |view, cx| {
            loop {
                let events_for_wait = events.clone();
                let Some(event) = cx
                    .background_spawn(async move { events_for_wait.recv().ok() })
                    .await
                else {
                    break;
                };
                let mut batch = Vec::with_capacity(4);
                batch.push(event);
                batch.extend(events.try_iter());
                if view
                    .update(cx, |view, cx| {
                        if view.thumbnail_manager.apply_events(batch) {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    pub(super) fn export_or_cancel(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = &self.export_state {
            state
                .cancel
                .store(true, std::sync::atomic::Ordering::Release);
            return;
        }
        if self.player.is_none() {
            return;
        }

        let suggested = self
            .video_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!(
                "{}-export.mp4",
                self.video_path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("recording")
            ));
        let output = match export::choose_output_path(&suggested.to_string_lossy()) {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                self.report_error(error.to_string(), cx);
                return;
            }
        };
        let request = ExportRequest {
            video_path: self.video_path.clone(),
            telemetry_path: self.telemetry_path.clone(),
            metadata_path: self.metadata_path.clone(),
            settings: self.project_settings.clone(),
        };
        let handle = match export::start(request, output) {
            Ok(handle) => handle,
            Err(error) => {
                self.report_error(error.to_string(), cx);
                return;
            }
        };
        let events = handle.events.clone();
        self.export_state = Some(ExportState {
            cancel: handle.cancel,
            completed: 0,
            total: 0,
        });
        let view = cx.entity().downgrade();
        self.export_task = Some(cx.spawn(async move |_, cx| {
            loop {
                let events_for_wait = events.clone();
                let event = cx
                    .background_spawn(async move { events_for_wait.recv().ok() })
                    .await;
                let Some(event) = event else {
                    break;
                };
                let mut latest = event;
                for event in events.try_iter() {
                    latest = event;
                }
                let terminal = matches!(
                    latest,
                    ExportEvent::Finished(_) | ExportEvent::Cancelled | ExportEvent::Error(_)
                );
                if view
                    .update(cx, |view, cx| view.apply_export_event(latest, cx))
                    .is_err()
                {
                    break;
                }
                if terminal {
                    break;
                }
            }
        }));
        cx.notify();
    }

    pub(super) fn export_available(&self) -> bool {
        self.player.is_some() && self.export_state.is_none()
    }

    pub(super) fn export_label(&self) -> String {
        let Some(state) = &self.export_state else {
            return "Export".to_string();
        };
        if state.total == 0 {
            "Exporting…".to_string()
        } else {
            format!(
                "Exporting {:.0}%",
                state.completed as f64 / state.total as f64 * 100.0
            )
        }
    }

    pub(super) fn exporting(&self) -> bool {
        self.export_state.is_some()
    }

    fn apply_export_event(&mut self, event: ExportEvent, cx: &mut Context<Self>) {
        match event {
            ExportEvent::Progress { completed, total } => {
                if let Some(state) = &mut self.export_state {
                    state.completed = completed;
                    state.total = total;
                }
            }
            ExportEvent::Finished(path) => {
                tracing::info!(target: "recorder::export", path = %path.display(), "export completed");
                self.export_state = None;
            }
            ExportEvent::Cancelled => {
                self.export_state = None;
                self.report_warning("Export cancelled.", cx);
            }
            ExportEvent::Error(error) => {
                self.export_state = None;
                self.report_error(format!("Export failed: {error}"), cx);
            }
        }
        cx.notify();
    }

    pub(super) fn subscribe_cursor_controls(&mut self, cx: &mut Context<Self>) {
        let size_slider = self.cursor_size_slider.clone();
        self.subscriptions.push(
            cx.subscribe(&size_slider, |view, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(value) = event else {
                    return;
                };
                let mut settings = view.project_settings.cursor;
                settings.scale = value.start();
                view.set_cursor_settings(settings, cx);
            }),
        );

        let smoothing_slider = self.cursor_smoothing_slider.clone();
        self.subscriptions.push(cx.subscribe(
            &smoothing_slider,
            |view, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(value) = event else {
                    return;
                };
                let mut settings = view.project_settings.cursor;
                settings.smoothing = value.start();
                view.set_cursor_settings(settings, cx);
            },
        ));

        let motion_blur_slider = self.motion_blur_slider.clone();
        self.subscriptions.push(cx.subscribe(
            &motion_blur_slider,
            |view, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(value) = event else {
                    return;
                };
                view.set_motion_blur_amount(value.start(), cx);
            },
        ));

        let style_select = self.cursor_style_select.clone();
        self.subscriptions.push(cx.subscribe(
            &style_select,
            |view, _, event: &SelectEvent<Vec<&'static str>>, cx| {
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                let mut settings = view.project_settings.cursor;
                settings.style = CursorStyle::from_label(label);
                view.set_cursor_settings(settings, cx);
            },
        ));
    }

    pub(super) fn subscribe_canvas_controls(&mut self, cx: &mut Context<Self>) {
        let aspect_ratio = self.canvas_controls.aspect_ratio.clone();
        self.subscriptions.push(cx.subscribe(
            &aspect_ratio,
            |view, _, event: &SelectEvent<Vec<&'static str>>, cx| {
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                let mut composition = view.project_settings.canvas_composition.clone();
                composition.aspect_ratio =
                    super::super::project_settings::AspectRatioPreset::from_label(label);
                view.set_canvas_composition(composition, cx);
            },
        ));

        let background_kind = self.canvas_controls.background_kind.clone();
        self.subscriptions.push(cx.subscribe(
            &background_kind,
            |view, _, event: &SelectEvent<Vec<&'static str>>, cx| {
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                view.set_canvas_background_kind(CanvasBackgroundKind::from_label(label), cx);
            },
        ));

        let padding = self.canvas_controls.padding.clone();
        self.subscriptions
            .push(cx.subscribe(&padding, |view, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(value) = event else {
                    return;
                };
                let mut composition = view.project_settings.canvas_composition.clone();
                composition.padding = f64::from(value.start());
                view.set_canvas_composition(composition, cx);
            }));

        let scale = self.canvas_controls.scale.clone();
        self.subscriptions
            .push(cx.subscribe(&scale, |view, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(value) = event else {
                    return;
                };
                let mut composition = view.project_settings.canvas_composition.clone();
                composition.scale = f64::from(value.start());
                view.set_canvas_composition(composition, cx);
            }));

        let corner_radius = self.canvas_controls.corner_radius.clone();
        self.subscriptions.push(cx.subscribe(
            &corner_radius,
            |view, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(value) = event else {
                    return;
                };
                let mut composition = view.project_settings.canvas_composition.clone();
                composition.corner_radius = f64::from(value.start());
                view.set_canvas_composition(composition, cx);
            },
        ));

        let solid_color = self.canvas_controls.solid_color.clone();
        self.subscriptions.push(cx.subscribe(
            &solid_color,
            |view, _, event: &ColorPickerEvent, cx| {
                let ColorPickerEvent::Change(Some(color)) = event else {
                    return;
                };
                view.set_canvas_color(CanvasColor::Solid, *color, cx);
            },
        ));

        let gradient_start = self.canvas_controls.gradient_start.clone();
        self.subscriptions.push(cx.subscribe(
            &gradient_start,
            |view, _, event: &ColorPickerEvent, cx| {
                let ColorPickerEvent::Change(Some(color)) = event else {
                    return;
                };
                view.set_canvas_color(CanvasColor::GradientStart, *color, cx);
            },
        ));

        let gradient_end = self.canvas_controls.gradient_end.clone();
        self.subscriptions.push(cx.subscribe(
            &gradient_end,
            |view, _, event: &ColorPickerEvent, cx| {
                let ColorPickerEvent::Change(Some(color)) = event else {
                    return;
                };
                view.set_canvas_color(CanvasColor::GradientEnd, *color, cx);
            },
        ));
    }

    /// Describes the current frame for the preview compositor.
    ///
    /// This is the whole bridge from editor state to the renderer: everything
    /// time-dependent is evaluated by the shared composition module, and the
    /// editor camera is deliberately absent, so navigating the workspace cannot
    /// reach the composited output.
    /// The frame the native compositor should draw, in the canvas rectangle the
    /// editor has laid out.
    ///
    /// The composition itself is evaluated exactly as export evaluates it, so
    /// the preview and the exported file describe the same picture. The editor
    /// camera enters only through `canvas`, and only as layout.
    pub(super) fn composition_state(
        &self,
        target_size: PhysicalSize,
        canvas: CanvasPlacement,
    ) -> Option<CompositionState> {
        let source = super::super::composition::SourceSize {
            width: self.video_width,
            height: self.video_height,
        };
        if !source.valid() {
            return None;
        }
        Some(CompositionState::new(
            target_size,
            canvas,
            source,
            super::super::composition::evaluate(
                &self.project_settings,
                source,
                self.timeline.playhead_us,
                self.cursor_frame,
            ),
            self.project_settings.canvas_composition.background.clone(),
            self.motion_blur.display(),
        ))
    }

    /// Where the composition canvas sits inside the preview surface.
    pub(super) fn canvas_placement(
        &self,
        stage: Bounds<Pixels>,
        surround: [f32; 4],
        scale_factor: f32,
    ) -> Option<CanvasPlacement> {
        let geometry = self.canvas_geometry()?;
        editor_canvas_geometry::canvas_placement(stage, geometry.canvas, surround, scale_factor)
    }

    fn canvas_geometry(&self) -> Option<editor_canvas_geometry::CanvasGeometry> {
        let stage = (*self.canvas_bounds.borrow())?;
        if self.video_width == 0 || self.video_height == 0 {
            return None;
        }
        Some(editor_canvas_geometry::preview_geometry(
            stage,
            self.project_settings.canvas,
            &self.project_settings.canvas_composition,
            self.video_width,
            self.video_height,
            super::super::zoom::effect_at(
                &self.project_settings.zoom_regions,
                self.timeline.playhead_us,
            ),
            self.cursor_frame,
        ))
    }

    pub(super) fn update_canvas_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let hit = self
            .canvas_geometry()
            .and_then(|geometry| editor_canvas_geometry::hit_test(geometry, position));
        if self.canvas_hover_hit != hit {
            self.canvas_hover_hit = hit;
            cx.notify();
        }
    }

    pub(super) fn clear_canvas_hover(&mut self, cx: &mut Context<Self>) {
        if self.canvas_hover_hit.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn canvas_cursor_style(&self) -> gpui::CursorStyle {
        match self.canvas_interaction {
            Some(CanvasInteraction::Move { .. }) => gpui::CursorStyle::ClosedHand,
            Some(CanvasInteraction::Resize { .. }) => gpui::CursorStyle::ResizeUpRightDownLeft,
            None => match self.canvas_hover_hit {
                Some(editor_canvas_geometry::CanvasHit::Recording) => gpui::CursorStyle::OpenHand,
                Some(editor_canvas_geometry::CanvasHit::Resize) => {
                    gpui::CursorStyle::ResizeUpRightDownLeft
                }
                None => gpui::CursorStyle::Arrow,
            },
        }
    }

    pub(super) fn begin_canvas_interaction(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let hit = self
            .canvas_geometry()
            .and_then(|geometry| editor_canvas_geometry::hit_test(geometry, position));
        self.canvas_interaction_changed = false;
        self.canvas_interaction = match hit {
            Some(editor_canvas_geometry::CanvasHit::Recording) => {
                self.selected_recording = true;
                Some(CanvasInteraction::Move { last: position })
            }
            Some(editor_canvas_geometry::CanvasHit::Resize) => {
                let Some(geometry) = self.canvas_geometry() else {
                    return;
                };
                self.selected_recording = true;
                Some(CanvasInteraction::Resize {
                    center: geometry.composition_layer.center(),
                    start_scale: self.project_settings.canvas_composition.scale,
                    start_distance: distance(position, geometry.composition_layer.center())
                        .max(1.0),
                })
            }
            None => {
                self.selected_recording = false;
                None
            }
        };
        cx.notify();
    }

    pub(super) fn update_canvas_interaction(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(interaction) = self.canvas_interaction else {
            return;
        };

        match interaction {
            CanvasInteraction::Move { last } => {
                let Some(geometry) = self.canvas_geometry() else {
                    return;
                };
                let width = geometry.canvas.size.width.as_f32();
                let height = geometry.canvas.size.height.as_f32();
                if width <= 0.0 || height <= 0.0 {
                    return;
                }

                let mut composition = self.project_settings.canvas_composition.clone();
                composition.position_x +=
                    f64::from(position.x.as_f32() - last.x.as_f32()) / f64::from(width);
                composition.position_y +=
                    f64::from(position.y.as_f32() - last.y.as_f32()) / f64::from(height);
                if self.update_canvas_composition(composition, cx) {
                    self.canvas_interaction_changed = true;
                }
                self.canvas_interaction = Some(CanvasInteraction::Move { last: position });
            }
            CanvasInteraction::Resize {
                center,
                start_scale,
                start_distance,
            } => {
                let scale = start_scale * f64::from(distance(position, center) / start_distance);
                let mut composition = self.project_settings.canvas_composition.clone();
                composition.scale = scale;
                if self.update_canvas_composition(composition, cx) {
                    self.canvas_interaction_changed = true;
                    let value = self.project_settings.canvas_composition.scale as f32;
                    self.canvas_controls
                        .scale
                        .update(cx, |slider, cx| slider.set_value(value, window, cx));
                }
            }
        }
    }

    pub(super) fn end_canvas_interaction(&mut self, cx: &mut Context<Self>) {
        if self.canvas_interaction.take().is_none() {
            return;
        }
        if self.canvas_interaction_changed {
            self.canvas_interaction_changed = false;
            self.persist_settings(cx);
        }
        cx.notify();
    }

    fn update_canvas_composition(
        &mut self,
        composition: CanvasComposition,
        cx: &mut Context<Self>,
    ) -> bool {
        let composition = composition.normalized();
        if self.project_settings.canvas_composition == composition {
            return false;
        }
        self.project_settings.canvas_composition = composition;
        cx.notify();
        true
    }

    fn set_canvas_composition(&mut self, composition: CanvasComposition, cx: &mut Context<Self>) {
        if self.update_canvas_composition(composition, cx) {
            self.persist_settings(cx);
        }
    }

    pub(super) fn set_canvas_background_kind(
        &mut self,
        kind: CanvasBackgroundKind,
        cx: &mut Context<Self>,
    ) {
        self.background_load_id = self.background_load_id.wrapping_add(1);
        let mut composition = self.project_settings.canvas_composition.clone();
        composition.background.kind = kind;
        self.set_canvas_composition(composition, cx);
    }

    pub(super) fn set_canvas_shadow(&mut self, shadow: bool, cx: &mut Context<Self>) {
        let mut composition = self.project_settings.canvas_composition.clone();
        composition.shadow = shadow;
        self.set_canvas_composition(composition, cx);
    }

    fn set_canvas_color(&mut self, target: CanvasColor, color: Hsla, cx: &mut Context<Self>) {
        let mut composition = self.project_settings.canvas_composition.clone();
        let value = editor_canvas_controls::color_value(color);
        match target {
            CanvasColor::Solid => composition.background.solid_color = Some(value),
            CanvasColor::GradientStart => composition.background.gradient_start = Some(value),
            CanvasColor::GradientEnd => composition.background.gradient_end = Some(value),
        }
        self.set_canvas_composition(composition, cx);
    }

    pub(super) fn choose_background_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.background_load_id = self.background_load_id.wrapping_add(1);
        let request_id = self.background_load_id;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a canvas background image".into()),
        });
        cx.spawn_in(window, async move |view, cx| {
            let path = match paths.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    view.update(cx, |view, cx| {
                        view.report_warning(
                            format!("Could not open the background image picker: {error}"),
                            cx,
                        );
                    })
                    .ok();
                    None
                }
                Err(error) => {
                    view.update(cx, |view, cx| {
                        view.report_warning(
                            format!("Could not open the background image picker: {error}"),
                            cx,
                        );
                    })
                    .ok();
                    None
                }
            };
            let Some(path) = path else {
                return;
            };

            let path_for_read = path.clone();
            let file = cx
                .background_spawn(async move { read_background_image(&path_for_read) })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.finish_background_image_load(request_id, path, file, cx)
            });
        })
        .detach();
    }

    fn start_background_image_load(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(path) = path else {
            return;
        };
        self.background_load_id = self.background_load_id.wrapping_add(1);
        let request_id = self.background_load_id;
        let path_for_read = path.clone();
        let view = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let file = cx
                .background_spawn(async move { read_background_image(&path_for_read) })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.finish_background_image_load(request_id, path, file, cx)
            });
        })
        .detach();
    }

    fn finish_background_image_load(
        &mut self,
        request_id: u64,
        path: PathBuf,
        file: Result<(ImageFormat, Vec<u8>)>,
        cx: &mut Context<Self>,
    ) {
        if self.background_load_id != request_id {
            return;
        }
        match file {
            Ok((format, bytes)) => {
                match Image::from_bytes(format, bytes).to_image_data(cx.svg_renderer()) {
                    Ok(image) => self.set_loaded_background(path, image, cx),
                    Err(error) => self.report_warning(
                        format!("Could not decode the background image: {error}"),
                        cx,
                    ),
                }
            }
            Err(error) => self.report_warning(error.to_string(), cx),
        }
    }

    fn set_loaded_background(
        &mut self,
        path: PathBuf,
        image: Arc<RenderImage>,
        cx: &mut Context<Self>,
    ) {
        self.background_image = Some(image);
        let mut composition = self.project_settings.canvas_composition.clone();
        composition.background.kind = CanvasBackgroundKind::Image;
        composition.background.image_path = Some(path);
        self.set_canvas_composition(composition, cx);
    }

    fn apply_events(&mut self, events: Vec<PlaybackEvent>, cx: &mut Context<Self>) {
        let latest_frame = events.iter().rev().find_map(|event| match event {
            PlaybackEvent::Frame { timing, .. } => Some(timing.sequence),
            _ => None,
        });

        for event in events {
            if let PlaybackEvent::Frame { ref timing, .. } = event
                && Some(timing.sequence) != latest_frame
            {
                self.metrics.frame_coalesced();
                continue;
            }
            self.apply_event(event, cx);
        }
    }

    fn apply_event(&mut self, event: PlaybackEvent, cx: &mut Context<Self>) {
        let update_started = Instant::now();
        match event {
            PlaybackEvent::Ready {
                duration,
                width,
                height,
            } => {
                self.timeline.set_duration_seconds(duration);
                self.video_width = width;
                self.video_height = height;
                if self.normalize_regions_for_duration() {
                    self.persist_settings(cx);
                }
                self.maybe_generate_auto_zooms(cx);
                cx.notify();
            }
            PlaybackEvent::Frame {
                seconds,
                image,
                timing,
            } => {
                let received_at = Instant::now();
                if timing.seek_generation < self.latest_seek_generation {
                    self.metrics.stale_event_dropped();
                } else if !self.playing || self.accept_preview_frame(seconds) {
                    self.metrics.frame_received(&timing, received_at);
                    let previous = self.image.replace(image);
                    if let Some(previous) = previous {
                        // Release the old image from the current playback window during its
                        // render pass. App::drop_image would walk every GPUI window for every
                        // video frame, adding unrelated UI locks to the hot path.
                        self.pending_image_releases.push(previous);
                    }
                    let display_seconds = self.display_seconds_for_frame(seconds);
                    self.timeline.set_playhead_seconds(display_seconds);
                    self.update_cursor(display_seconds);
                    self.update_motion_blur(display_seconds);
                    self.frame_timing = Some(timing.clone());
                    let invalidated_at = Instant::now();
                    self.frame_invalidated_at = Some(invalidated_at);
                    cx.notify();
                    self.metrics
                        .frame_invalidated(&timing, received_at, invalidated_at);
                }
            }
            PlaybackEvent::Time {
                seconds,
                seek_generation,
            } => {
                if seek_generation < self.latest_seek_generation {
                    self.metrics.stale_event_dropped();
                } else {
                    self.pending_seek_target = None;
                    self.timeline.set_playhead_seconds(seconds);
                    self.update_cursor(seconds);
                    cx.notify();
                }
            }
            PlaybackEvent::State(playing) => {
                if !playing {
                    self.last_preview_slot = None;
                    self.metrics.reset_presented();
                }
                if self.playing != playing {
                    self.playing = playing;
                    cx.notify();
                }
            }
            PlaybackEvent::Error(error) => {
                self.metrics.reset_presented();
                self.report_error(error, cx);
            }
        }
        self.metrics.gpui_update(update_started.elapsed());
    }

    fn update_cursor(&mut self, seconds: f64) {
        let started = Instant::now();
        let mut settings = self.project_settings.cursor.normalized();
        settings.scale = cursor_scale_at(
            &self.project_settings.cursor_size_regions,
            editor_timeline::seconds_to_micros(seconds),
            settings.scale,
        );
        self.cursor_frame = self.cursor_overlay.frame_at(seconds, settings);
        self.metrics.cursor_updated(started.elapsed());
    }

    fn update_motion_blur(&mut self, seconds: f64) {
        // A scrub jumps the playhead between presented frames. Seek generations
        // already cover most of that, but sub-frame drag steps are coalesced
        // rather than published, so dragging is made sharp explicitly.
        if self.timeline.scrubbing() {
            self.reset_motion_blur();
            return;
        }
        let started = Instant::now();
        let released = self.motion_blur.present(PresentedFrame {
            seconds,
            seek_generation: self.latest_seek_generation,
            cursor: self.cursor_frame,
            geometry: self.canvas_geometry(),
            video_width: self.video_width,
            video_height: self.video_height,
            cursor_images: &self.cursor_images,
            settings: self.project_settings.motion_blur,
        });
        self.release_image(released);
        self.metrics
            .motion_blur_classified(self.motion_blur.display().mode);
        self.metrics.motion_blur_prepared(started.elapsed());
    }

    fn reset_motion_blur(&mut self) {
        let released = self.motion_blur.reset();
        self.release_image(released);
    }

    /// Queues an image for release from this window during its next render
    /// pass, the same path the video frame uses.
    fn release_image(&mut self, image: Option<Arc<RenderImage>>) {
        if let Some(image) = image {
            self.pending_image_releases.push(image);
        }
    }

    pub(super) fn set_motion_blur_amount(&mut self, amount: f32, cx: &mut Context<Self>) {
        self.project_settings.motion_blur.amount = amount;
        self.project_settings.motion_blur = self.project_settings.motion_blur.normalized();
        if self.project_settings.motion_blur.is_disabled() {
            self.reset_motion_blur();
        }
        self.persist_settings(cx);
        cx.notify();
    }

    pub(super) fn set_cursor_settings(&mut self, settings: CursorSettings, cx: &mut Context<Self>) {
        self.project_settings.cursor = settings.normalized();
        self.persist_settings(cx);
        self.update_cursor(self.timeline.playhead_seconds());
        cx.notify();
    }

    /// Auto-saves the current project settings so every editor change persists.
    fn persist_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(error) = self.save_queue.take_error() {
            self.report_error(error, cx);
        }
        self.save_queue.request(&self.project_settings);
    }

    pub(super) fn toggle(&mut self, cx: &mut Context<Self>) {
        let replay = reached_end(
            self.timeline.playhead_seconds(),
            self.timeline.duration_seconds(),
        );
        let playing = replay || !self.playing;
        let result = (|| {
            if replay {
                self.timeline.set_playhead_seconds(0.0);
                self.update_cursor(0.0);
                self.pending_seek_target = Some(0.0);
                let generation = self
                    .player
                    .as_ref()
                    .ok_or_else(|| anyhow!("recording player is unavailable"))?
                    .seek(0.0)?;
                self.latest_seek_generation = generation;
                self.last_published_seek_us = Some(0);
                self.frame_timing = None;
                self.frame_invalidated_at = None;
                self.last_preview_slot = None;
                self.reset_motion_blur();
                self.metrics.reset_presented();
            }
            self.player
                .as_ref()
                .ok_or_else(|| anyhow!("recording player is unavailable"))?
                .set_playing(playing)
        })();
        match result {
            Ok(()) => {
                self.playing = playing;
                if !playing {
                    self.metrics.reset_presented();
                }
                self.error = None;
                cx.notify();
            }
            Err(error) => self.report_error(error.to_string(), cx),
        }
    }

    pub(super) fn seek_to(&mut self, seconds: f64, cx: &mut Context<Self>) {
        let target = seconds.max(0.0).min(self.timeline.duration_seconds());
        self.set_logical_playhead(target);
        self.publish_seek(target, None, cx);
        cx.notify();
    }

    pub(super) fn begin_timeline_scrub(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(bounds) = *self.timeline_bounds.borrow() else {
            return;
        };
        let pointer_started = Instant::now();
        self.metrics.scrub_pointer_moved();
        if let Some(seconds) = self.timeline.begin_scrub(position, bounds) {
            self.set_logical_playhead(seconds);
            self.publish_seek(seconds, Some(pointer_started), cx);
            cx.notify();
        }
    }

    pub(super) fn update_timeline_scrub(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = *self.timeline_bounds.borrow() else {
            return;
        };
        let pointer_started = Instant::now();
        self.metrics.scrub_pointer_moved();
        if let Some(seconds) = self.timeline.update_scrub(position, bounds) {
            self.set_logical_playhead(seconds);
            let target_us = editor_timeline::seconds_to_micros(seconds);
            let should_publish = should_publish_scrub_seek(self.last_published_seek_us, target_us);
            if should_publish {
                self.publish_seek(seconds, Some(pointer_started), cx);
            }
            cx.notify();
        }
    }

    pub(super) fn end_timeline_scrub(&mut self, cx: &mut Context<Self>) {
        if self.timeline.scrubbing() {
            let target = self.timeline.playhead_seconds();
            let target_us = editor_timeline::seconds_to_micros(target);
            if self.last_published_seek_us != Some(target_us) {
                self.publish_seek(target, None, cx);
            }
            self.timeline.end_scrub();
            cx.notify();
        }
    }

    pub(super) fn update_timeline_hover(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let (cursor_hit, zoom_hit) =
            (*self.timeline_bounds.borrow()).map_or((None, None), |bounds| {
                (
                    editor_timeline::hit_test_cursor_size_region(
                        position,
                        bounds,
                        self.timeline,
                        &self.project_settings.cursor_size_regions,
                    ),
                    editor_timeline::hit_test_zoom_region(
                        position,
                        bounds,
                        self.timeline,
                        &self.project_settings.zoom_regions,
                    ),
                )
            });
        if self.hovered_cursor_size_hit != cursor_hit || self.hovered_zoom_hit != zoom_hit {
            self.hovered_cursor_size_hit = cursor_hit;
            self.hovered_zoom_hit = zoom_hit;
            cx.notify();
        }
    }

    pub(super) fn clear_timeline_hover(&mut self, cx: &mut Context<Self>) {
        if self.hovered_cursor_size_hit.take().is_some() || self.hovered_zoom_hit.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn timeline_cursor_style(&self) -> gpui::CursorStyle {
        self.timeline_interaction
            .map(editor_timeline::TimelineInteraction::cursor_style)
            .or_else(|| {
                self.hovered_cursor_size_hit
                    .map(editor_timeline::CursorSizeHit::cursor_style)
            })
            .or_else(|| {
                self.hovered_zoom_hit
                    .map(editor_timeline::ZoomHit::cursor_style)
            })
            .unwrap_or(gpui::CursorStyle::Arrow)
    }

    pub(super) fn begin_timeline_interaction(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = *self.timeline_bounds.borrow() else {
            return;
        };
        if !editor_timeline::in_viewport(position, bounds) {
            return;
        }
        let cursor_hit = editor_timeline::hit_test_cursor_size_region(
            position,
            bounds,
            self.timeline,
            &self.project_settings.cursor_size_regions,
        );
        if let Some(hit) = cursor_hit {
            let index = hit.index();
            self.selected_cursor_size_region = Some(index);
            self.selected_zoom_region = None;
            self.hovered_cursor_size_hit = Some(hit);
            self.cursor_size_interaction_changed = false;
            self.timeline_interaction = Some(match hit {
                editor_timeline::CursorSizeHit::Body { index } => {
                    let pointer_us = self.timeline.time_at_position(position, bounds);
                    let grab_offset_us = pointer_us
                        .saturating_sub(self.project_settings.cursor_size_regions[index].start_us);
                    editor_timeline::TimelineInteraction::MoveCursorSize {
                        index,
                        grab_offset_us,
                    }
                }
                editor_timeline::CursorSizeHit::Start { index } => {
                    editor_timeline::TimelineInteraction::ResizeCursorSizeStart { index }
                }
                editor_timeline::CursorSizeHit::End { index } => {
                    editor_timeline::TimelineInteraction::ResizeCursorSizeEnd { index }
                }
            });
            cx.notify();
            return;
        }

        let zoom_hit = editor_timeline::hit_test_zoom_region(
            position,
            bounds,
            self.timeline,
            &self.project_settings.zoom_regions,
        );
        if let Some(hit) = zoom_hit {
            let index = hit.index();
            self.selected_zoom_region = Some(index);
            self.selected_cursor_size_region = None;
            self.hovered_zoom_hit = Some(hit);
            self.zoom_interaction_changed = false;
            self.timeline_interaction = Some(match hit {
                editor_timeline::ZoomHit::Body { index } => {
                    let pointer_us = self.timeline.time_at_position(position, bounds);
                    let grab_offset_us = pointer_us
                        .saturating_sub(self.project_settings.zoom_regions[index].start_us);
                    editor_timeline::TimelineInteraction::MoveZoom {
                        index,
                        grab_offset_us,
                    }
                }
                editor_timeline::ZoomHit::Start { index } => {
                    editor_timeline::TimelineInteraction::ResizeZoomStart { index }
                }
                editor_timeline::ZoomHit::End { index } => {
                    editor_timeline::TimelineInteraction::ResizeZoomEnd { index }
                }
                editor_timeline::ZoomHit::ZoomInEnd { index } => {
                    editor_timeline::TimelineInteraction::ResizeZoomInEnd { index }
                }
                editor_timeline::ZoomHit::ZoomOutStart { index } => {
                    editor_timeline::TimelineInteraction::ResizeZoomOutStart { index }
                }
            });
            cx.notify();
            return;
        }

        self.timeline_interaction = Some(editor_timeline::TimelineInteraction::Scrub);
        self.begin_timeline_scrub(position, cx);
    }

    pub(super) fn update_timeline_interaction(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(interaction) = self.timeline_interaction else {
            return;
        };
        match interaction {
            editor_timeline::TimelineInteraction::Scrub => {
                self.update_timeline_scrub(position, cx);
            }
            editor_timeline::TimelineInteraction::MoveCursorSize {
                index,
                grab_offset_us,
            } => self.move_cursor_size_region(index, grab_offset_us, position, cx),
            editor_timeline::TimelineInteraction::ResizeCursorSizeStart { index } => {
                self.resize_cursor_size_start(index, position, cx);
            }
            editor_timeline::TimelineInteraction::ResizeCursorSizeEnd { index } => {
                self.resize_cursor_size_end(index, position, cx);
            }
            editor_timeline::TimelineInteraction::MoveZoom {
                index,
                grab_offset_us,
            } => self.move_zoom_region(index, grab_offset_us, position, cx),
            editor_timeline::TimelineInteraction::ResizeZoomStart { index } => {
                self.resize_zoom_region_start(index, position, cx);
            }
            editor_timeline::TimelineInteraction::ResizeZoomEnd { index } => {
                self.resize_zoom_region_end(index, position, cx);
            }
            editor_timeline::TimelineInteraction::ResizeZoomInEnd { index } => {
                self.resize_zoom_in_end(index, position, cx);
            }
            editor_timeline::TimelineInteraction::ResizeZoomOutStart { index } => {
                self.resize_zoom_out_start(index, position, cx);
            }
        }
    }

    pub(super) fn end_timeline_interaction(&mut self, cx: &mut Context<Self>) {
        let Some(interaction) = self.timeline_interaction.take() else {
            return;
        };
        if matches!(interaction, editor_timeline::TimelineInteraction::Scrub) {
            self.end_timeline_scrub(cx);
        } else if self.zoom_interaction_changed || self.cursor_size_interaction_changed {
            self.zoom_interaction_changed = false;
            self.cursor_size_interaction_changed = false;
            self.persist_settings(cx);
            cx.notify();
        }
    }

    fn cursor_size_snap_points(&self, index: usize) -> Vec<u64> {
        let mut points = vec![0, self.timeline.duration_us, self.timeline.playhead_us];
        points.extend(
            self.project_settings
                .cursor_size_regions
                .iter()
                .enumerate()
                .filter(|(other_index, _)| *other_index != index)
                .flat_map(|(_, region)| [region.start_us, region.end_us]),
        );
        points.extend(
            self.project_settings
                .zoom_regions
                .iter()
                .flat_map(|region| [region.start_us, region.end_us]),
        );
        points.sort_unstable();
        points.dedup();
        points
    }

    fn move_cursor_size_region(
        &mut self,
        index: usize,
        grab_offset_us: u64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = *self.timeline_bounds.borrow() else {
            return;
        };
        let pointer_us = self.timeline.time_at_position(position, bounds);
        let duration_us = self.timeline.duration_us;
        let Some(region) = self
            .project_settings
            .cursor_size_regions
            .get(index)
            .copied()
        else {
            return;
        };
        let length_us = region.duration_us();
        let proposed_start_us = pointer_us
            .saturating_sub(grab_offset_us)
            .min(duration_us.saturating_sub(length_us));
        let (snapped_start_us, _) = editor_timeline::snap_range(
            proposed_start_us,
            proposed_start_us.saturating_add(length_us),
            self.timeline,
            &self.cursor_size_snap_points(index),
        );
        let start_us = snapped_start_us.min(duration_us.saturating_sub(length_us));
        let end_us = start_us.saturating_add(length_us).min(duration_us);
        let (ease_in_offset_us, ease_out_offset_us) = {
            let (ease_in_end_us, ease_out_start_us) = region.transition_points();
            (
                ease_in_end_us.saturating_sub(region.start_us),
                ease_out_start_us.saturating_sub(region.start_us),
            )
        };
        if region.start_us != start_us || region.end_us != end_us {
            let Some(region) = self.project_settings.cursor_size_regions.get_mut(index) else {
                return;
            };
            region.start_us = start_us;
            region.end_us = end_us;
            region.ease_in_end_us = Some(start_us.saturating_add(ease_in_offset_us).min(end_us));
            region.ease_out_start_us =
                Some(start_us.saturating_add(ease_out_offset_us).min(end_us));
            self.cursor_size_interaction_changed = true;
            self.update_cursor(self.timeline.playhead_seconds());
            cx.notify();
        }
    }

    fn resize_cursor_size_start(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = *self.timeline_bounds.borrow() else {
            return;
        };
        let pointer_us = self.timeline.time_at_position(position, bounds);
        let snapped_us = editor_timeline::snap_time(
            pointer_us,
            self.timeline,
            &self.cursor_size_snap_points(index),
        );
        let Some(current) = self
            .project_settings
            .cursor_size_regions
            .get(index)
            .copied()
        else {
            return;
        };
        let minimum = MIN_CURSOR_SIZE_REGION_DURATION_US.min(self.timeline.duration_us);
        let start_us = snapped_us.min(current.end_us.saturating_sub(minimum));
        if current.start_us != start_us {
            let Some(region) = self.project_settings.cursor_size_regions.get_mut(index) else {
                return;
            };
            region.start_us = start_us;
            set_cursor_transition_points(region);
            self.cursor_size_interaction_changed = true;
            self.update_cursor(self.timeline.playhead_seconds());
            cx.notify();
        }
    }

    fn resize_cursor_size_end(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = *self.timeline_bounds.borrow() else {
            return;
        };
        let pointer_us = self.timeline.time_at_position(position, bounds);
        let snapped_us = editor_timeline::snap_time(
            pointer_us,
            self.timeline,
            &self.cursor_size_snap_points(index),
        );
        let Some(current) = self
            .project_settings
            .cursor_size_regions
            .get(index)
            .copied()
        else {
            return;
        };
        let minimum = MIN_CURSOR_SIZE_REGION_DURATION_US.min(self.timeline.duration_us);
        let end_us = snapped_us
            .max(current.start_us.saturating_add(minimum))
            .min(self.timeline.duration_us);
        if current.end_us != end_us {
            let Some(region) = self.project_settings.cursor_size_regions.get_mut(index) else {
                return;
            };
            region.end_us = end_us;
            set_cursor_transition_points(region);
            self.cursor_size_interaction_changed = true;
            self.update_cursor(self.timeline.playhead_seconds());
            cx.notify();
        }
    }

    fn normalize_regions_for_duration(&mut self) -> bool {
        let before_zoom = self.project_settings.zoom_regions.clone();
        let before_cursor = self.project_settings.cursor_size_regions.clone();
        let duration_us = self.timeline.duration_us;
        self.project_settings.zoom_regions = self
            .project_settings
            .zoom_regions
            .iter()
            .copied()
            .filter_map(|region| region.normalized_for_duration(duration_us))
            .collect();
        self.project_settings.cursor_size_regions = self
            .project_settings
            .cursor_size_regions
            .iter()
            .copied()
            .filter_map(|region| region.normalized_for_duration(duration_us))
            .collect();
        self.selected_zoom_region = self
            .selected_zoom_region
            .filter(|index| *index < self.project_settings.zoom_regions.len());
        self.selected_cursor_size_region = self
            .selected_cursor_size_region
            .filter(|index| *index < self.project_settings.cursor_size_regions.len());
        self.hovered_zoom_hit = self
            .hovered_zoom_hit
            .filter(|hit| hit.index() < self.project_settings.zoom_regions.len());
        self.hovered_cursor_size_hit = self
            .hovered_cursor_size_hit
            .filter(|hit| hit.index() < self.project_settings.cursor_size_regions.len());
        before_zoom != self.project_settings.zoom_regions
            || before_cursor != self.project_settings.cursor_size_regions
    }

    pub(super) fn add_cursor_size_keyframe(&mut self, cx: &mut Context<Self>) {
        let Some(mut region) =
            CursorSizeRegion::new_at(self.timeline.playhead_us, self.timeline.duration_us)
        else {
            return;
        };
        let base_scale = self.project_settings.cursor.normalized().scale;
        region.start_scale = cursor_scale_at(
            &self.project_settings.cursor_size_regions,
            self.timeline.playhead_us,
            base_scale,
        );
        region.end_scale = (region.start_scale + 0.5).min(MAX_CURSOR_SCALE);
        self.project_settings.cursor_size_regions.push(region);
        self.selected_cursor_size_region =
            Some(self.project_settings.cursor_size_regions.len() - 1);
        self.selected_zoom_region = None;
        self.hovered_zoom_hit = None;
        self.hovered_cursor_size_hit = None;
        self.persist_settings(cx);
        self.update_cursor(self.timeline.playhead_seconds());
        cx.notify();
    }

    pub(super) fn delete_selected_cursor_size_region(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_cursor_size_region else {
            return;
        };
        if index >= self.project_settings.cursor_size_regions.len() {
            self.selected_cursor_size_region = None;
            cx.notify();
            return;
        }
        self.project_settings.cursor_size_regions.remove(index);
        self.selected_cursor_size_region = None;
        self.hovered_cursor_size_hit = None;
        self.persist_settings(cx);
        self.update_cursor(self.timeline.playhead_seconds());
        cx.notify();
    }

    pub(super) fn adjust_selected_cursor_size(&mut self, amount: f32, cx: &mut Context<Self>) {
        let Some(index) = self.selected_cursor_size_region else {
            return;
        };
        let Some(region) = self.project_settings.cursor_size_regions.get_mut(index) else {
            return;
        };
        region.end_scale = (region.end_scale + amount).clamp(MIN_CURSOR_SCALE, MAX_CURSOR_SCALE);
        self.persist_settings(cx);
        self.update_cursor(self.timeline.playhead_seconds());
        cx.notify();
    }

    fn zoom_snap_points(&self, index: usize) -> Vec<u64> {
        let mut points = vec![0, self.timeline.duration_us, self.timeline.playhead_us];
        points.extend(
            self.project_settings
                .zoom_regions
                .iter()
                .enumerate()
                .filter(|(other_index, _)| *other_index != index)
                .flat_map(|(_, region)| [region.start_us, region.end_us]),
        );
        points.sort_unstable();
        points.dedup();
        points
    }

    fn move_zoom_region(
        &mut self,
        index: usize,
        grab_offset_us: u64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = *self.timeline_bounds.borrow() else {
            return;
        };
        let pointer_us = self.timeline.time_at_position(position, bounds);
        let duration_us = self.timeline.duration_us;
        let Some(region) = self.project_settings.zoom_regions.get(index).copied() else {
            return;
        };
        let length_us = region.duration_us();
        let proposed_start_us = pointer_us
            .saturating_sub(grab_offset_us)
            .min(duration_us.saturating_sub(length_us));
        let (snapped_start_us, _) = editor_timeline::snap_range(
            proposed_start_us,
            proposed_start_us.saturating_add(length_us),
            self.timeline,
            &self.zoom_snap_points(index),
        );
        let start_us = snapped_start_us.min(duration_us.saturating_sub(length_us));
        let end_us = start_us.saturating_add(length_us).min(duration_us);
        let (zoom_in_offset_us, zoom_out_offset_us) = {
            let (zoom_in_end_us, zoom_out_start_us) = region.transition_points();
            (
                zoom_in_end_us.saturating_sub(region.start_us),
                zoom_out_start_us.saturating_sub(region.start_us),
            )
        };
        if region.start_us != start_us || region.end_us != end_us {
            let Some(region) = self.project_settings.zoom_regions.get_mut(index) else {
                return;
            };
            region.start_us = start_us;
            region.end_us = end_us;
            region.zoom_in_end_us = Some(start_us.saturating_add(zoom_in_offset_us).min(end_us));
            region.zoom_out_start_us =
                Some(start_us.saturating_add(zoom_out_offset_us).min(end_us));
            self.zoom_interaction_changed = true;
            cx.notify();
        }
    }

    fn resize_zoom_region_start(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = *self.timeline_bounds.borrow() else {
            return;
        };
        let pointer_us = self.timeline.time_at_position(position, bounds);
        let duration_us = self.timeline.duration_us;
        let snapped_us =
            editor_timeline::snap_time(pointer_us, self.timeline, &self.zoom_snap_points(index));
        let Some(current) = self.project_settings.zoom_regions.get(index).copied() else {
            return;
        };
        let minimum = MIN_ZOOM_REGION_DURATION_US.min(duration_us);
        let start_us = snapped_us.min(current.end_us.saturating_sub(minimum));
        if current.start_us != start_us {
            let Some(region) = self.project_settings.zoom_regions.get_mut(index) else {
                return;
            };
            region.start_us = start_us;
            set_transition_points(region);
            self.zoom_interaction_changed = true;
            cx.notify();
        }
    }

    fn resize_zoom_region_end(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = *self.timeline_bounds.borrow() else {
            return;
        };
        let pointer_us = self.timeline.time_at_position(position, bounds);
        let duration_us = self.timeline.duration_us;
        let snapped_us =
            editor_timeline::snap_time(pointer_us, self.timeline, &self.zoom_snap_points(index));
        let Some(current) = self.project_settings.zoom_regions.get(index).copied() else {
            return;
        };
        let minimum = MIN_ZOOM_REGION_DURATION_US.min(duration_us);
        let end_us = snapped_us
            .max(current.start_us.saturating_add(minimum))
            .min(duration_us);
        if current.end_us != end_us {
            let Some(region) = self.project_settings.zoom_regions.get_mut(index) else {
                return;
            };
            region.end_us = end_us;
            set_transition_points(region);
            self.zoom_interaction_changed = true;
            cx.notify();
        }
    }

    fn resize_zoom_in_end(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = *self.timeline_bounds.borrow() else {
            return;
        };
        let pointer_us = self.timeline.time_at_position(position, bounds);
        let snapped_us =
            editor_timeline::snap_time(pointer_us, self.timeline, &self.zoom_snap_points(index));
        let Some(current) = self.project_settings.zoom_regions.get(index).copied() else {
            return;
        };
        let (_, zoom_out_start_us) = current.transition_points();
        let zoom_in_end_us = snapped_us.clamp(current.start_us, zoom_out_start_us);
        if current.zoom_in_end_us != Some(zoom_in_end_us) {
            let Some(region) = self.project_settings.zoom_regions.get_mut(index) else {
                return;
            };
            region.zoom_in_end_us = Some(zoom_in_end_us);
            self.zoom_interaction_changed = true;
            cx.notify();
        }
    }

    fn resize_zoom_out_start(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = *self.timeline_bounds.borrow() else {
            return;
        };
        let pointer_us = self.timeline.time_at_position(position, bounds);
        let snapped_us =
            editor_timeline::snap_time(pointer_us, self.timeline, &self.zoom_snap_points(index));
        let Some(current) = self.project_settings.zoom_regions.get(index).copied() else {
            return;
        };
        let (zoom_in_end_us, _) = current.transition_points();
        let zoom_out_start_us = snapped_us.clamp(zoom_in_end_us, current.end_us);
        if current.zoom_out_start_us != Some(zoom_out_start_us) {
            let Some(region) = self.project_settings.zoom_regions.get_mut(index) else {
                return;
            };
            region.zoom_out_start_us = Some(zoom_out_start_us);
            self.zoom_interaction_changed = true;
            cx.notify();
        }
    }

    pub(super) fn add_zoom_region(&mut self, cx: &mut Context<Self>) {
        let Some(region) = ZoomRegion::new_at(self.timeline.playhead_us, self.timeline.duration_us)
        else {
            return;
        };
        self.project_settings.zoom_regions.push(region);
        self.selected_zoom_region = Some(self.project_settings.zoom_regions.len() - 1);
        self.selected_cursor_size_region = None;
        self.hovered_cursor_size_hit = None;
        self.persist_settings(cx);
        cx.notify();
    }

    fn maybe_generate_auto_zooms(&mut self, cx: &mut Context<Self>) {
        if self.generate_auto_zooms_on_open
            && !self.auto_zooms_generated
            && !self.cursor_overlay_loading
            && self.timeline.duration_us > 0
        {
            self.auto_zooms_generated = true;
            self.generate_auto_zooms(cx);
        }
    }

    pub(super) fn generate_auto_zooms(&mut self, cx: &mut Context<Self>) {
        let existing_count = self.project_settings.zoom_regions.len();
        let duration_us = self.timeline.duration_us;
        let duration_seconds = self.timeline.duration_seconds();
        let telemetry_available = self.cursor_overlay.has_telemetry();
        tracing::info!(
            target: "recorder::playback",
            duration_us,
            duration_seconds,
            existing_regions = existing_count,
            telemetry_available,
            "auto-zoom generation requested"
        );
        if duration_us == 0 {
            self.report_warning(
                "Auto zooms are unavailable because the recording duration has not loaded yet.",
                cx,
            );
            return;
        }

        let (generated, report) = self.cursor_overlay.auto_zoom_regions_with_report(
            self.timeline.duration_us,
            &self.project_settings.zoom_regions,
        );
        let generated_count = generated.len();
        tracing::info!(
            target: "recorder::playback",
            duration_us,
            clicks = report.clicks,
            clusters = report.clusters,
            candidates = report.candidates,
            generated = report.generated,
            existing_regions = existing_count,
            "auto-zoom generation analyzed"
        );
        if generated_count == 0 {
            let warning = if !telemetry_available {
                format!(
                    "Auto zooms could not be generated for this {duration_seconds:.2}s recording: cursor telemetry is unavailable, so there are no click events to analyze."
                )
            } else if report.clicks == 0 {
                format!(
                    "No auto-zoom regions were generated for this {duration_seconds:.2}s recording: no qualifying click activity was detected. Completed clicks, double-clicks, and context clicks are required; cursor movement and hovering are ignored."
                )
            } else if report.candidates == 0 {
                format!(
                    "Auto zooms could not be generated for this {duration_seconds:.2}s recording: {} qualifying click(s) were found, but none fit a usable region inside the recording bounds. The recording may be too short.",
                    report.clicks
                )
            } else {
                format!(
                    "No auto-zoom regions were generated: {} candidate(s) from {} click(s) overlap the {} existing zoom region(s). Existing regions were preserved.",
                    report.candidates, report.clicks, existing_count
                )
            };
            self.report_warning(warning, cx);
            return;
        }

        self.project_settings.zoom_regions.extend(generated);
        // New regions replace the composition transform discontinuously, so the
        // pending measurement no longer describes the layer that is on screen.
        self.reset_motion_blur();
        self.selected_zoom_region = Some(existing_count);
        self.selected_cursor_size_region = None;
        self.hovered_cursor_size_hit = None;
        self.hovered_zoom_hit = None;
        tracing::info!(
            target: "recorder::playback",
            generated = generated_count,
            existing = existing_count,
            "generated auto zoom regions"
        );
        self.persist_settings(cx);
        cx.notify();
    }

    pub(super) fn delete_selected_zoom_region(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_zoom_region else {
            return;
        };
        if index >= self.project_settings.zoom_regions.len() {
            self.selected_zoom_region = None;
            cx.notify();
            return;
        }
        self.project_settings.zoom_regions.remove(index);
        self.selected_zoom_region = None;
        self.selected_cursor_size_region = None;
        self.hovered_zoom_hit = None;
        self.hovered_cursor_size_hit = None;
        self.persist_settings(cx);
        cx.notify();
    }

    pub(super) fn adjust_selected_zoom_scale(&mut self, amount: f32, cx: &mut Context<Self>) {
        let Some(index) = self.selected_zoom_region else {
            return;
        };
        let Some(region) = self.project_settings.zoom_regions.get_mut(index) else {
            return;
        };
        region.scale = (region.scale + amount).clamp(1.0, MAX_ZOOM_REGION_SCALE);
        self.persist_settings(cx);
        cx.notify();
    }

    pub(super) fn set_selected_zoom_target(&mut self, target: ZoomTarget, cx: &mut Context<Self>) {
        let Some(index) = self.selected_zoom_region else {
            return;
        };
        let Some(region) = self.project_settings.zoom_regions.get_mut(index) else {
            return;
        };
        if region.target != target {
            region.target = target;
            self.persist_settings(cx);
            cx.notify();
        }
    }

    fn set_logical_playhead(&mut self, seconds: f64) {
        self.timeline.set_playhead_seconds(seconds);
        let target = self.timeline.playhead_seconds();
        self.pending_seek_target = Some(target);
        self.update_cursor(target);
    }

    fn publish_seek(
        &mut self,
        seconds: f64,
        pointer_started: Option<Instant>,
        cx: &mut Context<Self>,
    ) {
        let target = seconds.max(0.0).min(self.timeline.duration_seconds());
        let result = self
            .player
            .as_ref()
            .ok_or_else(|| anyhow!("recording player is unavailable"))
            .and_then(|player| player.seek(target));
        match result {
            Ok(generation) => {
                self.latest_seek_generation = generation;
                self.last_published_seek_us = Some(editor_timeline::seconds_to_micros(target));
                self.frame_timing = None;
                self.frame_invalidated_at = None;
                self.last_preview_slot = None;
                self.metrics.reset_presented();
                self.error = None;
                if let Some(pointer_started) = pointer_started {
                    self.metrics.scrub_seek_published(pointer_started.elapsed());
                }
            }
            Err(error) => self.report_error(error.to_string(), cx),
        }
    }

    pub(super) fn scroll_timeline(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some(bounds) = *self.timeline_bounds.borrow() else {
            return;
        };
        let (delta_x, delta_y) = match event.delta {
            ScrollDelta::Pixels(delta) => (delta.x.as_f32(), delta.y.as_f32()),
            ScrollDelta::Lines(delta) => (delta.x, delta.y),
        };
        if self
            .timeline
            .handle_scroll(delta_x, delta_y, event.position, bounds)
        {
            cx.notify();
        }
    }

    pub(super) fn adjust_canvas_zoom(&mut self, amount: f64, cx: &mut Context<Self>) {
        self.adjust_canvas_zoom_at(amount, None, cx);
    }

    pub(super) fn canvas_needs_recenter(&self) -> bool {
        editor_canvas_geometry::needs_recenter(self.project_settings.canvas)
    }

    pub(super) fn reset_canvas_view(&mut self, cx: &mut Context<Self>) {
        self.set_canvas_view(CanvasView::default(), cx);
    }

    pub(super) fn zoom_from_scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = match event.delta {
            ScrollDelta::Pixels(delta) => delta.y.as_f32(),
            ScrollDelta::Lines(delta) => delta.y,
        };
        if delta != 0.0 {
            self.adjust_canvas_zoom_at(
                if delta < 0.0 { 0.1 } else { -0.1 },
                Some(event.position),
                cx,
            );
        }
    }

    fn adjust_canvas_zoom_at(
        &mut self,
        amount: f64,
        anchor: Option<Point<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        let bounds = *self.canvas_bounds.borrow();
        let current = self.project_settings.canvas;
        let next_zoom = (current.zoom + amount).clamp(MIN_CANVAS_ZOOM, MAX_CANVAS_ZOOM);
        if (next_zoom - current.zoom).abs() < f64::EPSILON {
            return;
        }

        let mut next = current;
        if let (Some(anchor), Some(bounds)) = (anchor, bounds) {
            let ratio = next_zoom / current.zoom;
            let anchor_x = anchor.x.as_f32() - bounds.origin.x.as_f32();
            let anchor_y = anchor.y.as_f32() - bounds.origin.y.as_f32();
            next.pan_x += f64::from(
                (1.0 - ratio as f32)
                    * (anchor_x - bounds.size.width.as_f32() / 2.0 - current.pan_x as f32),
            );
            next.pan_y += f64::from(
                (1.0 - ratio as f32)
                    * (anchor_y - bounds.size.height.as_f32() / 2.0 - current.pan_y as f32),
            );
        }
        next.zoom = next_zoom;
        self.set_canvas_view(next, cx);
    }

    pub(super) fn begin_pan(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.panning = true;
        self.last_pan_position = Some(position);
        cx.notify();
    }

    pub(super) fn pan_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(last) = self.last_pan_position else {
            return;
        };
        if !self.panning {
            return;
        }
        let mut canvas = self.project_settings.canvas;
        canvas.pan_x += f64::from(position.x.as_f32() - last.x.as_f32());
        canvas.pan_y += f64::from(position.y.as_f32() - last.y.as_f32());
        self.project_settings.canvas = canvas.normalized();
        self.last_pan_position = Some(position);
        cx.notify();
    }

    pub(super) fn end_pan(&mut self, cx: &mut Context<Self>) {
        if !self.panning {
            return;
        }
        self.panning = false;
        self.last_pan_position = None;
        self.persist_settings(cx);
        cx.notify();
    }

    fn set_canvas_view(&mut self, view: CanvasView, cx: &mut Context<Self>) {
        self.project_settings.canvas = view.normalized();
        self.persist_settings(cx);
        cx.notify();
    }

    fn report_error(&mut self, error: String, cx: &mut Context<Self>) {
        tracing::error!(
            target: "recorder::playback",
            error = %error,
            "playback error"
        );
        let error: SharedString = error.into();
        self.error = Some(error.clone());
        self.pending_alerts.push(AppAlert::error(error));
        cx.notify();
    }

    fn report_warning(&mut self, warning: impl Into<SharedString>, cx: &mut Context<Self>) {
        let warning: SharedString = warning.into();
        tracing::warn!(
            target: "recorder::playback",
            warning = %warning,
            "playback warning"
        );
        self.pending_alerts.push(AppAlert::warning(warning));
        cx.notify();
    }

    pub(super) fn preview_fps(&self) -> f32 {
        self.metrics.presented_fps()
    }

    pub(super) fn preview_rate(&self) -> PreviewRate {
        self.preview_rate
    }

    pub(super) fn set_preview_rate(&mut self, rate: PreviewRate, cx: &mut Context<Self>) {
        if self.preview_rate == rate {
            return;
        }
        self.preview_rate = rate;
        self.last_preview_slot = None;
        // A new preview rate changes the media-time step between presented
        // frames, so the pending measurement no longer describes this cadence.
        self.reset_motion_blur();
        self.metrics.reset_presented();
        cx.notify();
    }

    pub(super) fn playhead_seconds(&self) -> f64 {
        self.timeline.playhead_seconds()
    }

    pub(super) fn duration_seconds(&self) -> f64 {
        self.timeline.duration_seconds()
    }

    fn accept_preview_frame(&mut self, seconds: f64) -> bool {
        let Some(slot) = self.preview_rate.frame_slot(seconds) else {
            return true;
        };
        if self
            .last_preview_slot
            .is_some_and(|last_slot| slot <= last_slot)
        {
            return false;
        }
        self.last_preview_slot = Some(slot);
        true
    }

    fn display_seconds_for_frame(&mut self, decoded_seconds: f64) -> f64 {
        let requested = self.pending_seek_target;
        let display_seconds =
            resolve_frame_seconds(&mut self.pending_seek_target, self.playing, decoded_seconds);
        if let Some(requested) = requested
            && (display_seconds - decoded_seconds).abs() > f64::EPSILON
        {
            tracing::debug!(
                target: "recorder::playback",
                requested_seconds = requested,
                decoded_seconds,
                "holding requested timeline position for pre-target decoded frame"
            );
        }
        display_seconds
    }
}

fn set_transition_points(region: &mut ZoomRegion) {
    let (zoom_in_end_us, zoom_out_start_us) = region.transition_points();
    region.zoom_in_end_us = Some(zoom_in_end_us);
    region.zoom_out_start_us = Some(zoom_out_start_us);
}

fn set_cursor_transition_points(region: &mut CursorSizeRegion) {
    let (ease_in_end_us, ease_out_start_us) = region.transition_points();
    region.ease_in_end_us = Some(ease_in_end_us);
    region.ease_out_start_us = Some(ease_out_start_us);
}

fn resolve_frame_seconds(pending_target: &mut Option<f64>, playing: bool, decoded: f64) -> f64 {
    let Some(target) = *pending_target else {
        return decoded;
    };
    if playing && decoded >= target {
        *pending_target = None;
        decoded
    } else {
        target
    }
}

fn read_background_image(path: &Path) -> Result<(ImageFormat, Vec<u8>)> {
    let bytes = fs::read(path).map_err(|error| {
        anyhow!(
            "Could not read canvas background {}: {error}",
            path.display()
        )
    })?;
    let format = image_format(path, &bytes).ok_or_else(|| {
        anyhow!(
            "Unsupported canvas background format for {}",
            path.display()
        )
    })?;
    Ok((format, bytes))
}

fn image_format(path: &Path, bytes: &[u8]) -> Option<ImageFormat> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    let from_extension = extension.as_deref().and_then(|extension| match extension {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "webp" => Some(ImageFormat::Webp),
        "gif" => Some(ImageFormat::Gif),
        "svg" => Some(ImageFormat::Svg),
        "bmp" => Some(ImageFormat::Bmp),
        "tif" | "tiff" => Some(ImageFormat::Tiff),
        "ico" => Some(ImageFormat::Ico),
        "pbm" | "ppm" | "pgm" | "pnm" => Some(ImageFormat::Pnm),
        _ => None,
    });
    from_extension.or_else(|| image_format_from_bytes(bytes))
}

fn image_format_from_bytes(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF8") {
        Some(ImageFormat::Gif)
    } else if bytes.starts_with(b"BM") {
        Some(ImageFormat::Bmp)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some(ImageFormat::Tiff)
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        Some(ImageFormat::Ico)
    } else if bytes.iter().take(512).any(|byte| *byte == b'<')
        && String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).contains("svg")
    {
        Some(ImageFormat::Svg)
    } else if bytes.starts_with(b"P1")
        || bytes.starts_with(b"P2")
        || bytes.starts_with(b"P3")
        || bytes.starts_with(b"P4")
        || bytes.starts_with(b"P5")
        || bytes.starts_with(b"P6")
    {
        Some(ImageFormat::Pnm)
    } else {
        None
    }
}

fn distance(left: Point<Pixels>, right: Point<Pixels>) -> f32 {
    let dx = left.x.as_f32() - right.x.as_f32();
    let dy = left.y.as_f32() - right.y.as_f32();
    dx.hypot(dy)
}

impl Render for PlaybackView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = self.metrics.clone();
        self.motion_blur.set_scale_factor(window.scale_factor());
        let stage = *self.canvas_bounds.borrow();
        if let Some(stage) = stage {
            let workspace = cx.theme().background;
            super::native_preview::compose(self, window, stage, workspace);
        }
        for image in self.thumbnail_manager.take_image_releases() {
            let _ = window.drop_image(image);
        }
        for image in self.pending_image_releases.drain(..) {
            let release_started = Instant::now();
            let _ = window.drop_image(image);
            metrics.image_released(release_started.elapsed());
        }
        editor_shell::render(self, cx)
    }
}

fn reached_end(current_seconds: f64, duration_seconds: f64) -> bool {
    duration_seconds.is_finite()
        && duration_seconds > 0.0
        && current_seconds.is_finite()
        && current_seconds >= duration_seconds
}

fn should_publish_scrub_seek(last_published_us: Option<u64>, target_us: u64) -> bool {
    last_published_us.is_none_or(|published| published.abs_diff(target_us) >= SCRUB_SEEK_STEP_US)
}

#[cfg(test)]
mod tests {
    use super::{reached_end, resolve_frame_seconds};

    #[test]
    fn detects_replay_at_end() {
        assert!(reached_end(4.0, 4.0));
        assert!(!reached_end(3.99, 4.0));
        assert!(!reached_end(0.0, 0.0));
    }

    #[test]
    fn holds_seek_target_until_playback_catches_up() {
        let mut pending = Some(7.0);
        assert_eq!(resolve_frame_seconds(&mut pending, false, 6.8), 7.0);
        assert_eq!(pending, Some(7.0));
        assert_eq!(resolve_frame_seconds(&mut pending, true, 6.9), 7.0);
        assert_eq!(pending, Some(7.0));
        assert_eq!(resolve_frame_seconds(&mut pending, true, 7.0), 7.0);
        assert_eq!(pending, None);
    }

    #[test]
    fn coalesces_sub_frame_scrub_requests() {
        assert!(!super::should_publish_scrub_seek(
            Some(1_000_000),
            1_010_000
        ));
        assert!(super::should_publish_scrub_seek(Some(1_000_000), 1_016_667));
        assert!(super::should_publish_scrub_seek(None, 1_000));
    }
}
