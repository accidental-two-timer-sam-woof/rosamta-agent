//! Publish, Pull and Sync between the repo and its remote. A failure at any
//! step leaves what this host already applied untouched.

mod scan;
mod state;

use std::path::Path;

use serde::Serialize;

use crate::adopt::{self, Lock};
use crate::apply::{self, Applied, Context};
use crate::config::{RepoManifest, SyncPolicy};
use crate::error::{Error, Result};
use crate::git::{self, Commit, Failure};
use crate::{generated, host, repo, validate};

pub use scan::Hit;
pub use state::{Attempt, Outcome, SyncState, load as load_state};

#[derive(Debug, Default, Serialize)]
pub struct Published {
    /// Edits made outside rosamta were committed first.
    pub committed: bool,
    pub pushed: u32,
    /// The remote had new commits, so they were pulled first.
    pub pulled: Option<Pulled>,
}

#[derive(Debug, Default, Serialize)]
pub struct Pulled {
    pub commits: u32,
    /// `None` when nothing came in or this host has never applied.
    pub applied: Option<Applied>,
}

pub fn publish(ctx: &Context) -> Result<Published> {
    let _lock = adopt::lock(ctx)?;
    publish_locked(ctx, true)
}

pub fn pull(ctx: &Context) -> Result<Pulled> {
    let _lock = adopt::lock(ctx)?;
    pull_locked(ctx, true)
}

/// Pull, then Publish.
pub fn sync(ctx: &Context) -> Result<Published> {
    let _lock = adopt::lock(ctx)?;
    sync_locked(ctx)
}

fn sync_locked(ctx: &Context) -> Result<Published> {
    let pulled = pull_locked(ctx, true)?;
    let mut published = publish_locked(ctx, true)?;
    if pulled.commits > 0 {
        published.pulled = Some(pulled);
    }
    Ok(published)
}

/// The background service's Sync: with the publish policy `manual`, only
/// Pull, leave edits the user has not committed alone and count unpushed
/// commits.
pub fn sync_per_policy(ctx: &Context) -> Result<()> {
    let _lock = adopt::lock(ctx)?;
    if host::load(&ctx.paths.host_config())?.publish == SyncPolicy::Auto {
        return sync_locked(ctx).map(drop);
    }
    pull_locked(ctx, false)?;
    let repo_dir = ctx.paths.repo();
    let unpushed = Remote::required(&repo_dir)?.unpushed(&repo_dir)?;
    state::update(ctx.paths.state(), |state| state.unpushed = unpushed)
}

/// After rosamta committed to the repo: push when the publish policy is
/// `auto` and the repo has a remote; otherwise only count unpushed commits.
/// Edits the user has not committed are left alone.
pub fn publish_if_auto(ctx: &Context, _held: &Lock) -> Result<Option<Published>> {
    let repo_dir = ctx.paths.repo();
    let Some(remote) = Remote::find(&repo_dir)? else {
        return Ok(None);
    };
    if host::load(&ctx.paths.host_config())?.publish == SyncPolicy::Manual {
        let unpushed = remote.unpushed(&repo_dir)?;
        state::update(ctx.paths.state(), |state| state.unpushed = unpushed)?;
        return Ok(None);
    }
    publish_locked(ctx, false).map(Some)
}

/// Record that a commit could not be signed, from any command that commits.
pub(crate) fn signing_blocked(ctx: &Context, message: &str) -> Result<()> {
    record_publish(ctx, Attempt::new(Outcome::Blocked, Some(signing(message))))
}

/// Record a commit that failed for any other reason, such as a rebase in progress.
pub(crate) fn commit_blocked(ctx: &Context, err: &Error) -> Result<()> {
    record_publish(
        ctx,
        Attempt::new(Outcome::Blocked, Some(format!("commit failed: {err}"))),
    )
}

fn signing(message: &str) -> String {
    format!("commit signing failed: {message}")
}

/// The remote and branch this repo publishes to.
struct Remote {
    name: String,
    branch: String,
    /// `origin/main`, once the branch tracks one.
    upstream: Option<String>,
}

