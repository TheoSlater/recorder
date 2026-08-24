mod capture;
mod components;
mod cursor;
mod encoder;
mod hooks;
mod input;
mod lifecycle;
mod media;
mod model;
mod monitors;
mod overlay;
mod playback;
mod session;
mod ui;

pub(crate) use lifecycle::ShutdownCoordinator;
pub(crate) use monitors::enumerate_monitors;
pub(crate) use ui::RecorderView;
