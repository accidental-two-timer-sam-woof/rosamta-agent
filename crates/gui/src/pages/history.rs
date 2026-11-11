//! What changed and when: the repo's commits, and this host's snapshots.

use gpui_kit::component::v_flex;
use gpui_kit::*;
use rosamta_core::repo::CommitInfo;

use super::{body, empty, frame, mono, row, segmented, toolbar};
use crate::app::{HistoryTab, Rosamta};
use crate::text::{t, tf, when};
use crate::theme::{font, palette};

pub fn render(app: &Rosamta, cx: &mut Context<Rosamta>) -> impl IntoElement {
    let tabs = segmented(
        "history-tabs",
        [(t("history.changes"), true), (t("history.snapshots"), true)],
        usize::from(app.history == HistoryTab::Snapshots),
        cx.listener(|this, index: &usize, _, cx| {
            this.history = if *index == 1 {
                HistoryTab::Snapshots
            } else {
                HistoryTab::Changes
            };
            cx.notify();
        }),
    );
    let snapshots = app.history == HistoryTab::Snapshots;
    let commits = if snapshots {
        &app.snap.snapshots
    } else {
        &app.snap.commits
    };
    let content = if commits.is_empty() {
        empty(t("history.empty"), cx).into_any_element()
    } else {
        list(commits, snapshots, cx).into_any_element()
    };
    v_flex()
        .size_full()
        .child(toolbar(t("nav.history"), tabs, div(), true, cx))
        .child(body("history", content))
}

/// Commits as git has them, with their hash; snapshots by what they were
/// taken for, with the files they saved.
fn list(commits: &[CommitInfo], snapshots: bool, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let rows = commits.iter().map(|commit| {
        let (what, about) = if snapshots {
            let files: Vec<String> = commit.files.iter().filter_map(|file| saved(file)).collect();
            (
                div().child(reason(&commit.subject)),
                mono(files.join("  "), cx),
            )
        } else {
            (
                mono(commit.subject.clone(), cx),
                mono(commit.hash.clone(), cx).flex_none(),
            )
        };
        row(cx)
            .child(
                div()
                    .flex_none()
                    .w(px(88.))
                    .text_size(px(font::CAPTION))
                    .text_color(p.muted)
                    .child(when(commit.at)),
            )
            .child(what.flex_1().min_w_0().truncate())
            .child(about.min_w_0().truncate().text_color(p.muted))
    });
    frame(cx).child(div().mb(px(-1.)).children(rows))
}

/// Why a snapshot was taken, in the interface's language. Core records it
/// in English, as the snapshot's commit subject.
fn reason(subject: &str) -> SharedString {
    let key = match subject {
        "deploy dotfiles" => "history.reason_deploy",
        "adopt" => "history.reason_adopt",
        "install hooks" => "history.reason_install_hooks",
        "eject" => "history.reason_eject",
        "restore original" => "history.reason_restore_original",
        _ if subject.starts_with("restore adoption ") => "history.reason_restore",
        _ if subject.starts_with("discard ") => "history.reason_discard",
        _ => {
            let shell = subject
                .strip_prefix("remove ")
                .and_then(|rest| rest.strip_suffix(" hook"));
            return match shell {
                Some(shell) => tf("history.reason_remove_hook", &[("shell", shell)]),
                None => subject.to_string().into(),
            };
        }
    };
    t(key)
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
    use super::{reason, saved};
    use crate::text::t;

    #[test]
    fn snapshot_reasons_are_named_in_the_interface_language() {
        assert_eq!(reason("adopt"), t("history.reason_adopt"));
        assert_eq!(reason("discard .gitconfig"), t("history.reason_discard"));
        assert!(reason("remove zsh hook").contains("zsh"));
        assert_ne!(reason("remove zsh hook"), "remove zsh hook");
        // One this app does not know is shown as it is.
        assert_eq!(reason("something new"), "something new");
    }

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
