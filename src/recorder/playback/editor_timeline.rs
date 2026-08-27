use std::{cell::RefCell, rc::Rc};

use gpui::*;
use gpui_component::ActiveTheme as _;

use super::super::thumbnails::{self, TimelineViewport};
use super::super::zoom::{CursorSizeRegion, ZoomRegion};
use super::{PlaybackView, editor_timeline_canvas::TimelineCanvas};

pub(super) const TIMELINE_HEIGHT: f32 = 148.;

pub(super) const LABEL_WIDTH: f32 = 64.;
pub(super) const RULER_HEIGHT: f32 = 32.;
pub(super) const TRACKS_TOP: f32 = 40.;
pub(super) const TRACK_HEIGHT: f32 = 28.;
pub(super) const TRACK_GAP: f32 = 4.;
const DEFAULT_PIXELS_PER_SECOND: f32 = 80.;
const MIN_PIXELS_PER_SECOND: f32 = 12.;
const MAX_PIXELS_PER_SECOND: f32 = 2_000.;
const DEFAULT_RULER_VIEWPORT_WIDTH: f32 = 1_024.;
const MIN_RULER_LABEL_GAP: f32 = 10.;

pub(super) const TRACK_NAMES: [&str; 3] = ["Video", "Cursor", "Zoom"];
pub(super) type TimelineBounds = Rc<RefCell<Option<Bounds<Pixels>>>>;

const ZOOM_EDGE_HIT_PIXELS: f32 = 8.;
const ZOOM_TRANSITION_HIT_PIXELS: f32 = 9.;
const ZOOM_SNAP_PIXELS: f32 = 8.;
const ZOOM_SNAP_MAX_US: u64 = 120_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TimelineInteraction {
    Scrub,
    MoveCursorSize { index: usize, grab_offset_us: u64 },
    ResizeCursorSizeStart { index: usize },
    ResizeCursorSizeEnd { index: usize },
    MoveZoom { index: usize, grab_offset_us: u64 },
    ResizeZoomStart { index: usize },
    ResizeZoomEnd { index: usize },
    ResizeZoomInEnd { index: usize },
    ResizeZoomOutStart { index: usize },
}

