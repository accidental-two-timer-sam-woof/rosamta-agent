//! Dotfiles for this host: how each is deployed and where it stands.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rosamta_core::adopt;
use rosamta_core::config::Deploy;

use super::{Tone, badge, empty, mono, page, row};
use crate::app::Rosamta;
use crate::state::DotfileStatus;
use crate::text::t;
use crate::theme::{palette, space::*};

pub fn render(app: &Rosamta, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let p = palette(cx);
    let home = &app.snap.home;
    let body = if app.snap.dotfiles.is_empty() {
        empty(t("dotfiles.empty"), cx).into_any_element()
    } else {
        v_flex()
            .rounded(px(S8))
            .border_1()
            .border_color(p.line)
            .bg(p.surface)
            .children(app.snap.dotfiles.iter().map(|dotfile| {
                let deploy = match dotfile.deploy {
                    Deploy::Link => "dotfiles.link",
                    Deploy::Copy => "dotfiles.copy",
                    Deploy::Template => "dotfiles.template",
                };
                let (status, tone) = match dotfile.status {
                    DotfileStatus::Deployed => ("dotfiles.deployed", Tone::Ok),
                    DotfileStatus::Pending => ("dotfiles.pending", Tone::Warn),
                    DotfileStatus::Skipped => ("dotfiles.skipped", Tone::Plain),
                    DotfileStatus::NotDeployed => ("dotfiles.not_deployed", Tone::Plain),
                };
                let path = adopt::display(&home.join(&dotfile.path), home);
                row(cx)
                    .child(mono(path, cx).flex_1().min_w_0().truncate())
                    .child(
                        h_flex()
                            .gap(px(S8))
                            .when(dotfile.encrypted, |badges| {
                                badges.child(badge(t("badge.encrypted"), Tone::Accent, cx))
                            })
                            .child(badge(t(deploy), Tone::Plain, cx))
                            .child(badge(t(status), tone, cx)),
                    )
            }))
            .into_any_element()
    };
    page("dotfiles", t("nav.dotfiles"), div(), body, cx)
}