impl Remote {
    fn find(repo_dir: &Path) -> Result<Option<Self>> {
        if !repo_dir.join(".git").exists() {
            return Err(Error::Usage(format!(
                "no repo at {}; run `rsmt init` first",
                repo_dir.display()
            )));
        }
        let branch = git::run(repo_dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .map_err(|_| Error::Usage("the repo is not on a branch".into()))?
            .trim()
            .to_string();
        let upstream = git::run(
            repo_dir,
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
        )
        .ok()
        .map(|name| name.trim().to_string());
        let configured = git::run(repo_dir, &["config", &format!("branch.{branch}.remote")])
            .ok()
            .map(|name| name.trim().to_string());
        let remotes = git::run(repo_dir, &["remote"])?;
        let name = configured.or_else(|| {
            let mut names = remotes.lines();
            names
                .clone()
                .find(|name| *name == "origin")
                .or_else(|| names.next())
                .map(str::to_string)
        });
        Ok(name.map(|name| Self {
            name,
            branch,
            upstream,
        }))
    }

    fn required(repo_dir: &Path) -> Result<Self> {
        Self::find(repo_dir)?.ok_or_else(|| Error::Usage("the repo has no remote".into()))
    }

    /// `git log` arguments for the commits the remote does not have.
    fn unpushed_range(&self) -> Vec<String> {
        match &self.upstream {
            Some(upstream) => vec![format!("{upstream}..HEAD")],
            None => vec![
                "HEAD".into(),
                "--not".into(),
                format!("--remotes={}", self.name),
            ],
        }
    }

    fn unpushed(&self, repo_dir: &Path) -> Result<u32> {
        count(repo_dir, &self.unpushed_range())
    }

    /// Track `<remote>/<branch>` once it exists, as after a first fetch.
    fn track(&mut self, repo_dir: &Path) -> Result<()> {
        let candidate = format!("{}/{}", self.name, self.branch);
        let exists = git::succeeds(
            repo_dir,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/remotes/{candidate}"),
            ],
        )?;
        if self.upstream.is_none() && exists {
            git::run(
                repo_dir,
                &["branch", "--quiet", "--set-upstream-to", &candidate],
            )?;
            self.upstream = Some(candidate);
        }
        Ok(())
    }
}

fn count(repo_dir: &Path, range: &[String]) -> Result<u32> {
    let mut args = vec!["rev-list", "--count"];
    args.extend(range.iter().map(String::as_str));
    Ok(git::run(repo_dir, &args)?.trim().parse().unwrap_or(0))
}

fn record_publish(ctx: &Context, attempt: Attempt) -> Result<()> {
    state::update(ctx.paths.state(), |state| state.publish = Some(attempt))
}

fn record_pull(ctx: &Context, attempt: Attempt) -> Result<()> {
    state::update(ctx.paths.state(), |state| state.pull = Some(attempt))
}

/// Record a failed git call against the remote and turn it into an error.
fn remote_failure(
    ctx: &Context,
    is_pull: bool,
    remote: &Remote,
    unpushed: u32,
    message: String,
) -> Error {
    let (outcome, text) = match git::classify(&message) {
        Failure::Offline => (
            Outcome::Offline,
            format!("cannot reach {}: {message}", remote.name),
        ),
        Failure::Denied => (
            Outcome::Blocked,
            format!("{} refused access: {message}", remote.name),
        ),
        _ => (Outcome::Failed, message),
    };
    let attempt = Attempt::new(outcome, Some(text.clone()));
    let recorded = state::update(ctx.paths.state(), |state| {
        state.unpushed = unpushed;
        if is_pull {
            state.pull = Some(attempt);
        } else {
            state.publish = Some(attempt);
        }
    });
    match recorded {
        Err(err) => err,
        Ok(()) if outcome == Outcome::Offline && unpushed > 0 => {
            let commits = if unpushed == 1 { "commit" } else { "commits" };
            Error::Sync(format!(
                "{text}\n{unpushed} {commits} will be pushed on the next sync"
            ))
        }
        Ok(()) => Error::Sync(text),
    }
}

