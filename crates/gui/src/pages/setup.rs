//! A host without a repo: create one, or join one by URL.

use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::actions;
use crate::app::Rosamta;
use crate::text::plain;
use crate::text::t;
use crate::theme::{font, palette, radius, size, space::*};

pub fn render(app: &Rosamta, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let p = palette(cx);
    let busy = app.busy.is_some();
    let card = || {
        v_flex()
            .p(px(S20))
            .gap(px(S12))
            .rounded(px(radius::BOX))
            .border_1()
            .border_color(p.line)
            .bg(p.surface)
    };
    // Nothing is shown in the title bar; it is there to move the window by.
    let bar = TitleBar::new()
        .h(px(size::TOOLBAR))
        .bg(p.bg)
        .border_color(p.bg);
    let page = v_flex().flex_1().items_center().justify_center().child(
        v_flex()
            .w(px(440.))
            .gap(px(S16))
            .child(
                div()
                    .text_size(px(font::TITLE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .pb(px(S8))
                    .child(t("setup.title")),
            )
            .children(app.snap.problems.first().map(|problem| {
                div()
                    .text_color(p.warn)
                    .child(plain(&problem.arg).to_string())
            }))
            .child(
                card().child(
                    h_flex()
                        .gap(px(S12))
                        .child(div().flex_1().child(t("setup.new")))
                        .child(
                            super::prominent(Button::new("create"))
                                .label(t("setup.create"))
                                .loading(app.busy == Some("create"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.run(window, cx, "create", |_| actions::init(None));
                                })),
                        ),
                ),
            )
            .when(app.snap.chezmoi, |column| {
                column.child(
                    card().child(
                        h_flex()
                            .gap(px(S12))
                            .child(div().flex_1().child(t("setup.chezmoi")))
                            .child(
                                super::bordered(Button::new("import"), cx)
                                    .label(t("setup.import"))
                                    .loading(app.busy == Some("import"))
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import_chezmoi(window, cx);
                                    })),
                            ),
                    ),
                )
            })
            .child(
                card().child(div().child(t("setup.join"))).child(
                    h_flex()
                        .gap(px(S8))
                        .child(div().flex_1().child(Input::new(&app.url)))
                        .child(
                            super::bordered(Button::new("join"), cx)
                                .label(t("setup.join_button"))
                                .loading(app.busy == Some("join"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let url = this.url.read(cx).value().trim().to_string();
                                    if !url.is_empty() {
                                        this.run(window, cx, "join", move |_| {
                                            actions::init(Some(url))
                                        });
                                    }
                                })),
                        ),
                ),
            ),
    );
    v_flex().size_full().child(bar).child(page)
}