impl TimelineInteraction {
    pub(super) fn cursor_style(self) -> CursorStyle {
        match self {
            Self::Scrub => CursorStyle::Arrow,
            Self::MoveCursorSize { .. } => CursorStyle::ClosedHand,
            Self::ResizeCursorSizeStart { .. } => CursorStyle::ResizeLeft,
            Self::ResizeCursorSizeEnd { .. } => CursorStyle::ResizeRight,
            Self::MoveZoom { .. } => CursorStyle::ClosedHand,
            Self::ResizeZoomStart { .. } => CursorStyle::ResizeLeft,
            Self::ResizeZoomEnd { .. } => CursorStyle::ResizeRight,
            Self::ResizeZoomInEnd { .. } | Self::ResizeZoomOutStart { .. } => {
                CursorStyle::ResizeLeftRight
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum CursorSizeHit {
    Body { index: usize },
    Start { index: usize },
    End { index: usize },
}

impl CursorSizeHit {
    pub(super) fn index(self) -> usize {
        match self {
            Self::Body { index } | Self::Start { index } | Self::End { index } => index,
        }
    }

    pub(super) fn cursor_style(self) -> CursorStyle {
        match self {
            Self::Body { .. } => CursorStyle::OpenHand,
            Self::Start { .. } => CursorStyle::ResizeLeft,
            Self::End { .. } => CursorStyle::ResizeRight,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ZoomHit {
    Body { index: usize },
    Start { index: usize },
    ZoomInEnd { index: usize },
    ZoomOutStart { index: usize },
    End { index: usize },
}

impl ZoomHit {
    pub(super) fn index(self) -> usize {
        match self {
            Self::Body { index }
            | Self::Start { index }
            | Self::ZoomInEnd { index }
            | Self::ZoomOutStart { index }
            | Self::End { index } => index,
        }
    }

    pub(super) fn cursor_style(self) -> CursorStyle {
        match self {
            Self::Body { .. } => CursorStyle::OpenHand,
            Self::Start { .. } => CursorStyle::ResizeLeft,
            Self::ZoomInEnd { .. } | Self::ZoomOutStart { .. } => CursorStyle::ResizeLeftRight,
            Self::End { .. } => CursorStyle::ResizeRight,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TimelineState {
    pub(super) playhead_us: u64,
    pub(super) duration_us: u64,
    pub(super) pixels_per_second: f32,
    pub(super) scroll_us: u64,
    scrubbing: bool,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            playhead_us: 0,
            duration_us: 0,
            pixels_per_second: DEFAULT_PIXELS_PER_SECOND,
            scroll_us: 0,
            scrubbing: false,
        }
    }
}

impl TimelineState {
    pub(super) fn playhead_seconds(self) -> f64 {
        micros_to_seconds(self.playhead_us)
    }

    pub(super) fn duration_seconds(self) -> f64 {
        micros_to_seconds(self.duration_us)
    }

    pub(super) fn set_duration_seconds(&mut self, seconds: f64) {
        self.duration_us = seconds_to_micros(seconds);
        self.playhead_us = self.playhead_us.min(self.duration_us);
        self.scroll_us = self.scroll_us.min(self.duration_us);
    }

    pub(super) fn set_playhead_seconds(&mut self, seconds: f64) {
        self.playhead_us = seconds_to_micros(seconds).min(self.duration_us);
    }

    pub(super) fn begin_scrub(
        &mut self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
    ) -> Option<f64> {
        self.scrubbing = true;
        self.seek_at(position, bounds)
    }

    pub(super) fn update_scrub(
        &mut self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
    ) -> Option<f64> {
        self.scrubbing
            .then(|| self.seek_at(position, bounds))
            .flatten()
    }

    pub(super) fn end_scrub(&mut self) {
        self.scrubbing = false;
    }

    pub(super) fn scrubbing(self) -> bool {
        self.scrubbing
    }

    pub(super) fn time_at_position(self, position: Point<Pixels>, bounds: Bounds<Pixels>) -> u64 {
        if self.duration_us == 0 || self.pixels_per_second <= 0.0 {
            return 0;
        }

        let viewport_width = content_width(bounds);
        let x = (position.x.as_f32() - bounds.origin.x.as_f32()).clamp(0.0, viewport_width);
        let seconds = micros_to_seconds(self.effective_scroll_us(bounds))
            + f64::from(x) / f64::from(self.pixels_per_second);
        seconds_to_micros(seconds).min(self.duration_us)
    }

    pub(super) fn time_to_x(self, time_us: u64, bounds: Bounds<Pixels>) -> f32 {
        bounds.origin.x.as_f32()
            + ((micros_to_seconds(time_us) - micros_to_seconds(self.effective_scroll_us(bounds)))
                * f64::from(self.pixels_per_second)) as f32
    }

    pub(super) fn handle_scroll(
        &mut self,
        delta_x: f32,
        delta_y: f32,
        anchor: Point<Pixels>,
        bounds: Bounds<Pixels>,
    ) -> bool {
        let mut changed = false;
        if delta_x.abs() > f32::EPSILON {
            changed |= self.scroll_by_pixels(-delta_x, bounds);
        }
        if delta_y.abs() > f32::EPSILON {
            let factor = if delta_y > 0.0 { 1.12 } else { 1. / 1.12 };
            changed |= self.zoom_at(anchor, factor, bounds);
        }
        changed
    }

    fn seek_at(&mut self, position: Point<Pixels>, bounds: Bounds<Pixels>) -> Option<f64> {
        if self.duration_us == 0 || self.pixels_per_second <= 0.0 {
            self.playhead_us = 0;
            return Some(0.0);
        }

        let time_us = self.time_at_position(position, bounds);
        self.playhead_us = time_us;
        Some(self.playhead_seconds())
    }

    fn scroll_by_pixels(&mut self, pixels: f32, bounds: Bounds<Pixels>) -> bool {
        let max_scroll = self.max_scroll_us(bounds);
        let next = (self.effective_scroll_us(bounds) as f64
            + f64::from(pixels) / f64::from(self.pixels_per_second) * 1_000_000.)
            .clamp(0., max_scroll as f64)
            .round() as u64;
        let changed = next != self.scroll_us;
        self.scroll_us = next;
        changed
    }

    fn zoom_at(&mut self, anchor: Point<Pixels>, factor: f32, bounds: Bounds<Pixels>) -> bool {
        let old_pixels_per_second = self.pixels_per_second;
        let next_pixels_per_second =
            (old_pixels_per_second * factor).clamp(MIN_PIXELS_PER_SECOND, MAX_PIXELS_PER_SECOND);
        if (next_pixels_per_second - old_pixels_per_second).abs() < f32::EPSILON {
            return false;
        }

        let anchor_x =
            (anchor.x.as_f32() - bounds.origin.x.as_f32()).clamp(0., content_width(bounds));
        let anchor_seconds = micros_to_seconds(self.effective_scroll_us(bounds))
            + f64::from(anchor_x) / f64::from(old_pixels_per_second);
        self.pixels_per_second = next_pixels_per_second;
        let next_scroll_seconds =
            anchor_seconds - f64::from(anchor_x) / f64::from(next_pixels_per_second);
        self.scroll_us =
            seconds_to_micros(next_scroll_seconds.max(0.)).min(self.max_scroll_us(bounds));
        true
    }

    pub(super) fn max_scroll_us(self, bounds: Bounds<Pixels>) -> u64 {
        self.max_scroll_us_for_width(content_width(bounds))
    }

    fn max_scroll_us_for_width(self, viewport_width: f32) -> u64 {
        if self.pixels_per_second <= 0.0 || !self.pixels_per_second.is_finite() {
            return 0;
        }
        let visible_seconds = f64::from(viewport_width.max(0.)) / f64::from(self.pixels_per_second);
        self.duration_us
            .saturating_sub(seconds_to_micros(visible_seconds))
    }

    pub(super) fn clamped_to_bounds(self, bounds: Bounds<Pixels>) -> Self {
        Self {
            scroll_us: self.effective_scroll_us(bounds),
            ..self
        }
    }

    pub(super) fn effective_scroll_us(self, bounds: Bounds<Pixels>) -> u64 {
        self.scroll_us.min(self.max_scroll_us(bounds))
    }

    pub(super) fn visible_time_range_us(
        self,
        viewport_width: f32,
        margin_pixels: f32,
    ) -> (u64, u64) {
        let scroll_us = self
            .scroll_us
            .min(self.max_scroll_us_for_width(viewport_width));
        if self.duration_us == 0 || self.pixels_per_second <= 0.0 {
            return (scroll_us, scroll_us);
        }

        let visible_us = seconds_to_micros(
            f64::from(viewport_width.max(0.)) / f64::from(self.pixels_per_second),
        );
        let margin_us =
            seconds_to_micros(f64::from(margin_pixels.max(0.)) / f64::from(self.pixels_per_second));
        (
            scroll_us.saturating_sub(margin_us),
            scroll_us
                .saturating_add(visible_us)
                .saturating_add(margin_us)
                .min(self.duration_us),
        )
    }

    pub(super) fn tick_data(self, viewport_width: f32) -> TickData {
        let major_seconds = major_tick_seconds(self.pixels_per_second);
        let minor_seconds = minor_tick_seconds(major_seconds);
        let visible_seconds = if self.pixels_per_second > 0.0 && self.pixels_per_second.is_finite()
        {
            f64::from(viewport_width.max(0.)) / f64::from(self.pixels_per_second)
        } else {
            0.
        };
        let start = micros_to_seconds(
            self.scroll_us
                .min(self.max_scroll_us_for_width(viewport_width)),
        );
        let end = start + visible_seconds + major_seconds;
        TickData {
            major_seconds,
            minor_seconds,
            start,
            end: end.min(self.duration_seconds()),
        }
    }

    fn labels(self, viewport_width: f32) -> Vec<TickLabel> {
        let viewport_width = viewport_width.max(0.);
        let data = self.tick_data(viewport_width);
        let first = (data.start / data.major_seconds).floor() * data.major_seconds;
        let mut labels = Vec::new();
        let mut previous_end = f32::NEG_INFINITY;
        let mut seconds = first;
        while seconds <= data.end + f64::EPSILON && labels.len() < 128 {
            let x = (seconds - data.start) as f32 * self.pixels_per_second;
            if (0. ..=viewport_width).contains(&x) {
                let text = format_tick_time(seconds);
                if x >= previous_end {
                    previous_end = x + label_width(&text) + MIN_RULER_LABEL_GAP;
                    labels.push(TickLabel { x, text });
                }
            }
            seconds += data.major_seconds;
        }
        labels
    }
}

#[derive(Clone, Debug)]
struct TickLabel {
    x: f32,
    text: String,
}

pub(super) struct TickData {
    pub(super) major_seconds: f64,
    pub(super) minor_seconds: f64,
    pub(super) start: f64,
    pub(super) end: f64,
}

pub(super) fn render(view: &PlaybackView, cx: &mut Context<PlaybackView>) -> impl IntoElement {
    let mut state = view.timeline;
    if let Some(bounds) = *view.timeline_bounds.borrow() {
        state = state.clamped_to_bounds(bounds);
    }
    let bounds_slot = view.timeline_bounds.clone();
    let viewport_width = view
        .timeline_bounds
        .borrow()
        .map_or(DEFAULT_RULER_VIEWPORT_WIDTH, |bounds| {
            bounds.size.width.as_f32()
        });
    let thumbnail_plan = thumbnails::plan(
        TimelineViewport {
            duration_us: state.duration_us,
            scroll_us: state.scroll_us,
            pixels_per_second: state.pixels_per_second,
            width_px: viewport_width,
        },
        thumbnails::thumbnail_size(view.video_width, view.video_height),
    );
    let thumbnail_strip = view.thumbnail_manager.request(&thumbnail_plan);
    let canvas = TimelineCanvas::new(
        state,
        bounds_slot,
        view.project_settings.zoom_regions.clone(),
        view.project_settings.cursor_size_regions.clone(),
        view.selected_zoom_region,
        view.selected_cursor_size_region,
        view.hovered_zoom_hit,
        view.hovered_cursor_size_hit,
        thumbnail_strip,
        cx.theme().background,
        cx.theme().popover,
        cx.theme().border,
        cx.theme().muted_foreground,
        cx.theme().primary,
        view.metrics.clone(),
    )
    .size_full();
    let muted = cx.theme().muted_foreground;
    let scale_color = cx.theme().popover_foreground;
    let labels = state.labels(viewport_width);
    let zoom_scale_labels = render_zoom_scale_labels(
        &state,
        &view.project_settings.zoom_regions,
        view.selected_zoom_region,
        viewport_width,
    );
    let cursor = view.timeline_cursor_style();
    let gutter = div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .w(px(LABEL_WIDTH))
        .bg(cx.theme().popover.opacity(0.35))
        .border_r_1()
        .border_color(cx.theme().border)
        .children(
            TRACK_NAMES
                .into_iter()
                .enumerate()
                .map(|(index, label)| render_label(12., track_y(index) + 6., label, muted)),
        );
    let viewport = div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(LABEL_WIDTH))
        .right_0()
        .overflow_hidden()
        .child(canvas)
        .children(
            labels
                .into_iter()
                .map(|label| render_label(label.x, 8., label.text, muted)),
        )
        .children(zoom_scale_labels.into_iter().map(|label| {
            div()
                .absolute()
                .left(px(label.x))
                .top(px(label.y))
                .w(px(label.width))
                .text_xs()
                .text_color(if label.selected {
                    scale_color
                } else {
                    muted.opacity(0.82)
                })
                .overflow_hidden()
                .child(label.text)
        }));

    div()
        .relative()
        .w_full()
        .h(px(TIMELINE_HEIGHT))
        .flex_shrink_0()
        .overflow_hidden()
        .track_focus(&view.timeline_focus_handle)
        .cursor(cursor)
        .border_t_1()
        .border_color(cx.theme().border)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|view, event: &MouseDownEvent, window, cx| {
                view.timeline_focus_handle.focus(window, cx);
                view.begin_timeline_interaction(event.position, cx);
            }),
        )
        .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| {
            view.update_timeline_hover(event.position, cx);
            view.update_timeline_interaction(event.position, cx);
        }))
        .on_mouse_exit(cx.listener(|view, _, _, cx| view.clear_timeline_hover(cx)))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|view, _, _, cx| view.end_timeline_interaction(cx)),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|view, _, _, cx| view.end_timeline_interaction(cx)),
        )
        .on_scroll_wheel(cx.listener(|view, event, _, cx| {
            view.scroll_timeline(event, cx);
        }))
        .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
            if !event.is_held
                && (event.keystroke.key.eq_ignore_ascii_case("delete")
                    || event.keystroke.key.eq_ignore_ascii_case("backspace"))
            {
                if view.selected_cursor_size_region.is_some() {
                    view.delete_selected_cursor_size_region(cx);
                } else {
                    view.delete_selected_zoom_region(cx);
                }
            }
        }))
        .child(gutter)
        .child(viewport)
}

