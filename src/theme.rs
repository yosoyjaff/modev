//! zeron's dark palette (crates/ui/src/theme.rs), in opaque sRGB.
//! zeron uses translucent tints (borders and hover at 8–16 % white); here they are
//! already composited onto its surface so they can be used with `rgb()`.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Rgba};

/// Content plane (#060606).
pub const BG: u32 = 0x060606;
/// Chrome: bars, panels and windows (#0d0d0d).
pub const SURFACE: u32 = 0x0d0d0d;
/// Alternate rows over `BG`.
pub const ROW_ALT: u32 = 0x0a0a0a;
/// Elevated plates inside a panel (zeron's popover, #161616).
pub const RAISED: u32 = 0x161616;
/// Hover: 92 % white at 11 %.
pub const HOVER: u32 = 0x252525;
/// Hairline: white at 8 %.
pub const BORDER: u32 = 0x202020;
pub const BORDER_STRONG: u32 = 0x2e2e2e;
/// neutral-200 / 400 / 500.
pub const TEXT: u32 = 0xe5e5e5;
pub const TEXT_MUTED: u32 = 0xa1a1a1;
pub const TEXT_FAINT: u32 = 0x737373;
/// zeron's indigo: main hue (text, icons) and strong fill (carries white text).
pub const ACCENT: u32 = 0x7c86ff;
pub const ACCENT_STRONG: u32 = 0x615fff;
/// `ACCENT_STRONG` at 45 % over `BG`: row selection.
pub const ACCENT_WASH: u32 = 0x2f2e76;
pub const DANGER: u32 = 0xff6467;
pub const WARNING: u32 = 0xffb900;
pub const SUCCESS: u32 = 0x00d492;
pub const INFO: u32 = 0x51a2ff;
pub const DEBUG: u32 = 0xc27aff;
/// JSON highlight (keys in the accent, like zeron).
pub const SYN_KEY: u32 = ACCENT;
pub const SYN_STRING: u32 = 0x5ee9b5;
pub const SYN_NUMBER: u32 = WARNING;
pub const SYN_LITERAL: u32 = DEBUG;
/// Search match background (amber at 35 % over `BG`).
pub const SEARCH_MATCH: u32 = 0x5c4304;
/// Close buttons red (like zeron/GNOME on hover).
pub const CLOSE_HOVER: u32 = 0xe81123;
/// Backgrounds/borders of console error and warning rows (10 % / 25 % of the color).
pub const DANGER_BG: u32 = 0x1f0f10;
pub const DANGER_BORDER: u32 = 0x4a1d1f;
pub const WARNING_BG: u32 = 0x1f1806;
pub const WARNING_BORDER: u32 = 0x4a3a08;

/// Light equivalents of each token. The whole UI paints with `rgb(TOKEN)`, so light
/// mode is resolved here without duplicating the palette in every view.
const LIGHT: &[(u32, u32)] = &[
    (BG, 0xffffff),
    (SURFACE, 0xf6f6f7),
    (ROW_ALT, 0xf9f9fa),
    (RAISED, 0xececee),
    (HOVER, 0xe4e4e7),
    (BORDER, 0xe1e1e4),
    (BORDER_STRONG, 0xcdcdd2),
    (TEXT, 0x18181b),
    (TEXT_MUTED, 0x5c5c66),
    (TEXT_FAINT, 0x8a8a94),
    (ACCENT, 0x4f46e5),
    (ACCENT_STRONG, 0xa5b4fc),
    (ACCENT_WASH, 0xdcdcfb),
    (DANGER, 0xd92d20),
    (WARNING, 0xb45309),
    (SUCCESS, 0x047857),
    (INFO, 0x2563eb),
    (DEBUG, 0x9333ea),
    (SYN_STRING, 0x047857),
    (SEARCH_MATCH, 0xfde68a),
    (DANGER_BG, 0xfef2f2),
    (DANGER_BORDER, 0xfecaca),
    (WARNING_BG, 0xfffbeb),
    (WARNING_BORDER, 0xfde68a),
    (0x888888, 0x6b6b75),
];

static IS_LIGHT: AtomicBool = AtomicBool::new(false);

pub fn is_light() -> bool {
    IS_LIGHT.load(Ordering::Relaxed)
}

/// Color of a token in the current mode (non-tokens pass through unchanged).
pub fn color(c: u32) -> u32 {
    if !is_light() {
        return c;
    }
    LIGHT
        .iter()
        .find(|(dark, _)| *dark == c)
        .map(|(_, light)| *light)
        .unwrap_or(c)
}

/// Replaces `gpui::rgb` across the UI: applies light/dark mode.
pub fn rgb(c: u32) -> Rgba {
    gpui_kit::rgb(color(c))
}

/// Switches the mode and repaints every window.
pub fn set_light(light: bool, cx: &mut App) {
    if light != is_light() {
        IS_LIGHT.store(light, Ordering::Relaxed);
        install(cx);
        cx.refresh_windows();
    }
}

/// gpui-kit theme with zeron's palette: used by its components (inputs, popups).
pub fn install(cx: &mut App) {
    Theme::change(
        if is_light() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        },
        None,
        cx,
    );
    Theme::update(cx, |t| {
        t.background = rgb(BG).into();
        t.foreground = rgb(TEXT).into();
        t.border = rgb(BORDER_STRONG).into();
        t.input = rgb(BORDER_STRONG).into();
        t.ring = rgb(ACCENT).into();
        t.caret = rgb(ACCENT).into();
        t.selection = rgb(ACCENT_WASH).into();
        t.muted = rgb(RAISED).into();
        t.muted_foreground = rgb(TEXT_FAINT).into();
        t.popover = rgb(RAISED).into();
        t.popover_foreground = rgb(TEXT).into();
        t.accent = rgb(HOVER).into();
        t.accent_foreground = rgb(TEXT).into();
        t.primary = rgb(ACCENT_STRONG).into();
        t.primary_hover = rgb(ACCENT).into();
        t.primary_foreground = if is_light() { rgb(TEXT) } else { rgb(0xffffff) }.into();
        t.list_hover = rgb(HOVER).into();
        t.list_active = rgb(ACCENT_WASH).into();
        t.scrollbar_thumb = rgb(BORDER_STRONG).into();
        t.scrollbar_thumb_hover = rgb(TEXT_FAINT).into();
        t.window_border = rgb(BORDER).into();
    });
}