fn publish_locked(ctx: &Context, commit_edits: bool) -> Result<Published> {
    let repo_dir = ctx.paths.repo();
    let mut remote = Remote::required(&repo_dir)?;
    let mut published = Published {
        committed: commit_edits && commit_pending(ctx, &repo_dir)?,
        ..Published::default()
    };
    // A push the remote rejects means another host pushed first: pull, then
    // push once more.
    for retry in [false, true] {
        let pushed = remote.unpushed(&repo_dir)?;
        if pushed == 0 {
            break;
        }
        let range = remote.unpushed_range();
        let range: Vec<&str> = range.iter().map(String::as_str).collect();
        let hits = scan::scan(&repo_dir, &range)?;
        if !hits.is_empty() {
            return Err(secret_blocked(ctx, &hits, Some(pushed)));
        }
        let refspec = format!("HEAD:refs/heads/{}", remote.branch);
        let mut args = vec!["push", "--quiet"];
        if remote.upstream.is_none() {
            args.push("--set-upstream");
        }
        args.extend([remote.name.as_str(), refspec.as_str()]);
        match git::run(&repo_dir, &args) {
            Ok(_) => published.pushed = pushed,
            Err(Error::Git { message, .. })
                if !retry && git::classify(&message) == Failure::Rejected =>
            {
                published.pulled = Some(pull_locked(ctx, commit_edits)?);
                remote = Remote::required(&repo_dir)?;
                continue;
            }
            Err(Error::Git { message, .. }) => {
                return Err(remote_failure(ctx, false, &remote, pushed, message));
            }
            Err(err) => return Err(err),
        }
        break;
    }
    let unpushed = Remote::required(&repo_dir)?.unpushed(&repo_dir)?;
    state::update(ctx.paths.state(), |state| {
        state.publish = Some(Attempt::ok());
        state.unpushed = unpushed;
    })?;
    Ok(published)
}

/// `unpushed` is `None` when the secret is in changes not committed yet.
fn secret_blocked(ctx: &Context, hits: &[Hit], unpushed: Option<u32>) -> Error {
    let lines: Vec<String> = hits
        .iter()
        .map(|hit| format!("  {}:{} ({})", hit.file, hit.line, hit.rule))
        .collect();
    let text = format!(
        "publishing stopped: possible secret in\n{}",
        lines.join("\n")
    );
    let mut attempt = Attempt::new(Outcome::Blocked, Some(text.clone()));
    attempt.files = hits.iter().map(|hit| hit.file.clone()).collect();
    attempt.files.sort();
    attempt.files.dedup();
    match state::update(ctx.paths.state(), |state| {
        state.publish = Some(attempt);
        if let Some(unpushed) = unpushed {
            state.unpushed = unpushed;
        }
    }) {
        Ok(()) => Error::Sync(text),
        Err(err) => err,
    }
}

/// Commit edits made outside rosamta, named after what changed. A secret
/// in them stops this before it is committed.
fn commit_pending(ctx: &Context, _repo_dir: &Path) -> Result<bool> {
    commit_scanned(ctx, &[], None)
}

/// Stage the changes under `paths` (every change when empty) and commit
/// exactly what the secret scan passed; `message` defaults to one naming
/// what changed. A commit that fails, to sign or otherwise, is recorded and
/// returned as an error.
pub(crate) fn commit_scanned(ctx: &Context, paths: &[&str], message: Option<&str>) -> Result<bool> {
    let repo_dir = ctx.paths.repo();
    git::ensure_on_branch(&repo_dir)?;
    let limit = |args: &[&'static str]| {
        let mut all: Vec<&str> = args.to_vec();
        if !paths.is_empty() {
            all.push("--");
            all.extend_from_slice(paths);
        }
        all
    };
    let status = git::run(
        &repo_dir,
        &limit(&[
            "-c",
            "core.quotePath=false",
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--no-renames",
        ]),
    )?;
    if status.trim().is_empty() {
        return Ok(false);
    }
    git::run(&repo_dir, &limit(&["add", "--all"]))?;
    let hits = scan::staged(&repo_dir)?;
    if !hits.is_empty() {
        git::run(&repo_dir, &["reset", "--quiet"])?;
        return Err(secret_blocked(ctx, &hits, None));
    }
    let message = message.map_or_else(|| edit_message(&status), str::to_string);
    match git::commit_staged(&repo_dir, &message, paths) {
        Ok(Commit::Clean) => Ok(false),
        Ok(Commit::Created) => Ok(true),
        Ok(Commit::SigningFailed(message)) => {
            signing_blocked(ctx, &message)?;
            Err(Error::Sync(signing(&message)))
        }
        Err(err) => {
            record_publish(ctx, Attempt::new(Outcome::Failed, Some(err.to_string())))?;
            Err(err)
        }
    }
}