struct ZoomScaleLabel {
    x: f32,
    y: f32,
    width: f32,
    selected: bool,
    text: String,
}

fn render_zoom_scale_labels(
    state: &TimelineState,
    regions: &[ZoomRegion],
    selected_zoom_region: Option<usize>,
    viewport_width: f32,
) -> Vec<ZoomScaleLabel> {
    const MIN_LABEL_WIDTH: f32 = 38.;
    const MAX_LABEL_WIDTH: f32 = 52.;
    let (visible_start_us, visible_end_us) = state.visible_time_range_us(viewport_width, 0.);
    let scroll_seconds = micros_to_seconds(
        state
            .scroll_us
            .min(state.max_scroll_us_for_width(viewport_width)),
    );
    regions
        .iter()
        .enumerate()
        .filter(|(_, region)| {
            region.end_us >= visible_start_us && region.start_us <= visible_end_us
        })
        .filter_map(|(index, region)| {
            let start_x = (micros_to_seconds(region.start_us) - scroll_seconds) as f32
                * state.pixels_per_second;
            let end_x = (micros_to_seconds(region.end_us) - scroll_seconds) as f32
                * state.pixels_per_second;
            let x = start_x.max(2.);
            let available = end_x - x - 2.;
            (available >= MIN_LABEL_WIDTH).then(|| ZoomScaleLabel {
                x,
                y: track_y(2) + 7.,
                width: available.min(MAX_LABEL_WIDTH),
                selected: selected_zoom_region == Some(index),
                text: format!("{:.1}×", region.scale),
            })
        })
        .collect()
}

