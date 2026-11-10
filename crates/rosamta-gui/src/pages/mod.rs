//! Pages, and the pieces they share.

pub mod dotfiles;
pub mod history;
pub mod modules;
pub mod overview;
pub mod review;
pub mod settings;
pub mod setup;

use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::{Palette, palette, space::*};

/// Badge colors by meaning.
#[derive(Clone, Copy)]
pub enum Tone {
    Accent,
    Ok,
    Warn,
    Plain,
}

impl Tone {
    fn colors(self, p: &Palette) -> (Hsla, Hsla) {
        match self {
            Tone::Accent => (p.accent_soft, p.accent),
            Tone::Ok => (p.ok_soft, p.ok),
            Tone::Warn => (p.warn_soft, p.warn),
            Tone::Plain => (p.code, p.muted),
        }
    }
}

/// A status or type label, the same width in every row.
pub fn badge(text: SharedString, tone: Tone, cx: &App) -> impl IntoElement {
    let (bg, fg) = tone.colors(&palette(cx));
    div()
        .flex_none()
        .w(px(88.))
        .h(px(S20))
        .line_height(px(S20))
        .rounded(px(S4))
        .bg(bg)
        .text_color(fg)
        .text_xs()
        .text_center()
        .truncate()
        .child(text)
}

pub fn count_badge(count: usize, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    div()
        .px(px(S8))
        .rounded_full()
        .bg(p.accent)
        .text_color(p.surface)
        .text_xs()
        .child(count.to_string())
}

/// The background service's status: green while it runs.
pub fn dot(running: bool, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    div()
        .flex_none()
        .size(px(S8))
        .rounded_full()
        .bg(if running { p.ok } else { p.line_strong })
}

/// A page's title, with its actions on the right.
pub fn header(title: SharedString, actions: impl IntoElement, cx: &App) -> impl IntoElement {
    h_flex()
        .flex_none()
        .px(px(S32))
        .pt(px(S24))
        .pb(px(S16))
        .gap(px(S12))
        .child(
            div()
                .flex_1()
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette(cx).text)
                .child(title),
        )
        .child(actions)
}

/// A page: its header and scrolling content.
pub fn page(
    id: &'static str,
    title: SharedString,
    actions: impl IntoElement,
    body: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .size_full()
        .child(header(title, actions, cx))
        .child(
            div()
                .id(id)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(S32))
                .pb(px(S32))
                .child(body),
        )
}

/// A titled group of rows.
pub fn section(title: SharedString, body: impl IntoElement, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    v_flex()
        .gap(px(S8))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(title),
        )
        .child(
            v_flex()
                .rounded(px(S8))
                .border_1()
                .border_color(p.line)
                .bg(p.surface)
                .child(body),
        )
}

/// One line inside a section, ruled from the next.
pub fn row(cx: &App) -> Div {
    let p = palette(cx);
    h_flex()
        .min_h(px(S40))
        .px(px(S16))
        .py(px(S8))
        .gap(px(S12))
        .text_sm()
        .border_b_1()
        .border_color(p.line)
}

/// A label column for label–value rows.
pub fn label(text: SharedString, cx: &App) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(160.))
        .text_color(palette(cx).muted)
        .child(text)
}

pub fn mono(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .font_family(cx.theme().mono_font_family.clone())
        .text_xs()
        .child(text.into())
}

/// Text in the middle of an empty list.
pub fn empty(text: SharedString, cx: &App) -> impl IntoElement {
    div()
        .py(px(S48))
        .text_center()
        .text_sm()
        .text_color(palette(cx).muted)
        .child(text)
}

/// A unified diff, added and removed lines tinted.
pub fn diff(text: &str, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let font = cx.theme().mono_font_family.clone();
    v_flex()
        .rounded(px(S8))
        .bg(p.code)
        .py(px(S8))
        .font_family(font)
        .text_xs()
        .children(text.lines().map(|line| {
            let tint = if line.starts_with("+++") || line.starts_with("---") {
                None
            } else if line.starts_with('+') {
                Some((p.ok_soft, p.ok))
            } else if line.starts_with('-') {
                Some((p.warn_soft, p.warn))
            } else {
                None
            };
            let muted =
                line.starts_with("@@") || line.starts_with("+++") || line.starts_with("---");
            div()
                .px(px(S12))
                .whitespace_nowrap()
                .when_some(tint, |line, (bg, fg)| line.bg(bg).text_color(fg))
                .when(muted, |line| line.text_color(p.muted))
                .child(SharedString::from(line.to_string()))
        }))
}

/// A code block for whole files.
pub fn code(text: &str, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    v_flex()
        .rounded(px(S8))
        .bg(p.code)
        .px(px(S12))
        .py(px(S8))
        .font_family(cx.theme().mono_font_family.clone())
        .text_xs()
        .children(text.lines().map(|line| {
            div()
                .min_h(px(S16))
                .child(SharedString::from(line.to_string()))
        }))
}