/// `edit(modules): add bun, update zoxide` from `git status --porcelain`.
fn edit_message(status: &str) -> String {
    let changes: Vec<(&str, String)> = status
        .lines()
        .filter(|line| line.len() > 3)
        .map(|line| {
            let path = line[3..].trim_matches('"');
            let verb = match &line[..2] {
                code if code.contains('D') => "remove",
                code if code == "??" || code.contains('A') => "add",
                _ => "update",
            };
            (verb, path.to_string())
        })
        .collect();
    let in_modules = |path: &str| path.starts_with(&format!("{}/", repo::MODULES));
    let in_home = |path: &str| {
        path.starts_with("home/") || path.starts_with("hosts/") || path == crate::dotfiles::MANIFEST
    };
    let scope = if changes.iter().all(|(_, path)| in_modules(path)) {
        "modules"
    } else if changes.iter().all(|(_, path)| in_home(path)) {
        "home"
    } else {
        "repo"
    };
    if changes.len() > 3 {
        return format!("edit({scope}): change {} files", changes.len());
    }
    let name = |path: &str| {
        if in_modules(path) {
            repo::module_name(Path::new(path))
        } else {
            path.strip_prefix("home/").unwrap_or(path).to_string()
        }
    };
    let parts: Vec<String> = ["add", "update", "remove"]
        .into_iter()
        .filter_map(|verb| {
            let names: Vec<String> = changes
                .iter()
                .filter(|(change, _)| *change == verb)
                .map(|(_, path)| name(path))
                .collect();
            (!names.is_empty()).then(|| format!("{verb} {}", names.join(", ")))
        })
        .collect();
    format!("edit({scope}): {}", parts.join(", "))
}

/// `commit_edits`: commit edits the user has not committed before the
/// rebase; otherwise they are set aside during it and put back.
fn pull_locked(ctx: &Context, commit_edits: bool) -> Result<Pulled> {
    let repo_dir = ctx.paths.repo();
    // A rebase cut short (the process was killed) must not stay half done.
    if git::rebasing(&repo_dir) {
        git::run(&repo_dir, &["rebase", "--abort"])?;
    }
    let mut remote = Remote::required(&repo_dir)?;
    if let Err(Error::Git { message, .. }) =
        git::run(&repo_dir, &["fetch", "--quiet", &remote.name])
    {
        let unpushed = remote.unpushed(&repo_dir)?;
        return Err(remote_failure(ctx, true, &remote, unpushed, message));
    }
    remote.track(&repo_dir)?;
    let Some(upstream) = remote.upstream.clone() else {
        record_pull(ctx, Attempt::ok())?;
        return Ok(Pulled::default());
    };
    let commits = count(&repo_dir, &[format!("HEAD..{upstream}")])?;
    if commits == 0 {
        record_pull(ctx, Attempt::ok())?;
        return Ok(Pulled::default());
    }
    if let Err(err) = check_format(&repo_dir, &upstream) {
        record_pull(ctx, Attempt::new(Outcome::Failed, Some(err.to_string())))?;
        return Err(err);
    }
    let committed = if commit_edits {
        let head = git::run(&repo_dir, &["rev-parse", "HEAD"])?;
        commit_pending(ctx, &repo_dir)
            .map_err(|err| edits_blocked(ctx, err))?
            .then_some(head)
    } else {
        check_edits(ctx, &repo_dir, &upstream)?;
        None
    };
    if let Err(err) = rebase(ctx, &repo_dir, &upstream) {
        // What was committed above goes back to being edits.
        if let Some(head) = committed {
            git::run(&repo_dir, &["reset", "--quiet", head.trim()])?;
        }
        return Err(err);
    }
    let unpushed = remote.unpushed(&repo_dir)?;
    state::update(ctx.paths.state(), |state| state.unpushed = unpushed)?;
    // A host that never applied keeps its shells untouched until it does.
    if generated::current_id(&ctx.paths.generated()).is_none() {
        record_pull(ctx, Attempt::ok())?;
        return Ok(Pulled {
            commits,
            applied: None,
        });
    }
    // Judged against how shells start now, so only new errors stop it.
    match apply::apply_after(ctx, || Ok(())).and_then(|applied| apply::with_dotfiles(ctx, applied))
    {
        Ok(applied) => {
            record_pull(ctx, Attempt::ok())?;
            Ok(Pulled {
                commits,
                applied: Some(applied),
            })
        }
        Err(err) => {
            record_pull(
                ctx,
                Attempt {
                    applying: true,
                    ..Attempt::new(Outcome::Failed, Some(err.to_string()))
                },
            )?;
            Err(err)
        }
    }
}