pub(super) fn hit_test_zoom_region(
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    state: TimelineState,
    regions: &[ZoomRegion],
) -> Option<ZoomHit> {
    if !in_viewport(position, bounds) {
        return None;
    }
    let zoom_track_top = bounds.origin.y.as_f32() + track_y(2);
    let zoom_track_bottom = zoom_track_top + TRACK_HEIGHT;
    if !(zoom_track_top..=zoom_track_bottom).contains(&position.y.as_f32()) {
        return None;
    }

    let pointer_x = position.x.as_f32();
    let (visible_start_us, visible_end_us) = state.visible_time_range_us(
        content_width(bounds),
        ZOOM_EDGE_HIT_PIXELS.max(ZOOM_TRANSITION_HIT_PIXELS),
    );
    for (index, region) in regions.iter().enumerate().rev() {
        if region.end_us < visible_start_us || region.start_us > visible_end_us {
            continue;
        }
        let start_x = state.time_to_x(region.start_us, bounds);
        let end_x = state.time_to_x(region.end_us, bounds);
        let (zoom_in_end_us, zoom_out_start_us) = region.transition_points();
        let zoom_in_end_x = state.time_to_x(zoom_in_end_us, bounds);
        let zoom_out_start_x = state.time_to_x(zoom_out_start_us, bounds);
        if (pointer_x - start_x).abs() <= ZOOM_EDGE_HIT_PIXELS {
            return Some(ZoomHit::Start { index });
        }
        if (pointer_x - end_x).abs() <= ZOOM_EDGE_HIT_PIXELS {
            return Some(ZoomHit::End { index });
        }
        if (pointer_x - zoom_in_end_x).abs() <= ZOOM_TRANSITION_HIT_PIXELS {
            return Some(ZoomHit::ZoomInEnd { index });
        }
        if (pointer_x - zoom_out_start_x).abs() <= ZOOM_TRANSITION_HIT_PIXELS {
            return Some(ZoomHit::ZoomOutStart { index });
        }
        if pointer_x >= start_x && pointer_x <= end_x {
            return Some(ZoomHit::Body { index });
        }
    }
    None
}

