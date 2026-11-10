//! The repo's commits and this host's snapshots.

use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rosamta_core::repo::CommitInfo;

use super::{mono, page, row, section};
use crate::app::Rosamta;
use crate::text::{t, when};
use crate::theme::{palette, space::*};

pub fn render(app: &Rosamta, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let body = v_flex()
        .gap(px(S24))
        .when(!app.snap.commits.is_empty(), |body| {
            body.child(section(
                t("history.commits"),
                list(&app.snap.commits, cx),
                cx,
            ))
        })
        .when(!app.snap.snapshots.is_empty(), |body| {
            body.child(section(
                t("history.snapshots"),
                list(&app.snap.snapshots, cx),
                cx,
            ))
        });
    page("history", t("nav.history"), div(), body, cx)
}

fn list(commits: &[CommitInfo], cx: &App) -> impl IntoElement {
    let p = palette(cx);
    v_flex().children(commits.iter().map(|commit| {
        row(cx)
            .child(
                div()
                    .flex_none()
                    .w(px(88.))
                    .text_color(p.muted)
                    .child(when(commit.at)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(commit.subject.clone()),
            )
            .child(mono(commit.hash.clone(), cx).text_color(p.muted))
    }))
}
