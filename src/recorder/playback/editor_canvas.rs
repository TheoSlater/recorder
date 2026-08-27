use std::{cell::RefCell, rc::Rc, sync::Arc};

use gpui::*;

use super::super::{
    cursor::CursorFrame,
    media::{FrameTiming, PlaybackMetrics},
    project_settings::{CanvasComposition, CanvasView},
    zoom::ZoomEffect,
};
use super::editor_canvas_cursor_blur::BlurredCursor;

pub(super) type CanvasBounds = Rc<RefCell<Option<Bounds<Pixels>>>>;

pub(super) struct Canvas {
    interactivity: Interactivity,
    image: Option<Arc<RenderImage>>,
    video_width: u32,
    video_height: u32,
    cursor: Option<CursorFrame>,
    cursor_images: [Arc<RenderImage>; 2],
    blurred_cursor: Option<BlurredCursor>,
    canvas_view: CanvasView,
    composition: CanvasComposition,
    background_image: Option<Arc<RenderImage>>,
    zoom_effect: Option<ZoomEffect>,
    stage_background: Hsla,
    canvas_background: Hsla,
    border: Hsla,
    selection: Hsla,
    shadow: Hsla,
    selected_recording: bool,
    bounds_slot: CanvasBounds,
    metrics: PlaybackMetrics,
    frame_timing: Option<FrameTiming>,
    frame_invalidated_at: Option<std::time::Instant>,
    playing: bool,
    /// True when this editor's native compositor owns the composition layers,
    /// so this element paints only chrome over them.
    composed_natively: bool,
}

impl Canvas {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        image: Option<Arc<RenderImage>>,
        video_width: u32,
        video_height: u32,
        cursor: Option<CursorFrame>,
        cursor_images: [Arc<RenderImage>; 2],
        blurred_cursor: Option<BlurredCursor>,
        canvas_view: CanvasView,
        composition: CanvasComposition,
        background_image: Option<Arc<RenderImage>>,
        zoom_effect: Option<ZoomEffect>,
        stage_background: Hsla,
        canvas_background: Hsla,
        border: Hsla,
        selection: Hsla,
        shadow: Hsla,
        selected_recording: bool,
        bounds_slot: CanvasBounds,
        metrics: PlaybackMetrics,
        frame_timing: Option<FrameTiming>,
        frame_invalidated_at: Option<std::time::Instant>,
        playing: bool,
        composed_natively: bool,
    ) -> Self {
        Self {
            interactivity: Interactivity::new(),
            image,
            video_width,
            video_height,
            cursor,
            cursor_images,
            blurred_cursor,
            canvas_view,
            composition,
            background_image,
            zoom_effect,
            stage_background,
            canvas_background,
            border,
            selection,
            shadow,
            selected_recording,
            bounds_slot,
            metrics,
            frame_timing,
            frame_invalidated_at,
            playing,
            composed_natively,
        }
    }
}

impl Element for Canvas {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, cx| window.request_layout(style, None, cx),
        );
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |_, _, hitbox, _, _| hitbox,
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let image = self.image.clone();
        let cursor = self.cursor;
        let cursor_images = self.cursor_images.clone();
        let blurred_cursor = self.blurred_cursor.clone();
        let canvas_view = self.canvas_view;
        let composition = self.composition.clone();
        let background_image = self.background_image.clone();
        let zoom_effect = self.zoom_effect;
        let video_width = self.video_width;
        let video_height = self.video_height;
        let stage_background = self.stage_background;
        let canvas_background = self.canvas_background;
        let border = self.border;
        let selection = self.selection;
        let shadow = self.shadow;
        let selected_recording = self.selected_recording;
        let bounds_slot = self.bounds_slot.clone();
        let metrics = self.metrics.clone();
        let frame_timing = self.frame_timing.clone();
        let frame_invalidated_at = self.frame_invalidated_at;
        let playing = self.playing;
        let composed_natively = self.composed_natively;

        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            move |_, window, _cx| {
                *bounds_slot.borrow_mut() = Some(bounds);
                window.paint_layer(bounds, |window| {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        super::editor_canvas_paint::paint_preview(
                            window,
                            bounds,
                            image,
                            video_width,
                            video_height,
                            cursor,
                            cursor_images,
                            blurred_cursor,
                            canvas_view,
                            composition,
                            background_image,
                            zoom_effect,
                            stage_background,
                            canvas_background,
                            border,
                            selection,
                            shadow,
                            selected_recording,
                            metrics,
                            frame_timing,
                            frame_invalidated_at,
                            playing,
                            composed_natively,
                        );
                    });
                });
            },
        )
    }
}

impl IntoElement for Canvas {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for Canvas {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for Canvas {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}
