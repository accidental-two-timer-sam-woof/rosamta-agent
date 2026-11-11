//! Content the repo no longer has, from the commit before it was deleted:
//! another host forgot a linked dotfile, and this host's link would dangle.

use std::path::Path;

use super::tree::{self, Content};
use crate::error::{Error, Result};
use crate::git;

/// Write `source` (relative to the repo) as it was before its deletion to `to`.
pub fn recover(repo: &Path, source: &str, to: &Path) -> Result<()> {
    let log = ["log", "-1", "--format=%H", "--diff-filter=D", "--", source];
    let commit = git::run(repo, &log)?.trim().to_string();
    if commit.is_empty() {
        return Err(Error::Usage(format!(
            "`{source}` is gone and not in the repo history"
        )));
    }
    let parent = to.parent().expect("a target has a parent");
    let name = to
        .file_name()
        .expect("a target has a name")
        .to_string_lossy();
    let scratch = parent.join(format!(".{name}.rosamta-history"));
    tree::remove(&scratch)?;
    std::fs::create_dir_all(&scratch).map_err(Error::write(&scratch))?;
    let work_tree = format!("--work-tree={}", scratch.display());
    let from = format!("--source={commit}^");
    let restored = git::run(
        repo,
        &[&work_tree, "restore", &from, "--worktree", "--", source],
    )
    .and_then(|_| tree::put(Content::Tree(&scratch.join(source)), to));
    tree::remove(&scratch)?;
    restored
}