/// Refuse what a newer rosamta wrote before it reaches the working tree.
fn check_format(repo_dir: &Path, upstream: &str) -> Result<()> {
    let spec = format!("{upstream}:{}", repo::MANIFEST);
    let text = git::run(repo_dir, &["show", &spec])?;
    let manifest: RepoManifest = validate::parse(&text)
        .map_err(|issue| Error::Usage(format!("{spec}: {}", issue.message)))?;
    repo::ensure_supported(manifest.format)
}

/// Pull stopped because the edits could not be committed. An `Error::Sync`
/// was recorded for Publish already, and stops Pull for the same reason.
fn edits_blocked(ctx: &Context, err: Error) -> Error {
    let recorded = state::update(ctx.paths.state(), |state| {
        state.pull = match &err {
            Error::Sync(_) => state.publish.clone(),
            other => Some(Attempt::new(Outcome::Failed, Some(other.to_string()))),
        }
    });
    match recorded {
        Ok(()) => err,
        Err(recording) => recording,
    }
}

/// Edits set aside during the rebase would conflict when put back on files
/// the remote changed too: stop before the rebase instead.
fn check_edits(ctx: &Context, repo_dir: &Path, upstream: &str) -> Result<()> {
    let edited = git::run(repo_dir, &["diff", "--name-only", "-z", "HEAD"])?;
    let incoming = format!("HEAD...{upstream}");
    let incoming = git::run(repo_dir, &["diff", "--name-only", "-z", &incoming])?;
    let edited: Vec<&str> = edited.split('\0').collect();
    let files: Vec<String> = incoming
        .split('\0')
        .filter(|file| !file.is_empty() && edited.contains(file))
        .map(str::to_string)
        .collect();
    if files.is_empty() {
        return Ok(());
    }
    Err(conflict(ctx, files))
}

/// Rebase local commits onto `upstream`, setting edits not committed aside
/// meanwhile; on any failure, abort so the repo is exactly as before.
fn rebase(ctx: &Context, repo_dir: &Path, upstream: &str) -> Result<()> {
    let Err(err) = git::run(repo_dir, &["rebase", "--quiet", "--autostash", upstream]) else {
        return Ok(());
    };
    let files: Vec<String> = git::run(repo_dir, &["diff", "--name-only", "--diff-filter=U"])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    git::run(repo_dir, &["rebase", "--abort"])?;
    if !files.is_empty() {
        return Err(conflict(ctx, files));
    }
    let message = match &err {
        Error::Git { message, .. } => message.clone(),
        other => other.to_string(),
    };
    let (outcome, text) = if git::classify(&message) == Failure::Signing {
        (Outcome::Blocked, signing(&message))
    } else {
        (Outcome::Failed, message)
    };
    record_pull(ctx, Attempt::new(outcome, Some(text.clone())))?;
    Err(Error::Sync(text))
}

