//! Pages, and the pieces they share.

pub mod dotfiles;
pub mod history;
pub mod import;
pub mod modules;
pub mod overview;
pub mod review;
pub mod settings;
pub mod setup;

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonGroup, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Selectable, h_flex, v_flex};
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
        .w(px(badge_width()))
        .px(px(S8))
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

/// The proposal's 56 px fits every Chinese label; English ones need more.
fn badge_width() -> f32 {
    match crate::text::lang() {
        rosamta_core::i18n::Lang::ZhTw => 56.,
        rosamta_core::i18n::Lang::En => 96.,
    }
}

pub fn count_badge(count: usize, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    div()
        .h(px(S20))
        .line_height(px(S20))
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
        .px(px(S40))
        .pt(px(S32))
        .pb(px(S24))
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
                .px(px(S40))
                .pb(px(S48))
                .child(body),
        )
}

/// A titled group of rows.
pub fn section(title: SharedString, body: impl IntoElement, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    v_flex()
        .gap(px(S12))
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
                .overflow_hidden()
                // Hides the last row's rule under the frame.
                .child(div().mb(px(-1.)).child(body)),
        )
}

/// The main action: a filled capsule in the accent color.
pub fn prominent(button: Button) -> Button {
    button.primary().rounded_full().shadow_sm()
}

/// Every other action: a translucent capsule with a hairline edge, like
/// glass over the page. gpui-kit edges a custom button in its color and
/// fills it with a fifth of that color at rest.
pub fn glass(button: Button, cx: &App) -> Button {
    let p = palette(cx);
    let white = |alpha| hsla(0., 0., 1., alpha);
    let (hover, active) = if cx.theme().is_dark() {
        (white(0.12), white(0.04))
    } else {
        (p.panel, p.line)
    };
    button.rounded_full().border_1().custom(
        ButtonCustomVariant::new(cx)
            .color(p.line)
            .foreground(p.text)
            .hover(hover)
            .active(active),
    )
}

/// A quiet action: text only until hovered.
pub fn ghost(button: Button) -> Button {
    button.ghost().rounded_full()
}

/// A choice of one value: a capsule track holding a pill per option.
pub fn segments(group: ButtonGroup, cx: &App) -> ButtonGroup {
    group
        .p(px(2.))
        .gap(px(2.))
        .rounded_full()
        .bg(palette(cx).line)
}

/// One option of `segments`: the chosen one is a raised pill. Its colors
/// are the button's own so they hold while the choice is disabled.
pub fn segment(button: Button, selected: bool, cx: &App) -> Button {
    let button = button.selected(selected).ghost().rounded_full();
    if !selected {
        return button;
    }
    let p = palette(cx);
    let pill = if cx.theme().is_dark() {
        hsla(0., 0., 1., 0.14)
    } else {
        p.surface
    };
    button.bg(pill).text_color(p.text).shadow_sm()
}

/// One line inside a section, ruled from the next.
pub fn row(cx: &App) -> Div {
    let p = palette(cx);
    h_flex()
        .min_h(px(S40))
        .px(px(S16))
        .py(px(S8))
        .gap(px(S16))
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
        .children(without_header(text).map(|line| {
            let tint = if line.starts_with('+') {
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

/// The lines from the first hunk on: the title already names the file.
fn without_header(text: &str) -> impl Iterator<Item = &str> {
    text.lines().skip_while(|line| !line.starts_with("@@"))
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

#[cfg(test)]
mod tests {
    use super::without_header;

    #[test]
    fn diffs_keep_removed_lines_that_look_like_headers() {
        let text = "--- a\n+++ b\n@@ -1,2 +1,1 @@\n--- comment\n+++ added\n";
        let lines: Vec<&str> = without_header(text).collect();
        assert_eq!(lines, ["@@ -1,2 +1,1 @@", "--- comment", "+++ added"]);
    }
}