pub(super) fn hit_test_cursor_size_region(
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    state: TimelineState,
    regions: &[CursorSizeRegion],
) -> Option<CursorSizeHit> {
    if !in_viewport(position, bounds) {
        return None;
    }
    let cursor_track_top = bounds.origin.y.as_f32() + track_y(1);
    let cursor_track_bottom = cursor_track_top + TRACK_HEIGHT;
    if !(cursor_track_top..=cursor_track_bottom).contains(&position.y.as_f32()) {
        return None;
    }

    let pointer_x = position.x.as_f32();
    let (visible_start_us, visible_end_us) =
        state.visible_time_range_us(content_width(bounds), ZOOM_EDGE_HIT_PIXELS);
    for (index, region) in regions.iter().enumerate().rev() {
        if region.end_us < visible_start_us || region.start_us > visible_end_us {
            continue;
        }
        let start_x = state.time_to_x(region.start_us, bounds);
        let end_x = state.time_to_x(region.end_us, bounds);
        if (pointer_x - start_x).abs() <= ZOOM_EDGE_HIT_PIXELS {
            return Some(CursorSizeHit::Start { index });
        }
        if (pointer_x - end_x).abs() <= ZOOM_EDGE_HIT_PIXELS {
            return Some(CursorSizeHit::End { index });
        }
        if pointer_x >= start_x && pointer_x <= end_x {
            return Some(CursorSizeHit::Body { index });
        }
    }
    None
}

