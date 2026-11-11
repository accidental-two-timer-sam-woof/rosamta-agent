//! Git externals: a detached checkout of the locked commit, fetched with
//! the user's own git so its credentials apply.

use std::path::Path;

use crate::error::{Error, Result};
use crate::git::{run_foreign as run, succeeds};

pub(crate) fn head(dir: &Path) -> Option<String> {
    run(dir, &["rev-parse", "--verify", "--quiet", "HEAD"])
        .ok()
        .map(|out| out.trim().to_string())
}

pub(crate) fn origin(dir: &Path) -> Option<String> {
    run(dir, &["remote", "get-url", "origin"])
        .ok()
        .map(|out| out.trim().to_string())
}

/// The same repository however the URL ends.
pub(crate) fn same_url(a: &str, b: &str) -> bool {
    let trim = |url: &str| {
        url.trim_end_matches('/')
            .trim_end_matches(".git")
            .to_string()
    };
    trim(a) == trim(b)
}

/// No uncommitted changes to tracked files. Untracked files do not count:
/// checking out leaves them, and plugins inside oh-my-zsh are untracked.
pub(crate) fn is_clean(dir: &Path) -> Result<bool> {
    run(dir, &["status", "--porcelain", "--untracked-files=no"]).map(|out| out.trim().is_empty())
}

/// Fetch `rev` unless it is already here: by itself when the server allows
/// it, else everything.
pub(crate) fn fetch(dir: &Path, rev: &str) -> Result<()> {
    if succeeds(dir, &["cat-file", "-e", &format!("{rev}^{{commit}}")])? {
        return Ok(());
    }
    if run(dir, &["fetch", "--quiet", "--depth", "1", "origin", rev]).is_ok() {
        return Ok(());
    }
    run(dir, &["fetch", "--quiet", "origin"])?;
    if succeeds(dir, &["cat-file", "-e", &format!("{rev}^{{commit}}")])? {
        Ok(())
    } else {
        Err(Error::External(format!(
            "the repository has no commit {rev}"
        )))
    }
}

pub(crate) fn checkout(dir: &Path, rev: &str) -> Result<()> {
    run(dir, &["checkout", "--quiet", "--detach", rev]).map(drop)
}

/// A new checkout of `rev` at `into`, which must not exist.
pub(crate) fn clone(url: &str, rev: &str, into: &Path) -> Result<()> {
    std::fs::create_dir(into).map_err(Error::write(into))?;
    run(into, &["init", "--quiet"])?;
    run(into, &["remote", "add", "origin", url])?;
    fetch(into, rev)?;
    checkout(into, rev)
}

pub(crate) fn set_origin(dir: &Path, url: &str) -> Result<()> {
    run(dir, &["remote", "set-url", "origin", url]).map(drop)
}

/// The commit `branch`, or the remote's default branch, is at now.
pub(crate) fn remote_head(dir: &Path, url: &str, branch: Option<&str>) -> Result<String> {
    let reference = branch.map_or_else(|| "HEAD".to_string(), |b| format!("refs/heads/{b}"));
    let out = run(dir, &["ls-remote", url, &reference])?;
    out.split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| Error::External(format!("{url} has no {reference}")))
}
