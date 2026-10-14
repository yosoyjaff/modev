//! macOS window tweaks gpui doesn't expose.

use gpui_kit::Window;
use objc2_app_kit::{NSView, NSWindowButton};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Without the system shadow: on a transparent window it outlines the whole rectangle,
/// gaps between the panels included, like a pane of glass. The zoom button is disabled
/// (no maximize, project decision).
pub fn style_window(window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: gpui's AppKit handle points to its live content NSView, and window
    // creation runs on the main thread.
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    let Some(ns_window) = view.window() else {
        return;
    };
    ns_window.setHasShadow(false);
    if let Some(zoom) = ns_window.standardWindowButton(NSWindowButton::ZoomButton) {
        zoom.setEnabled(false);
    }
}
