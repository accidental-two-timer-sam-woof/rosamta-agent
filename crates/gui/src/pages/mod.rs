//! Pages, and the pieces they share.

pub mod history;
pub mod import;
pub mod library;
pub mod review;
pub mod settings;
pub mod setup;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, TitleBar, h_flex, v_flex};
use gpui_kit::*;

use crate::theme::{Palette, font, palette, radius, size, space::*};

/// Badge colors by the state they stand for.
#[derive(Clone, Copy)]
pub enum Tone {
    Ok,
    Warn,
    Plain,
}

impl Tone {
    fn colors(self, p: &Palette) -> (Hsla, Hsla) {
        match self {
            Tone::Ok => (p.ok_soft, p.ok),
            Tone::Warn => (p.warn_soft, p.warn),
            Tone::Plain => (p.chip, p.muted),
        }
    }
}

/// A state, in a capsule the same width in every row.
pub fn badge(text: SharedString, tone: Tone, cx: &App) -> impl IntoElement {
    let (bg, fg) = tone.colors(&palette(cx));
    div()
        .flex_none()
        .min_w(px(badge_width()))
        .px(px(S8))
        .h(px(S20))
        .line_height(px(S20))
        .rounded_full()
        .bg(bg)
        .text_color(fg)
        .text_size(px(font::CAPTION))
        .text_center()
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
        .flex_none()
        .min_w(px(S20))
        .h(px(18.))
        .line_height(px(18.))
        .px(px(6.))
        .rounded_full()
        .bg(p.accent)
        .text_color(p.surface)
        .text_size(px(font::CAPTION))
        .text_center()
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

/// The bar over a page: its title, what switches the view, and the page's
/// actions at the trailing end. It is the window's title bar: it drags the
/// window, and holds the window's buttons where the system draws none.
pub fn toolbar(
    title: SharedString,
    leading: impl IntoElement,
    trailing: impl IntoElement,
    sidebar: bool,
    cx: &App,
) -> impl IntoElement {
    let p = palette(cx);
    // Without the sidebar, macOS draws the window's buttons over this bar.
    let inset = if cfg!(target_os = "macos") && !sidebar {
        88.
    } else {
        S40
    };
    TitleBar::new()
        .h(px(size::TOOLBAR))
        .pl(px(inset))
        .pr(px(S24))
        .bg(p.bg)
        .border_color(p.line)
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(S12))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(font::TITLE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(leading)
                .child(div().flex_1())
                .child(trailing),
        )
}

/// A page's scrolling content, under its toolbar.
pub fn body(id: &'static str, content: impl IntoElement) -> impl IntoElement {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(S40))
        .pt(px(S24))
        .pb(px(S32))
        .child(content)
}

/// The frame around a group of rows.
pub fn frame(cx: &App) -> Div {
    let p = palette(cx);
    v_flex()
        .rounded(px(radius::BOX))
        .border_1()
        .border_color(p.line)
        .bg(p.surface)
        .overflow_hidden()
}

/// A titled group of rows.
pub fn section(title: SharedString, rows: impl IntoElement, cx: &App) -> Div {
    v_flex()
        .gap(px(S12))
        .child(heading(title, cx))
        // Hides the last row's rule under the frame.
        .child(frame(cx).child(div().mb(px(-1.)).child(rows)))
}

pub fn heading(title: SharedString, cx: &App) -> impl IntoElement {
    div()
        .px(px(S4))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(palette(cx).muted)
        .child(title)
}

/// The main action: a filled capsule in the accent color. One per screen,
/// at the trailing end.
pub fn prominent(button: Button) -> Button {
    button.primary().rounded_full()
}

/// Any other action: a capsule with a hairline edge. gpui-kit edges a
/// custom button in its color and fills it with a fifth of that at rest.
pub fn bordered(button: Button, cx: &App) -> Button {
    let p = palette(cx);
    let white = |alpha| hsla(0., 0., 1., alpha);
    let (hover, active) = if cx.theme().is_dark() {
        (white(0.12), white(0.04))
    } else {
        (p.panel, p.line)
    };
    button.rounded_full().border_1().custom(
        ButtonCustomVariant::new(cx)
            .color(p.line_strong)
            .foreground(p.text)
            .hover(hover)
            .active(active),
    )
}

/// An action that dismisses: text only until hovered.
pub fn borderless(button: Button) -> Button {
    button.ghost().rounded_full()
}

/// An action shown as its icon alone; pointing at it says what it does.
pub fn icon_button(id: impl Into<ElementId>, icon: IconName, tip: SharedString) -> Button {
    Button::new(id)
        .icon(icon)
        .ghost()
        .small()
        .rounded_full()
        .tooltip(tip)
}

/// A property shown as an icon; pointing at it says what it means.
pub fn hint(
    id: impl Into<ElementId>,
    icon: IconName,
    tip: SharedString,
    cx: &App,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_none()
        .w(px(S20))
        .flex()
        .justify_center()
        .child(Icon::new(icon).size(px(14.)).text_color(palette(cx).muted))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
}