pub(super) fn snap_time(time_us: u64, state: TimelineState, candidates: &[u64]) -> u64 {
    nearest_snap(time_us, state, candidates).map_or(time_us, |(time, _)| time)
}

pub(super) fn snap_range(
    start_us: u64,
    end_us: u64,
    state: TimelineState,
    candidates: &[u64],
) -> (u64, u64) {
    let start_snap = nearest_snap(start_us, state, candidates);
    let end_snap = nearest_snap(end_us, state, candidates);
    let Some((edge, target)) = (match (start_snap, end_snap) {
        (Some(start), Some(end)) if start.1 <= end.1 => Some((start_us, start.0)),
        (Some(_), Some(end)) => Some((end_us, end.0)),
        (Some(start), None) => Some((start_us, start.0)),
        (None, Some(end)) => Some((end_us, end.0)),
        (None, None) => None,
    }) else {
        return (start_us, end_us);
    };

    let length = end_us.saturating_sub(start_us);
    let next_start = if target >= edge {
        start_us.saturating_add(target - edge)
    } else {
        start_us.saturating_sub(edge - target)
    };
    let next_end = next_start.saturating_add(length);
    (next_start, next_end)
}

fn nearest_snap(time_us: u64, state: TimelineState, candidates: &[u64]) -> Option<(u64, u64)> {
    if state.pixels_per_second <= 0.0 || !state.pixels_per_second.is_finite() {
        return None;
    }
    let threshold = ((f64::from(ZOOM_SNAP_PIXELS) / f64::from(state.pixels_per_second))
        * 1_000_000.)
        .round()
        .min(ZOOM_SNAP_MAX_US as f64) as u64;
    candidates
        .iter()
        .copied()
        .map(|candidate| (candidate, time_us.abs_diff(candidate)))
        .filter(|(_, distance)| *distance <= threshold)
        .min_by_key(|(_, distance)| *distance)
}

fn render_label(x: f32, y: f32, text: impl Into<SharedString>, color: Hsla) -> impl IntoElement {
    div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .text_xs()
        .text_color(color)
        .child(text.into())
}

fn label_width(text: &str) -> f32 {
    text.chars().count() as f32 * 7. + 2.
}

pub(super) fn track_y(index: usize) -> f32 {
    TRACKS_TOP + index as f32 * (TRACK_HEIGHT + TRACK_GAP)
}

pub(super) fn content_width(bounds: Bounds<Pixels>) -> f32 {
    bounds.size.width.as_f32().max(0.)
}

pub(super) fn in_viewport(position: Point<Pixels>, bounds: Bounds<Pixels>) -> bool {
    let x = position.x.as_f32();
    let left = bounds.origin.x.as_f32();
    x >= left && x < left + bounds.size.width.as_f32()
}

pub(super) fn seconds_to_micros(seconds: f64) -> u64 {
    if !seconds.is_finite() || seconds <= 0. {
        return 0;
    }
    let micros = seconds * 1_000_000.;
    if micros >= u64::MAX as f64 {
        u64::MAX
    } else {
        micros.round() as u64
    }
}

