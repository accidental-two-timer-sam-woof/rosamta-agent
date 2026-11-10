//! Calls to the system `git`, so the user's own config (signing, credentials) applies.

use std::path::Path;
use std::process::{Command, Output};

use crate::error::{Error, Result};

pub(crate) fn run(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output();
    check(args, output)
}

pub(crate) fn clone(url: &str, dest: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["clone", "--quiet", url])
        .arg(dest)
        .output();
    check(&["clone", url], output).map(drop)
}

/// Stage everything and commit; does nothing when the tree is clean.
pub(crate) fn commit_all(dir: &Path, message: &str) -> Result<bool> {
    run(dir, &["add", "--all"])?;
    if run(dir, &["status", "--porcelain"])?.trim().is_empty() {
        return Ok(false);
    }
    run(dir, &["commit", "--quiet", "--message", message])?;
    Ok(true)
}

fn check(args: &[&str], output: std::io::Result<Output>) -> Result<String> {
    let command = args.join(" ");
    let output = output.map_err(|err| Error::Git {
        command: command.clone(),
        message: err.to_string(),
    })?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::Git { command, message });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
