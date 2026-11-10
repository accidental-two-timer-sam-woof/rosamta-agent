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
        .gap(px(S32))
        .when(!app.snap.commits.is_empty(), |body| {
            body.child(section(
                t("history.commits"),
                list(&app.snap.commits, false, cx),
                cx,
            ))
        })
        .when(!app.snap.snapshots.is_empty(), |body| {
            body.child(section(
                t("history.snapshots"),
                list(&app.snap.snapshots, true, cx),
                cx,
            ))
        });
    page("history", t("nav.history"), div(), body, cx)
}

/// Commits with their hash; snapshots with the files they saved instead.
fn list(commits: &[CommitInfo], snapshots: bool, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    v_flex().children(commits.iter().map(|commit| {
        let last = if snapshots {
            let files: Vec<String> = commit.files.iter().filter_map(|file| saved(file)).collect();
            mono(files.join("  "), cx).flex_1().min_w_0().truncate()
        } else {
            mono(commit.hash.clone(), cx)
        };
        row(cx)
            .child(
                div()
                    .flex_none()
                    .w(px(96.))
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
            .child(last.text_color(p.muted))
    }))
}

/// A file a snapshot saved, as the user knows it: `files/home/x` is `~/x`.
fn saved(path: &str) -> Option<String> {
    let key = path.strip_prefix("files/")?;
    Some(match key.strip_prefix("home/") {
        Some(rel) => format!("~/{rel}"),
        None => format!("/{}", key.strip_prefix("root/").unwrap_or(key)),
    })
}

#[cfg(test)]
mod tests {
    use super::saved;

    #[test]
    fn snapshot_keys_read_as_paths() {
        assert_eq!(saved("files/home/.zshrc").as_deref(), Some("~/.zshrc"));
        assert_eq!(
            saved("files/root/etc/zshenv").as_deref(),
            Some("/etc/zshenv")
        );
        assert_eq!(saved("original/home/.zshrc"), None);
        assert_eq!(saved("original-absent"), None);
    }
}
