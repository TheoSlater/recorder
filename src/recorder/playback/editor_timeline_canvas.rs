use gpui::*;

use super::super::media::PlaybackMetrics;
use super::super::thumbnails::ThumbnailStrip;
use super::super::zoom::{CursorSizeRegion, ZoomRegion};
use super::editor_timeline::{
    RULER_HEIGHT, TRACK_HEIGHT, TRACK_NAMES, TimelineBounds, TimelineState, ZoomHit, content_width,
    micros_to_seconds, track_y,
};

const REGION_CULL_MARGIN_PIXELS: f32 = 12.;

pub(super) struct TimelineCanvas {
    interactivity: Interactivity,
    state: TimelineState,
    bounds_slot: TimelineBounds,
    zoom_regions: Vec<ZoomRegion>,
    cursor_size_regions: Vec<CursorSizeRegion>,
    selected_zoom_region: Option<usize>,
    selected_cursor_size_region: Option<usize>,
    hovered_zoom_hit: Option<ZoomHit>,
    hovered_cursor_size_hit: Option<super::editor_timeline::CursorSizeHit>,
    thumbnail_strip: ThumbnailStrip,
    background: Hsla,
    ruler_background: Hsla,
    border: Hsla,
    muted: Hsla,
    primary: Hsla,
    metrics: PlaybackMetrics,
}

impl TimelineCanvas {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        state: TimelineState,
        bounds_slot: TimelineBounds,
        zoom_regions: Vec<ZoomRegion>,
        cursor_size_regions: Vec<CursorSizeRegion>,
        selected_zoom_region: Option<usize>,
        selected_cursor_size_region: Option<usize>,
        hovered_zoom_hit: Option<ZoomHit>,
        hovered_cursor_size_hit: Option<super::editor_timeline::CursorSizeHit>,
        thumbnail_strip: ThumbnailStrip,
        background: Hsla,
        ruler_background: Hsla,
        border: Hsla,
        muted: Hsla,
        primary: Hsla,
        metrics: PlaybackMetrics,
    ) -> Self {
        Self {
            interactivity: Interactivity::new(),
            state,
            bounds_slot,
            zoom_regions,
            cursor_size_regions,
            selected_zoom_region,
            selected_cursor_size_region,
            hovered_zoom_hit,
            hovered_cursor_size_hit,
            thumbnail_strip,
            background,
            ruler_background,
            border,
            muted,
            primary,
            metrics,
        }
    }
}

impl Element for TimelineCanvas {
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
        let state = self.state;
        let bounds_slot = &self.bounds_slot;
        let zoom_regions = &self.zoom_regions;
        let cursor_size_regions = &self.cursor_size_regions;
        let selected_zoom_region = self.selected_zoom_region;
        let selected_cursor_size_region = self.selected_cursor_size_region;
        let hovered_zoom_hit = self.hovered_zoom_hit;
        let hovered_cursor_size_hit = self.hovered_cursor_size_hit;
        let thumbnail_strip = &self.thumbnail_strip;
        let background = self.background;
        let ruler_background = self.ruler_background;
        let border = self.border;
        let muted = self.muted;
        let primary = self.primary;
        let metrics = &self.metrics;
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |_, window, _| {
                *bounds_slot.borrow_mut() = Some(bounds);
                window.paint_layer(bounds, |window| {
                    paint_timeline(
                        window,
                        bounds,
                        state,
                        zoom_regions,
                        cursor_size_regions,
                        selected_zoom_region,
                        selected_cursor_size_region,
                        hovered_zoom_hit,
                        hovered_cursor_size_hit,
                        thumbnail_strip,
                        background,
                        ruler_background,
                        border,
                        muted,
                        primary,
                        metrics,
                    );
                });
            },
        );
    }
}