/// Record that Pull stopped on files changed both here and on the remote.
fn conflict(ctx: &Context, files: Vec<String>) -> Error {
    let text = format!(
        "pull stopped: changed both here and on the remote:\n  {}",
        files.join("\n  ")
    );
    let mut attempt = Attempt::new(Outcome::Conflict, Some(text.clone()));
    attempt.files = files;
    match record_pull(ctx, attempt) {
        Ok(()) => Error::Sync(text),
        Err(err) => err,
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;
    use crate::paths::Paths;

    /// With `publish = "manual"` the background service's sync only pulls.
    #[test]
    fn manual_publish_policy_keeps_periodic_sync_to_a_pull() {
        git::tests::isolate();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let ctx = Context {
            paths: Paths::from_vars(|name| (name == "HOME").then(|| OsString::from(&home)))
                .unwrap(),
            home: home.clone(),
            path_var: String::new(),
            interactive: false,
        };
        std::fs::create_dir_all(ctx.paths.host_config().parent().unwrap()).unwrap();
        std::fs::write(
            ctx.paths.host_config(),
            "id = \"a\"\npublish = \"manual\"\n",
        )
        .unwrap();
        repo::init_new(&ctx.paths).unwrap();
        let repo_dir = ctx.paths.repo();
        let bare = dir.path().join("remote.git");
        let bare_url = bare.to_str().unwrap();
        git::run(dir.path(), &["init", "--quiet", "--bare", "remote.git"]).unwrap();
        git::run(&repo_dir, &["remote", "add", "origin", bare_url]).unwrap();
        publish(&ctx).unwrap();
        // Another host pushes a module.
        let other = dir.path().join("other");
        git::clone(bare_url, &other, &[]).unwrap();
        std::fs::create_dir_all(other.join("modules")).unwrap();
        std::fs::write(other.join("modules/b.toml"), "[env]\nB = \"1\"\n").unwrap();
        git::commit_all(&other, "b").unwrap();
        git::run(&other, &["push", "--quiet"]).unwrap();
        // This host commits a module, then edits it by hand.
        std::fs::create_dir_all(repo_dir.join("modules")).unwrap();
        std::fs::write(repo_dir.join("modules/a.toml"), "[env]\nA = \"1\"\n").unwrap();
        git::commit_all(&repo_dir, "a").unwrap();
        std::fs::write(repo_dir.join("modules/a.toml"), "[env]\nA = \"2\"\n").unwrap();
        let remote_head = git::run(&bare, &["rev-parse", "HEAD"]).unwrap();

        sync_per_policy(&ctx).unwrap();
        assert!(repo_dir.join("modules/b.toml").is_file());
        assert_eq!(
            git::run(&bare, &["rev-parse", "HEAD"]).unwrap(),
            remote_head
        );
        assert_eq!(
            git::run(&repo_dir, &["status", "--porcelain"]).unwrap(),
            " M modules/a.toml\n"
        );
        let state = load_state(ctx.paths.state()).unwrap();
        assert_eq!(state.unpushed, 1);
        assert_eq!(state.pull.unwrap().outcome, Outcome::Ok);
    }

    #[test]
    fn edit_messages_name_what_changed() {
        let cases = [
            (" M modules/zoxide.toml\n", "edit(modules): update zoxide"),
            (
                "?? modules/tools/bun.toml\n M modules/zoxide.toml\n",
                "edit(modules): add bun, update zoxide",
            ),
            (
                " D modules/old.toml\n M rosamta.toml\n",
                "edit(repo): update rosamta.toml, remove old",
            ),
            (
                " M modules/a.toml\n M modules/b.toml\n M modules/c.toml\n M modules/d.toml\n",
                "edit(modules): change 4 files",
            ),
            (
                " M home/.config/nvim/lazy-lock.json\n",
                "edit(home): update .config/nvim/lazy-lock.json",
            ),
            (
                " M dotfiles.toml\n?? hosts/mbp/.gitconfig\n",
                "edit(home): add hosts/mbp/.gitconfig, update dotfiles.toml",
            ),
        ];
        for (status, expected) in cases {
            assert_eq!(edit_message(status), expected);
        }
    }
}
