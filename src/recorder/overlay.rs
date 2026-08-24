use std::{
    cell::RefCell,
    ffi::c_void,
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Root, Sizable, Size,
    button::{Button, ButtonVariants},
    h_flex,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE},
};

use super::ui::RecorderView;

const TIMER_INTERVAL: Duration = Duration::from_millis(250);
const OVERLAY_SIZE: gpui::Size<gpui::Pixels> = size(px(292.), px(48.));
const BOTTOM_MARGIN: Pixels = px(24.);

pub(super) struct RecordingOverlay {
    recorder: WeakEntity<RecorderView>,
    started_at: Instant,
    _timer_task: Task<()>,
}

impl RecordingOverlay {
    fn new(
        window: &mut Window,
        recorder: WeakEntity<RecorderView>,
        started_at: Instant,
        cx: &mut Context<Self>,
    ) -> Self {
        let timer_task = cx.spawn_in(window, async move |overlay, cx| {
            loop {
                cx.background_executor().timer(TIMER_INTERVAL).await;
                if overlay.update_in(cx, |_, _, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });

        Self {
            recorder,
            started_at,
            _timer_task: timer_task,
        }
    }
}

impl Render for RecordingOverlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let recorder = self.recorder.clone();
        let stop_button = Button::new("overlay-stop")
            .danger()
            .compact()
            .with_size(Size::Small)
            .label("Stop")
            .on_click(move |_, _, app| {
                let _ = recorder.update(app, |view, cx| view.stop_recording(cx));
            });

        div()
            .size_full()
            .flex()
            .items_center()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .shadow_lg()
            .p_2()
            .child(
                h_flex()
                    .flex_1()
                    .h_full()
                    .gap_2()
                    .items_center()
                    .px_1()
                    .window_control_area(WindowControlArea::Drag)
                    .child(div().size(px(7.)).rounded_full().bg(cx.theme().danger))
                    .child(
                        div()
                            .text_lg()
                            .font_family("Cascadia Mono")
                            .child(format_elapsed(self.started_at.elapsed())),
                    ),
            )
            .child(stop_button)
    }
}

pub(super) fn open(
    cx: &mut AsyncApp,
    recorder: WeakEntity<RecorderView>,
    display_id: DisplayId,
    started_at: Instant,
) -> Result<WindowHandle<Root>> {
    let exclusion_error = Rc::new(RefCell::new(None));
    let exclusion_error_for_window = exclusion_error.clone();
    let options = cx.update(|app| overlay_options(app, display_id));

    let handle = cx.open_window(options, move |window, cx| {
        let overlay = cx.new(|cx| RecordingOverlay::new(window, recorder, started_at, cx));
        let can_show = match exclude_from_capture(window) {
            Ok(()) => true,
            Err(error) => {
                *exclusion_error_for_window.borrow_mut() = Some(error.to_string());
                window.remove_window();
                false
            }
        };

        let root = cx.new(|cx| Root::new(overlay, window, cx));
        if can_show {
            window.activate_window();
        }
        root
    })?;

    if let Some(error) = exclusion_error.borrow_mut().take() {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
        return Err(anyhow!("overlay capture exclusion failed: {error}"));
    }

    Ok(handle)
}

fn overlay_options(cx: &App, display_id: DisplayId) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(overlay_bounds(cx, display_id))),
        display_id: Some(display_id),
        titlebar: None,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: true,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    }
}

fn overlay_bounds(cx: &App, display_id: DisplayId) -> Bounds<Pixels> {
    let Some(display) = cx.find_display(display_id).or_else(|| cx.primary_display()) else {
        return Bounds::centered(None, OVERLAY_SIZE, cx);
    };

    let viewport = display.bounds();
    let center = point(
        viewport.center().x,
        viewport.bottom() - BOTTOM_MARGIN - OVERLAY_SIZE.height.half(),
    );
    Bounds::centered_at(center, OVERLAY_SIZE)
}

fn exclude_from_capture(window: &Window) -> Result<()> {
    let raw_handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow!("could not access overlay window handle: {error}"))?
        .as_raw();
    let RawWindowHandle::Win32(raw_handle) = raw_handle else {
        return Err(anyhow!("overlay does not have a Win32 window handle"));
    };

    let hwnd = HWND(raw_handle.hwnd.get() as *mut c_void);
    unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) }
        .map_err(|error| anyhow!("SetWindowDisplayAffinity failed: {error}"))
}

fn format_elapsed(elapsed: Duration) -> String {
    let total_seconds = elapsed.as_secs();
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds / 60) % 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::format_elapsed;

    #[test]
    fn formats_elapsed_time_as_clock() {
        assert_eq!(
            format_elapsed(std::time::Duration::from_secs(102)),
            "00:01:42"
        );
    }
}