impl IntoElement for TimelineCanvas {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for TimelineCanvas {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for TimelineCanvas {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_timeline(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    state: TimelineState,
    zoom_regions: &[ZoomRegion],
    cursor_size_regions: &[CursorSizeRegion],
    selected_zoom_region: Option<usize>,
    selected_cursor_size_region: Option<usize>,
    hovered_zoom_hit: Option<ZoomHit>,
    hovered_cursor_size_hit: Option<super::editor_timeline::CursorSizeHit>,
    thumbnail_strip: &ThumbnailStrip,
    background: Hsla,
    ruler_background: Hsla,
    border: Hsla,
    muted: Hsla,
    primary: Hsla,
    metrics: &PlaybackMetrics,
) {
    let paint_started_at = std::time::Instant::now();
    window.paint_quad(fill(bounds, background));
    let ruler = Bounds::new(bounds.origin, size(bounds.size.width, px(RULER_HEIGHT)));
    window.paint_quad(fill(ruler, ruler_background));

    let content = bounds;
    if content_width(content) <= 0. {
        metrics.timeline_painted(paint_started_at.elapsed());
        return;
    }

    let scroll_us = state.effective_scroll_us(bounds);
    window.with_content_mask(Some(ContentMask { bounds: content }), |window| {
        let ticks = state.tick_data(content.size.width.as_f32());
        let first_minor = (ticks.start / ticks.minor_seconds).floor() * ticks.minor_seconds;
        let mut seconds = first_minor;
        let mut tick_count = 0;
        while seconds <= ticks.end + ticks.minor_seconds && tick_count < 512 {
            let x = content.origin.x.as_f32()
                + (seconds - micros_to_seconds(scroll_us)) as f32 * state.pixels_per_second;
            let is_major =
                ((seconds / ticks.major_seconds).round() - seconds / ticks.major_seconds).abs()
                    < 0.0001;
            let height = if is_major { 14. } else { 8. };
            let color = if is_major {
                border.opacity(0.85)
            } else {
                muted.opacity(0.35)
            };
            if x >= content.origin.x.as_f32()
                && x <= content.origin.x.as_f32() + content.size.width.as_f32()
            {
                window.paint_quad(fill(
                    Bounds::new(point(px(x), bounds.origin.y), size(px(1.), px(height))),
                    color,
                ));
            }
            seconds += ticks.minor_seconds;
            tick_count += 1;
        }

        for index in 0..TRACK_NAMES.len() {
            let track = Bounds::new(
                point(
                    content.origin.x,
                    px(bounds.origin.y.as_f32() + track_y(index)),
                ),
                size(content.size.width, px(TRACK_HEIGHT)),
            );
            let track_color = if index == 0 {
                primary.opacity(0.12)
            } else {
                muted.opacity(0.08)
            };
            window.paint_quad(quad(
                track,
                px(4.),
                track_color,
                px(1.),
                border.opacity(0.55),
                BorderStyle::Solid,
            ));
        }

        let video_start = content.origin.x.as_f32()
            + (0. - micros_to_seconds(scroll_us)) as f32 * state.pixels_per_second;
        let video_end = content.origin.x.as_f32()
            + (state.duration_seconds() - micros_to_seconds(scroll_us)) as f32
                * state.pixels_per_second;
        window.paint_quad(fill(
            Bounds::new(
                point(
                    px(video_start),
                    px(bounds.origin.y.as_f32() + track_y(0) + 3.),
                ),
                size(px((video_end - video_start).max(0.)), px(TRACK_HEIGHT - 6.)),
            ),
            primary.opacity(0.32),
        ));
        super::editor_timeline_thumbnails::paint(
            window,
            content,
            state,
            thumbnail_strip,
            border,
            background,
        );
        for index in 1..TRACK_NAMES.len() {
            let y = bounds.origin.y.as_f32() + track_y(index) + TRACK_HEIGHT / 2. - 1.;
            window.paint_quad(fill(
                Bounds::new(
                    point(px(content.origin.x.as_f32()), px(y)),
                    size(content.size.width, px(2.)),
                ),
                muted.opacity(0.4),
            ));
        }

        let (visible_start_us, visible_end_us) =
            state.visible_time_range_us(content.size.width.as_f32(), REGION_CULL_MARGIN_PIXELS);
        for (index, region) in cursor_size_regions.iter().enumerate() {
            if region.end_us < visible_start_us || region.start_us > visible_end_us {
                continue;
            }
            let start_x = content.origin.x.as_f32()
                + (micros_to_seconds(region.start_us) - micros_to_seconds(scroll_us)) as f32
                    * state.pixels_per_second;
            let end_x = content.origin.x.as_f32()
                + (micros_to_seconds(region.end_us) - micros_to_seconds(scroll_us)) as f32
                    * state.pixels_per_second;
            let y = bounds.origin.y.as_f32() + track_y(1) + 3.;
            let height = TRACK_HEIGHT - 6.;
            let region_bounds = Bounds::new(
                point(px(start_x), px(y)),
                size(px((end_x - start_x).max(0.)), px(height)),
            );
            let selected = selected_cursor_size_region == Some(index);
            let (ease_in_end_us, ease_out_start_us) = region.transition_points();
            let ease_in_end_x = content.origin.x.as_f32()
                + (micros_to_seconds(ease_in_end_us) - micros_to_seconds(scroll_us)) as f32
                    * state.pixels_per_second;
            let ease_out_start_x = content.origin.x.as_f32()
                + (micros_to_seconds(ease_out_start_us) - micros_to_seconds(scroll_us)) as f32
                    * state.pixels_per_second;
            let ramp_color = primary.opacity(if selected { 0.34 } else { 0.13 });
            let hold_color = primary.opacity(if selected { 0.56 } else { 0.25 });
            paint_segment(window, start_x, ease_in_end_x, y, height, ramp_color);
            paint_segment(
                window,
                ease_in_end_x,
                ease_out_start_x,
                y,
                height,
                hold_color,
            );
            paint_segment(window, ease_out_start_x, end_x, y, height, ramp_color);
            if selected {
                window.paint_quad(outline(region_bounds, primary, BorderStyle::Solid));
            }

            let keyframe_color = primary.opacity(if selected { 1.0 } else { 0.78 });
            paint_keyframe(window, start_x, y + height / 2., keyframe_color);
            paint_keyframe(window, ease_in_end_x, y + height / 2., keyframe_color);
            paint_keyframe(window, ease_out_start_x, y + height / 2., keyframe_color);
            paint_keyframe(window, end_x, y + height / 2., keyframe_color);
            let show_handles =
                selected || hovered_cursor_size_hit.is_some_and(|hit| hit.index() == index);
            if show_handles {
                let handle_color = primary.opacity(0.9);
                paint_edge_handle(window, start_x, y, height, handle_color);
                paint_edge_handle(window, end_x, y, height, handle_color);
            }
        }

        for (index, region) in zoom_regions.iter().enumerate() {
            if region.end_us < visible_start_us || region.start_us > visible_end_us {
                continue;
            }
            let start_x = content.origin.x.as_f32()
                + (micros_to_seconds(region.start_us) - micros_to_seconds(scroll_us)) as f32
                    * state.pixels_per_second;
            let end_x = content.origin.x.as_f32()
                + (micros_to_seconds(region.end_us) - micros_to_seconds(scroll_us)) as f32
                    * state.pixels_per_second;
            let y = bounds.origin.y.as_f32() + track_y(2) + 3.;
            let height = TRACK_HEIGHT - 6.;
            let region_bounds = Bounds::new(
                point(px(start_x), px(y)),
                size(px((end_x - start_x).max(0.)), px(height)),
            );
            let selected = selected_zoom_region == Some(index);
            let (zoom_in_end_us, zoom_out_start_us) = region.transition_points();
            let zoom_in_end_x = content.origin.x.as_f32()
                + (micros_to_seconds(zoom_in_end_us) - micros_to_seconds(scroll_us)) as f32
                    * state.pixels_per_second;
            let zoom_out_start_x = content.origin.x.as_f32()
                + (micros_to_seconds(zoom_out_start_us) - micros_to_seconds(scroll_us)) as f32
                    * state.pixels_per_second;
            let ramp_color = primary.opacity(if selected { 0.34 } else { 0.13 });
            let hold_color = primary.opacity(if selected { 0.56 } else { 0.25 });
            paint_segment(window, start_x, zoom_in_end_x, y, height, ramp_color);
            paint_segment(
                window,
                zoom_in_end_x,
                zoom_out_start_x,
                y,
                height,
                hold_color,
            );
            paint_segment(window, zoom_out_start_x, end_x, y, height, ramp_color);
            if selected {
                window.paint_quad(outline(region_bounds, primary, BorderStyle::Solid));
            }
            let show_handles = selected || hovered_zoom_hit.is_some_and(|hit| hit.index() == index);
            if show_handles {
                let handle_color = primary.opacity(0.9);
                paint_edge_handle(window, start_x, y, height, handle_color);
                paint_edge_handle(window, end_x, y, height, handle_color);
                paint_zoom_transition_handle(window, zoom_in_end_x, y, height, handle_color);
                paint_zoom_transition_handle(window, zoom_out_start_x, y, height, handle_color);
            }
        }

        let playhead_x = content.origin.x.as_f32()
            + (state.playhead_seconds() - micros_to_seconds(scroll_us)) as f32
                * state.pixels_per_second;
        window.paint_quad(fill(
            Bounds::new(
                point(px(playhead_x - 1.), bounds.origin.y),
                size(px(2.), bounds.size.height),
            ),
            primary,
        ));
        window.paint_quad(fill(
            Bounds::new(
                point(px(playhead_x - 5.), bounds.origin.y),
                size(px(10.), px(8.)),
            ),
            primary,
        ));
    });

    window.paint_quad(fill(
        Bounds::new(
            point(
                bounds.origin.x,
                px(bounds.origin.y.as_f32() + RULER_HEIGHT - 1.),
            ),
            size(bounds.size.width, px(1.)),
        ),
        border,
    ));
    metrics.timeline_painted(paint_started_at.elapsed());
}

fn paint_segment(window: &mut Window, start_x: f32, end_x: f32, y: f32, height: f32, color: Hsla) {
    if end_x <= start_x {
        return;
    }
    window.paint_quad(fill(
        Bounds::new(
            point(px(start_x), px(y)),
            size(px(end_x - start_x), px(height)),
        ),
        color,
    ));
}

fn paint_keyframe(window: &mut Window, x: f32, center_y: f32, color: Hsla) {
    let half = 4.;
    let mut builder = PathBuilder::fill();
    builder.add_polygon(
        &[
            point(px(x), px(center_y - half)),
            point(px(x + half), px(center_y)),
            point(px(x), px(center_y + half)),
            point(px(x - half), px(center_y)),
        ],
        true,
    );
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn paint_edge_handle(window: &mut Window, x: f32, y: f32, height: f32, color: Hsla) {
    window.paint_quad(fill(
        Bounds::new(
            point(px(x - 1.), px(y + 3.)),
            size(px(2.), px((height - 6.).max(2.))),
        ),
        color,
    ));
}

fn paint_zoom_transition_handle(window: &mut Window, x: f32, y: f32, height: f32, color: Hsla) {
    let diameter = 6.;
    window.paint_quad(quad(
        Bounds::new(
            point(px(x - diameter / 2.), px(y + height / 2. - diameter / 2.)),
            size(px(diameter), px(diameter)),
        ),
        px(diameter / 2.),
        color,
        px(0.),
        color,
        BorderStyle::Solid,
    ));
}
