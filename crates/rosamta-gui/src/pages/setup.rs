//! A host without a repo: create one, or join one by URL.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, h_flex, v_flex};
use gpui_kit::*;

use crate::actions;
use crate::app::Rosamta;
use crate::text::t;
use crate::theme::{palette, space::*};

pub fn render(app: &Rosamta, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let p = palette(cx);
    let busy = app.busy;
    let card = || {
        v_flex()
            .p(px(S20))
            .gap(px(S12))
            .rounded(px(S8))
            .border_1()
            .border_color(p.line)
            .bg(p.surface)
    };
    v_flex().size_full().items_center().justify_center().child(
        v_flex()
            .w(px(440.))
            .gap(px(S16))
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .pb(px(S8))
                    .child(t("setup.title")),
            )
            .children(
                app.snap
                    .problem
                    .clone()
                    .map(|problem| div().text_sm().text_color(p.warn).child(problem)),
            )
            .child(
                card().child(
                    h_flex()
                        .gap(px(S12))
                        .child(div().flex_1().text_sm().child(t("setup.new")))
                        .child(
                            Button::new("create")
                                .label(t("setup.create"))
                                .primary()
                                .loading(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.run(window, cx, |_| actions::init(None));
                                })),
                        ),
                ),
            )
            .child(
                card().child(div().text_sm().child(t("setup.join"))).child(
                    h_flex()
                        .gap(px(S8))
                        .child(div().flex_1().child(Input::new(&app.url)))
                        .child(
                            Button::new("join")
                                .label(t("setup.join_button"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let url = this.url.read(cx).value().trim().to_string();
                                    if !url.is_empty() {
                                        this.run(window, cx, move |_| actions::init(Some(url)));
                                    }
                                })),
                        ),
                ),
            ),
    )
}
