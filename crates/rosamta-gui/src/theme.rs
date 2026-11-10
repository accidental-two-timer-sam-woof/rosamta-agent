//! Colors and spacing from the confirmed preview, in light and dark.

use gpui_kit::component::{ActiveTheme, Theme, ThemeRegistry};
use gpui_kit::*;

/// The preview's tokens. gpui-component's theme carries the same colors for
/// its own controls; these are for the app's lists, badges and panels.
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Hsla,
    pub panel: Hsla,
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
    pub code: Hsla,
}

const LIGHT: Palette = Palette {
    bg: hex(0xFBFBFA),
    panel: hex(0xF2F2EF),
    surface: hex(0xFFFFFF),
    line: hex(0xE2E1DC),
    line_strong: hex(0xCBCAC4),
    text: hex(0x1C1B19),
    muted: hex(0x5C5A54),
    accent: hex(0x2B57C9),
    accent_soft: hex(0xE7EDFB),
    ok: hex(0x1E6E46),
    ok_soft: hex(0xE2F1E8),
    warn: hex(0x8A5300),
    warn_soft: hex(0xFAEFD9),
    code: hex(0xF6F6F4),
};

const DARK: Palette = Palette {
    bg: hex(0x17171A),
    panel: hex(0x1E1E22),
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
    code: hex(0x1B1B1F),
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
    pub const S4: f32 = 4.;
    pub const S8: f32 = 8.;
    pub const S12: f32 = 12.;
    pub const S16: f32 = 16.;
    pub const S20: f32 = 20.;
    pub const S24: f32 = 24.;
    pub const S32: f32 = 32.;
    pub const S40: f32 = 40.;
    pub const S48: f32 = 48.;
}

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
    Theme::sync_system_appearance(Some(window), cx);
    window
        .observe_window_appearance(|window, cx| Theme::sync_system_appearance(Some(window), cx))
        .detach();
}
