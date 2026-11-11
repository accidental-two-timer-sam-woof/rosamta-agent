//! Colors, sizes and motion from the confirmed preview, in light and dark.

use std::time::Duration;

use gpui_kit::base::{Easing, Spring};
use gpui_kit::component::{ActiveTheme, Theme, ThemeRegistry};
use gpui_kit::*;

/// The preview's tokens. gpui-component's theme (`theme.json`) carries the
/// same colors for its own controls; these are for the app's own surfaces.
#[derive(Clone, Copy)]
pub struct Palette {
    /// The content area.
    pub bg: Hsla,
    /// The sidebar.
    pub panel: Hsla,
    /// Recessed areas: the review list's column, card headers, the settings window.
    pub well: Hsla,
    /// Boxes of rows.
    pub surface: Hsla,
    pub line: Hsla,
    pub line_strong: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub ok: Hsla,
    pub ok_soft: Hsla,
    pub warn: Hsla,
    pub warn_soft: Hsla,
    pub danger: Hsla,
    pub danger_soft: Hsla,
    /// Neutral badges and tags.
    pub chip: Hsla,
}

const LIGHT: Palette = Palette {
    bg: hex(0xFFFFFF),
    panel: hex(0xF1F1F4),
    well: hex(0xF6F6F8),
    surface: hex(0xFFFFFF),
    line: hex(0xE1E1E6),
    line_strong: hex(0xC9C9CF),
    text: hex(0x1D1D1F),
    muted: hex(0x5F5F64),
    accent: hex(0x2B57C9),
    accent_soft: hex(0xE4EAFB),
    ok: hex(0x175C34),
    ok_soft: hex(0xE3F3EA),
    warn: hex(0x7A4700),
    warn_soft: hex(0xFBEFD9),
    danger: hex(0x8C1D18),
    danger_soft: hex(0xFBE9E7),
    chip: hex(0xEDEDF0),
};

const DARK: Palette = Palette {
    bg: hex(0x17171A),
    panel: hex(0x1E1E22),
    well: hex(0x1B1B1F),
    surface: hex(0x232328),
    line: hex(0x34343B),
    line_strong: hex(0x4A4A52),
    text: hex(0xECECEF),
    muted: hex(0xA6A6AF),
    accent: hex(0x7FA4FF),
    accent_soft: hex(0x22304F),
    ok: hex(0x62C793),
    ok_soft: hex(0x173326),
    warn: hex(0xE9B865),
    warn_soft: hex(0x3A2D15),
    danger: hex(0xF0958C),
    danger_soft: hex(0x3C1B18),
    chip: hex(0x2C2C32),
};

const fn hex(rgb: u32) -> Hsla {
    // `rgb()` is not const; convert by hand.
    let r = ((rgb >> 16) & 0xFF) as f32 / 255.0;
    let g = ((rgb >> 8) & 0xFF) as f32 / 255.0;
    let b = (rgb & 0xFF) as f32 / 255.0;
    let max = if r > g {
        if r > b { r } else { b }
    } else if g > b {
        g
    } else {
        b
    };
    let min = if r < g {
        if r < b { r } else { b }
    } else if g < b {
        g
    } else {
        b
    };
    let l = (max + min) / 2.0;
    if max == min {
        return Hsla {
            h: 0.0,
            s: 0.0,
            l,
            a: 1.0,
        };
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    Hsla {
        h: h / 6.0,
        s,
        l,
        a: 1.0,
    }
}

pub fn palette(cx: &App) -> Palette {
    if cx.theme().is_dark() { DARK } else { LIGHT }
}

/// Spacing, on the 4 px scale the proposal allows.
pub mod space {
    pub const S2: f32 = 2.;
    pub const S4: f32 = 4.;
    pub const S8: f32 = 8.;
    pub const S12: f32 = 12.;
    pub const S16: f32 = 16.;
    pub const S20: f32 = 20.;
    pub const S24: f32 = 24.;
    pub const S32: f32 = 32.;
    pub const S40: f32 = 40.;
}

/// Text sizes: one per role.
pub mod font {
    /// Toolbar and detail titles.
    pub const TITLE: f32 = 17.;
    /// Rows, labels and buttons.
    pub const BODY: f32 = 13.;
    /// Times, counts and badges.
    pub const CAPTION: f32 = 11.;
    /// Paths, code and diffs.
    pub const MONO: f32 = 12.;
}

/// Corner radii, concentric: a row inside a box is the box's less its inset.
pub mod radius {
    pub const BOX: f32 = 12.;
    pub const ROW: f32 = 8.;
}

/// Fixed sizes of the window's parts.
pub mod size {
    pub const TOOLBAR: f32 = 52.;
    pub const SIDEBAR: f32 = 200.;
    pub const LIST: f32 = 340.;
}

/// The sidebar lets the desktop show through, blurred, where the system can
/// do that; screenshots are opaque so they come out the same everywhere.
pub const TRANSLUCENT: bool = cfg!(all(target_os = "macos", not(feature = "screenshot")));

/// What moves, moves this long, easing in and out.
pub const MOVE: Duration = Duration::from_millis(200);
/// What appears, fades in this long.
pub const FADE: Duration = Duration::from_millis(150);

/// Register the light and dark themes and follow the system's appearance.
pub fn init(window: &mut Window, cx: &mut App) {
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(include_str!("theme.json"))
        .expect("the bundled theme is valid");
    let themes = ThemeRegistry::global(cx).themes();
    let (light, dark) = (
        themes["rosamta Light"].clone(),
        themes["rosamta Dark"].clone(),
    );
    let theme = Theme::global_mut(cx);
    theme.light_theme = light;
    theme.dark_theme = dark;
    // Switches and segmented controls slide without overshoot.
    theme.motion.spring_move = Spring::new(MOVE);
    theme.motion.easing_move = Easing::EaseInOut;
    Theme::sync_system_appearance(Some(window), cx);
    window
        .observe_window_appearance(|window, cx| Theme::sync_system_appearance(Some(window), cx))
        .detach();
}
