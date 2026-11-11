//! What needs the user on this host: one list for the background service's
//! notifications and for the app.

use std::path::PathBuf;

use crate::apply::Context;
use crate::config::{HostConfig, SyncPolicy};
use crate::i18n::{self, Lang};
use crate::sync::{self, Attempt, Outcome};
use crate::{adopt, dotfiles};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Names its text: `problem.<kind>`.
    pub kind: &'static str,
    /// What the text is about: files, a count, or the reason's first line.
    pub arg: String,
    /// The publish or pull behind it.
    pub attempt: Option<Attempt>,
}

impl Problem {
    pub fn new(kind: &'static str, arg: String) -> Self {
        Problem {
            kind,
            arg,
            attempt: None,
        }
    }

    /// The same for as long as the problem lasts. Being offline is one
    /// problem however many commits wait.
    pub fn key(&self) -> String {
        if self.kind == "offline" {
            "offline:".to_string()
        } else {
            format!("{}:{}", self.kind, self.arg)
        }
    }

    pub fn text(&self, lang: Lang) -> String {
        let (text, arg) = match self.kind {
            "conflict" => ("problem.conflict", "files"),
            "hooks_missing" => ("problem.hooks_missing", "files"),
            "offline" if self.arg == "1" => ("problem.offline_one", "count"),
            "offline" => ("problem.offline", "count"),
            kind => (&*format!("problem.{kind}"), "reason"),
        };
        i18n::text(lang, text, &[(arg, &self.arg)])
    }
}

/// Hooks that are gone, encrypted dotfiles without their key, and the
/// publish and pull that need the user.
pub fn current(ctx: &Context, host: &HostConfig, missing_hooks: &[PathBuf]) -> Vec<Problem> {
    let mut problems = Vec::new();
    if !missing_hooks.is_empty() {
        let files: Vec<String> = missing_hooks
            .iter()
            .map(|path| adopt::display(path, &ctx.home))
            .collect();
        problems.push(Problem::new("hooks_missing", files.join(", ")));
    }
    if dotfiles::needs_key(ctx).unwrap_or(false) {
        problems.push(Problem::new("locked", String::new()));
    }
    let Ok(state) = sync::load_state(ctx.paths.state()) else {
        return problems;
    };
    // Edits that could not be committed stop the pull with the publish
    // attempt itself: one problem, reported as publishing.
    let pull = state
        .pull
        .as_ref()
        .filter(|pull| state.publish.as_ref() != Some(*pull));
    for (attempt, pull) in [(state.publish.as_ref(), false), (pull, true)] {
        let Some(attempt) = attempt else {
            continue;
        };
        let reason = attempt.reason.clone().unwrap_or_default();
        let first = reason.lines().next().unwrap_or_default().to_string();
        let (kind, arg) = match (attempt.outcome, pull) {
            (Outcome::Offline, false) if host.publish == SyncPolicy::Auto && state.unpushed > 0 => {
                ("offline", state.unpushed.to_string())
            }
            (Outcome::Ok | Outcome::Offline, _) => continue,
            (Outcome::Conflict, _) => ("conflict", attempt.files.join(", ")),
            (Outcome::Failed, _) if reason.contains("upgrade rosamta") => {
                ("upgrade", String::new())
            }
            (Outcome::Failed, true) if attempt.applying => ("apply_failed", first),
            (Outcome::Failed, true) => ("pull_failed", first),
            (Outcome::Failed, false) => ("publish_failed", first),
            (Outcome::Blocked, true) => ("pull_blocked", first),
            (Outcome::Blocked, false) => ("publish_blocked", first),
        };
        // Pull and publish can stop for the same reason.
        if problems
            .iter()
            .any(|known: &Problem| known.kind == kind && known.arg == arg)
        {
            continue;
        }
        problems.push(Problem {
            kind,
            arg,
            attempt: Some(attempt.clone()),
        });
    }
    problems
}