/// A choice of one value or view; the chosen one slides under its label.
/// `options` are each label and whether it can be chosen.
pub fn segmented(
    id: impl Into<ElementId>,
    options: impl IntoIterator<Item = (SharedString, bool)>,
    selected: usize,
    on_pick: impl Fn(&usize, &mut Window, &mut App) + 'static,
) -> TabBar {
    TabBar::new(id)
        .segmented()
        .selected_index(selected)
        .children(
            options
                .into_iter()
                .map(|(label, enabled)| Tab::new().label(label).disabled(!enabled)),
        )
        .on_click(on_pick)
}

/// One line inside a frame, ruled from the next.
pub fn row(cx: &App) -> Div {
    h_flex()
        .min_h(px(S40))
        .px(px(S16))
        .py(px(S4))
        .gap(px(S12))
        .border_b_1()
        .border_color(palette(cx).line)
}

pub fn mono(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(px(font::MONO))
        .child(text.into())
}

/// Text in the middle of where a list or a detail would be.
pub fn empty(text: SharedString, cx: &App) -> impl IntoElement {
    div()
        .size_full()
        .min_h(px(160.))
        .flex()
        .items_center()
        .justify_center()
        .text_color(palette(cx).muted)
        .child(text)
}

/// A named block of a detail: a header bar with what it is about and its
/// action, over its content.
pub fn card(
    title: SharedString,
    subject: Option<SharedString>,
    action: Option<AnyElement>,
    content: impl IntoElement,
    cx: &App,
) -> Div {
    let p = palette(cx);
    frame(cx)
        .child(
            h_flex()
                .min_h(px(S40))
                .pl(px(S16))
                .pr(px(S8))
                .gap(px(S12))
                .bg(p.well)
                .border_b_1()
                .border_color(p.line)
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().flex_1().min_w_0().children(
                    subject.map(|subject| mono(subject, cx).truncate().text_color(p.muted)),
                ))
                .children(action),
        )
        .child(content)
}

/// One numbered line of code: its number, an added or removed mark, the text.
fn line(number: Option<u32>, mark: &str, text: &str, cx: &App) -> Div {
    h_flex()
        .items_start()
        .min_h(px(S20))
        .line_height(px(S20))
        .child(
            div()
                .flex_none()
                .w(px(44.))
                .pr(px(S12))
                .text_right()
                .text_color(palette(cx).muted)
                .children(number.map(|number| number.to_string())),
        )
        .child(
            div()
                .flex_none()
                .w(px(S16))
                .child(SharedString::from(mark.to_string())),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(SharedString::from(text.to_string())),
        )
}

fn listing(cx: &App) -> Div {
    v_flex()
        .py(px(S8))
        .overflow_hidden()
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(px(font::MONO))
}

/// A whole file, its lines numbered.
pub fn code(text: &str, cx: &App) -> impl IntoElement {
    listing(cx).children(
        text.lines()
            .zip(1..)
            .map(|(text, number)| line(Some(number), "", text, cx)),
    )
}

/// A unified diff: lines numbered, added and removed ones tinted.
pub fn diff(text: &str, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    listing(cx).children(numbered(text).into_iter().map(|(number, text)| {
        let Some(number) = number else {
            return line(None, "", "⋯", cx).text_color(p.muted);
        };
        let (mark, rest) = text.split_at(text.len().min(1));
        let row = line(Some(number), mark.trim(), rest, cx);
        match mark {
            "+" => row.bg(p.ok_soft).text_color(p.ok),
            "-" => row.bg(p.danger_soft).text_color(p.danger),
            _ => row,
        }
    }))
}

/// A diff's lines from the first hunk on, each with its number in the file
/// it is from: the old one for removed and unchanged lines, the new one for
/// added lines. A line without a number is the gap between two hunks.
fn numbered(text: &str) -> Vec<(Option<u32>, &str)> {
    let (mut old, mut new) = (0, 0);
    let mut out = Vec::new();
    for text in text.lines().skip_while(|line| !line.starts_with("@@")) {
        if let Some(header) = text.strip_prefix("@@ -") {
            // `@@ -old[,count] +new[,count] @@`
            let start = |part: Option<&str>| {
                let digits = part?.trim_start_matches('+').split(',').next()?;
                digits.parse::<u32>().ok()
            };
            let mut parts = header.split(' ');
            (old, new) = (
                start(parts.next()).unwrap_or(1),
                start(parts.next()).unwrap_or(1),
            );
            if !out.is_empty() {
                out.push((None, ""));
            }
        } else if text.starts_with('+') {
            out.push((Some(new), text));
            new += 1;
        } else if !text.starts_with('\\') {
            // Not "\ No newline at end of file".
            out.push((Some(old), text));
            old += 1;
            new += u32::from(!text.starts_with('-'));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::numbered;

    #[test]
    fn diff_lines_are_numbered_by_the_file_they_are_from() {
        let text = "--- a\n+++ b\n@@ -6,3 +6,2 @@\n keep\n--- comment\n+++ added\n\\ No newline at end of file\n@@ -20 +19,2 @@\n last\n+more\n";
        assert_eq!(
            numbered(text),
            [
                (Some(6), " keep"),
                // Removed and added lines that look like a diff's header.
                (Some(7), "--- comment"),
                (Some(7), "+++ added"),
                (None, ""),
                (Some(20), " last"),
                (Some(20), "+more"),
            ]
        );
    }
}