pub(super) fn micros_to_seconds(micros: u64) -> f64 {
    micros as f64 / 1_000_000.
}

fn major_tick_seconds(pixels_per_second: f32) -> f64 {
    const STEPS: [f64; 15] = [
        0.1, 0.2, 0.5, 1., 2., 5., 10., 15., 30., 60., 120., 300., 600., 1_800., 3_600.,
    ];
    STEPS
        .into_iter()
        .find(|step| *step * f64::from(pixels_per_second) >= 88.)
        .unwrap_or(3_600.)
}

fn minor_tick_seconds(major_seconds: f64) -> f64 {
    if major_seconds >= 30. {
        major_seconds / 3.
    } else if major_seconds >= 5. {
        major_seconds / 5.
    } else if major_seconds >= 1. {
        major_seconds / 2.
    } else {
        major_seconds / 5.
    }
}

fn format_tick_time(seconds: f64) -> String {
    let total_milliseconds = (seconds.max(0.) * 1_000.).round() as u64;
    let total_seconds = total_milliseconds / 1_000;
    let milliseconds = total_milliseconds % 1_000;
    let seconds_part = total_seconds % 60;
    let minutes = total_seconds / 60;
    let base = if minutes >= 60 {
        format!(
            "{:02}:{:02}:{:02}",
            minutes / 60,
            minutes % 60,
            seconds_part
        )
    } else {
        format!("{:02}:{:02}", minutes, seconds_part)
    };
    if milliseconds == 0 {
        return base;
    }

    if milliseconds.is_multiple_of(100) {
        format!("{base}.{}", milliseconds / 100)
    } else if milliseconds.is_multiple_of(10) {
        format!("{base}.{:02}", milliseconds / 10)
    } else {
        format!("{base}.{:03}", milliseconds)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TimelineState, format_tick_time, hit_test_zoom_region, label_width, major_tick_seconds,
        minor_tick_seconds, seconds_to_micros, snap_range, snap_time, track_y,
    };
    use crate::recorder::zoom::ZoomRegion;

    #[test]
    fn clamps_playhead_to_duration() {
        let mut state = TimelineState::default();
        state.set_duration_seconds(5.);
        state.set_playhead_seconds(7.);
        assert_eq!(state.playhead_us, 5_000_000);
    }

    #[test]
    fn preserves_zoom_anchor() {
        let mut state = TimelineState::default();
        state.set_duration_seconds(30.);
        let bounds = gpui::Bounds::new(
            gpui::point(gpui::px(200.), gpui::px(0.)),
            gpui::size(gpui::px(800.), gpui::px(100.)),
        );
        let anchor = gpui::point(gpui::px(500.), gpui::px(20.));
        let before = state.time_at_position(anchor, bounds);
        state.zoom_at(anchor, 2., bounds);
        let after = state.time_at_position(anchor, bounds);
        assert!(before.abs_diff(after) <= 2);
    }

    #[test]
    fn aligns_zero_with_the_viewport_origin() {
        let mut state = TimelineState::default();
        state.set_duration_seconds(30.);
        let bounds = gpui::Bounds::new(
            gpui::point(gpui::px(200.), gpui::px(0.)),
            gpui::size(gpui::px(800.), gpui::px(100.)),
        );

        assert_eq!(
            state.time_at_position(gpui::point(gpui::px(200.), gpui::px(20.)), bounds),
            0
        );
        assert_eq!(state.time_to_x(0, bounds), 200.);
    }

    #[test]
    fn keeps_repeated_zoom_anchored_at_different_positions() {
        let mut state = TimelineState::default();
        state.set_duration_seconds(120.);
        let bounds = gpui::Bounds::new(
            gpui::point(gpui::px(100.), gpui::px(0.)),
            gpui::size(gpui::px(800.), gpui::px(100.)),
        );

        for (x, factor) in [(250., 2.), (600., 2.), (360., 0.5), (450., 1.5)] {
            let anchor = gpui::point(gpui::px(x), gpui::px(20.));
            let before = state.time_at_position(anchor, bounds);
            state.zoom_at(anchor, factor, bounds);
            let after = state.time_at_position(anchor, bounds);
            assert!(before.abs_diff(after) <= 5);
        }
    }

    #[test]
    fn uses_clamped_scroll_for_pointer_mapping() {
        let mut state = TimelineState::default();
        state.set_duration_seconds(30.);
        state.scroll_us = 30_000_000;
        let bounds = gpui::Bounds::new(
            gpui::point(gpui::px(200.), gpui::px(0.)),
            gpui::size(gpui::px(800.), gpui::px(100.)),
        );

        let clamped = state.clamped_to_bounds(bounds);
        assert!(clamped.scroll_us < state.scroll_us);
        assert_eq!(
            state.time_at_position(gpui::point(gpui::px(200.), gpui::px(20.)), bounds),
            clamped.scroll_us
        );
    }

    #[test]
    fn ruler_labels_stay_inside_the_viewport() {
        let mut state = TimelineState::default();
        state.set_duration_seconds(30.);
        state.scroll_us = 5_000_000;

        let labels = state.labels(400.);
        assert!(!labels.is_empty());
        assert!(labels.iter().all(|label| (0. ..=400.).contains(&label.x)));
        assert_eq!(labels[0].text, "00:06");
    }

    #[test]
    fn ruler_labels_do_not_overlap() {
        let mut state = TimelineState::default();
        state.set_duration_seconds(10.);
        state.pixels_per_second = 2_000.;

        let labels = state.labels(1_000.);
        for pair in labels.windows(2) {
            let previous_end = pair[0].x + label_width(&pair[0].text);
            assert!(pair[1].x >= previous_end);
        }
    }

    #[test]
    fn visible_range_tracks_scroll_and_margin() {
        let mut state = TimelineState::default();
        state.set_duration_seconds(120.);
        state.scroll_us = 30_000_000;

        assert_eq!(
            state.visible_time_range_us(800., 0.),
            (30_000_000, 40_000_000)
        );
        assert_eq!(
            state.visible_time_range_us(800., 80.),
            (29_000_000, 41_000_000)
        );
    }

    #[test]
    fn hit_testing_skips_regions_outside_visible_range() {
        let mut state = TimelineState::default();
        state.set_duration_seconds(120.);
        state.scroll_us = 30_000_000;
        let bounds = gpui::Bounds::new(
            gpui::point(gpui::px(0.), gpui::px(0.)),
            gpui::size(gpui::px(800.), gpui::px(100.)),
        );
        let regions = vec![
            ZoomRegion::new_at(0, state.duration_us).unwrap(),
            ZoomRegion::new_at(35_000_000, state.duration_us).unwrap(),
        ];

        assert!(
            hit_test_zoom_region(
                gpui::point(gpui::px(20.), gpui::px(track_y(2) + 8.)),
                bounds,
                state,
                &regions,
            )
            .is_none()
        );
        assert!(
            hit_test_zoom_region(
                gpui::point(gpui::px(440.), gpui::px(track_y(2) + 8.)),
                bounds,
                state,
                &regions,
            )
            .is_some()
        );
    }

    #[test]
    fn chooses_readable_ruler_steps() {
        assert_eq!(major_tick_seconds(80.), 2.);
        assert_eq!(major_tick_seconds(12.), 10.);
        assert_eq!(major_tick_seconds(2_000.), 0.1);
        assert_eq!(minor_tick_seconds(5.), 1.);
        assert_eq!(minor_tick_seconds(30.), 10.);
    }

    #[test]
    fn formats_ruler_labels() {
        assert_eq!(format_tick_time(0.), "00:00");
        assert_eq!(format_tick_time(0.5), "00:00.5");
        assert_eq!(format_tick_time(65.), "01:05");
        assert_eq!(format_tick_time(3661.), "01:01:01");
        assert_eq!(seconds_to_micros(f64::NAN), 0);
    }

    #[test]
    fn snaps_near_timeline_points() {
        let state = TimelineState::default();
        assert_eq!(snap_time(1_050_000, state, &[1_000_000]), 1_000_000);
        assert_eq!(snap_time(1_101_000, state, &[1_000_000]), 1_101_000);
    }

    #[test]
    fn moves_a_range_without_changing_its_duration() {
        let state = TimelineState::default();
        assert_eq!(
            snap_range(1_050_000, 2_050_000, state, &[1_000_000]),
            (1_000_000, 2_000_000)
        );
    }
}
