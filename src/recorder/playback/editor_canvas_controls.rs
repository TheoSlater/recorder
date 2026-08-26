use gpui::*;
use gpui_component::{
    ActiveTheme as _, Colorize as _, IndexPath, Sizable, Size,
    button::Button,
    color_picker::{ColorPicker, ColorPickerState},
    h_flex,
    select::{Select, SelectState},
    slider::{Slider, SliderState},
    switch::Switch,
    v_flex,
};

use super::super::project_settings::{
    ASPECT_RATIO_LABELS, BACKGROUND_KIND_LABELS, CanvasBackgroundKind, CanvasComposition,
    MAX_COMPOSITION_PADDING, MAX_COMPOSITION_RADIUS, MAX_COMPOSITION_SCALE, MIN_COMPOSITION_SCALE,
};
use super::PlaybackView;

pub(super) struct CanvasControls {
    pub(super) aspect_ratio: Entity<SelectState<Vec<&'static str>>>,
    pub(super) background_kind: Entity<SelectState<Vec<&'static str>>>,
    pub(super) padding: Entity<SliderState>,
    pub(super) scale: Entity<SliderState>,
    pub(super) corner_radius: Entity<SliderState>,
    pub(super) solid_color: Entity<ColorPickerState>,
    pub(super) gradient_start: Entity<ColorPickerState>,
    pub(super) gradient_end: Entity<ColorPickerState>,
}

impl CanvasControls {
    pub(super) fn new(composition: &CanvasComposition, window: &mut Window, cx: &mut App) -> Self {
        let background = &composition.background;
        let solid_color = setting_color(background.solid_color.as_deref(), cx.theme().popover);
        let gradient_start =
            setting_color(background.gradient_start.as_deref(), cx.theme().primary);
        let gradient_end = setting_color(background.gradient_end.as_deref(), cx.theme().background);

        Self {
            aspect_ratio: select_state(
                ASPECT_RATIO_LABELS.to_vec(),
                composition.aspect_ratio.index(),
                window,
                cx,
            ),
            background_kind: select_state(
                BACKGROUND_KIND_LABELS.to_vec(),
                background.kind.index(),
                window,
                cx,
            ),
            padding: cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(MAX_COMPOSITION_PADDING as f32)
                    .step(0.01)
                    .default_value(composition.padding as f32)
            }),
            scale: cx.new(|_| {
                SliderState::new()
                    .min(MIN_COMPOSITION_SCALE as f32)
                    .max(MAX_COMPOSITION_SCALE as f32)
                    .step(0.05)
                    .default_value(composition.scale as f32)
            }),
            corner_radius: cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(MAX_COMPOSITION_RADIUS as f32)
                    .step(0.01)
                    .default_value(composition.corner_radius as f32)
            }),
            solid_color: color_state(solid_color, window, cx),
            gradient_start: color_state(gradient_start, window, cx),
            gradient_end: color_state(gradient_end, window, cx),
        }
    }
}

pub(super) fn render(view: &PlaybackView, cx: &mut Context<PlaybackView>) -> AnyElement {
    let composition = &view.project_settings.canvas_composition;
    let background = &composition.background;
    let muted = cx.theme().muted_foreground;

    let aspect = Select::new(&view.canvas_controls.aspect_ratio)
        .with_size(Size::Small)
        .w(px(116.));
    let background_kind = Select::new(&view.canvas_controls.background_kind)
        .with_size(Size::Small)
        .w(px(116.));
    let padding = Slider::new(&view.canvas_controls.padding)
        .flex_1()
        .min_w(px(64.))
        .h(px(22.));
    let scale = Slider::new(&view.canvas_controls.scale)
        .flex_1()
        .min_w(px(64.))
        .h(px(22.));
    let corner_radius = Slider::new(&view.canvas_controls.corner_radius)
        .flex_1()
        .min_w(px(64.))
        .h(px(22.));
    let shadow = Switch::new("canvas-shadow")
        .checked(composition.shadow)
        .with_size(Size::Small)
        .on_click(cx.listener(|view, checked, _, cx| {
            view.set_canvas_shadow(*checked, cx);
        }));

    let color_controls = match background.kind {
        CanvasBackgroundKind::Solid => h_flex()
            .items_center()
            .justify_between()
            .child(label("Colour"))
            .child(ColorPicker::new(&view.canvas_controls.solid_color).with_size(Size::Small))
            .into_any_element(),
        CanvasBackgroundKind::Gradient => v_flex()
            .gap_1()
            .child(color_row(
                "Start",
                ColorPicker::new(&view.canvas_controls.gradient_start).with_size(Size::Small),
            ))
            .child(color_row(
                "End",
                ColorPicker::new(&view.canvas_controls.gradient_end).with_size(Size::Small),
            ))
            .into_any_element(),
        CanvasBackgroundKind::Image => {
            let image_name = background
                .image_path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .map_or_else(|| "Choose image".to_string(), ToString::to_string);
            Button::new("choose-background-image")
                .outline()
                .compact()
                .with_size(Size::Small)
                .label(image_name)
                .tooltip("Choose a canvas background image")
                .on_click(cx.listener(|view, _, window, cx| {
                    view.choose_background_image(window, cx);
                }))
                .into_any_element()
        }
    };

    v_flex()
        .gap_2()
        .p_2()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(section_title("Canvas"))
        .child(row("Aspect ratio", aspect))
        .child(slider_row(
            "Padding",
            padding,
            format!("{}%", (composition.padding * 100.0).round() as u32),
            muted,
        ))
        .child(slider_row(
            "Scale",
            scale,
            format!("{:.2}×", composition.scale),
            muted,
        ))
        .child(slider_row(
            "Radius",
            corner_radius,
            format!("{}%", (composition.corner_radius * 100.0).round() as u32),
            muted,
        ))
        .child(row("Shadow", shadow))
        .child(row("Background", background_kind))
        .child(color_controls)
        .into_any_element()
}

pub(super) fn setting_color(value: Option<&str>, fallback: Hsla) -> Hsla {
    value
        .and_then(|value| Rgba::try_from(value).ok())
        .map(Into::into)
        .unwrap_or(fallback)
}

pub(super) fn color_value(color: Hsla) -> String {
    color.to_hex()
}

fn select_state(
    items: Vec<&'static str>,
    selected: usize,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SelectState<Vec<&'static str>>> {
    cx.new(|cx| SelectState::new(items, Some(IndexPath::default().row(selected)), window, cx))
}

fn color_state(color: Hsla, window: &mut Window, cx: &mut App) -> Entity<ColorPickerState> {
    cx.new(|cx| ColorPickerState::new(window, cx).default_value(color))
}

fn section_title(title: &'static str) -> impl IntoElement {
    div().text_sm().font_weight(FontWeight::MEDIUM).child(title)
}

fn label(text: &'static str) -> impl IntoElement {
    div().w(px(76.)).text_xs().child(text)
}

fn row(name: &'static str, control: impl IntoElement) -> impl IntoElement {
    h_flex()
        .items_center()
        .justify_between()
        .gap_1()
        .child(label(name))
        .child(control)
}

fn color_row(name: &'static str, picker: impl IntoElement) -> impl IntoElement {
    h_flex()
        .items_center()
        .justify_between()
        .child(label(name))
        .child(picker)
}

fn slider_row(
    name: &'static str,
    slider: impl IntoElement,
    value: String,
    value_color: Hsla,
) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap_1()
        .child(label(name))
        .child(slider)
        .child(
            div()
                .w(px(40.))
                .text_xs()
                .text_right()
                .text_color(value_color)
                .child(value),
        )
}
