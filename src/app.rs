use gpui::{AppContext, WindowBounds, WindowOptions, px, size};
use gpui_component::Root;

use crate::recorder::{RecorderView, enumerate_monitors};

pub(crate) fn run() {
    let monitors = enumerate_monitors();
    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(680.), px(400.)), cx)),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|_| RecorderView::new(monitors));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open recorder window");
        })
        .detach();
    });
}
