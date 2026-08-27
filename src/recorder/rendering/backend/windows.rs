//! Windows preview surface.
//!
//! # Integration constraints (audited against the pinned GPUI revision)
//!
//! GPUI's Windows renderer composes through DirectComposition. `WindowsWindow`
//! is created with `WS_EX_NOREDIRECTIONBITMAP` and `DirectXRenderer` calls
//! `IDCompositionDevice::CreateTargetForHwnd(hwnd, true)`, taking the *topmost*
//! composition target and pointing it at a composition swapchain. Its
//! composition device, target, and visual are private with no accessor.
//!
//! Three consequences shape this backend:
//!
//! 1. The window handle itself is public: `gpui::Window` implements
//!    `raw_window_handle::HasWindowHandle`, and the Windows implementation
//!    returns a real `Win32WindowHandle`. No fork is needed to find the HWND.
//! 2. We cannot join GPUI's composition tree, because the device and target it
//!    built are private.
//! 3. DirectComposition allows one topmost and one non-topmost target per HWND.
//!    GPUI holds the topmost one, so the non-topmost target should be free.
//!
//! This module takes that third route: an independent composition device and
//! the non-topmost target on GPUI's own HWND. Our visual then composes beneath
//! GPUI's output, which is the useful direction — the editor keeps painting
//! overlays over the video instead of losing them to a child window's airspace.
//!
//! # Confirmed by [`probe`]
//!
//! Windows does honour the second target. A surface attached this way composes
//! underneath GPUI's output, and GPUI keeps drawing its toolbar, inspector,
//! timeline, and canvas on top of it. No child window is involved, so there is
//! no airspace problem and the reconstructed cursor can stay in GPUI.
//!
//! Three opaque layers have to be cleared before anything underneath is
//! visible, and missing any one of them hides the surface completely:
//!
//! 1. The window itself. GPUI's Windows backend never reads
//!    `WindowOptions::window_background`, so the appearance must be set through
//!    `Window::set_background_appearance`. Only then does GPUI clear its
//!    swapchain to a transparent `[0.0; 4]` instead of an opaque `[1.0; 4]`.
//! 2. `gpui_component::Root`, which paints its own themed fill. It implements
//!    `Styled`, and its refinement is applied after that fill, so overriding
//!    the background there is enough.
//! 3. The editor's own shell and preview backgrounds.

mod background;
pub(crate) mod composition;
mod constants;
pub(crate) mod device;
mod resources;
mod shaders;
mod source;
mod surface;

pub(crate) use composition::CompositionRenderer;
pub(crate) use device::create as create_device;
pub(crate) use surface::{DirectCompositionSurface, window_handle};
