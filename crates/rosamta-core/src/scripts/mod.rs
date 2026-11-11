//! Scripts declared in `scripts.toml`: run on a host once, or whenever
//! they or what they watch change.

use std::collections::BTreeMap;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::When;
use crate::dotfiles::clean;
use crate::error::{Error, Result};
use crate::validate;

pub const MANIFEST: &str = "scripts.toml";

/// `scripts.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScriptManifest {
    #[serde(default)]
    pub script: Vec<Script>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// The script, relative to the repo root. It names the script: renamed,
    /// it is a new one.
    pub source: String,
    pub run: Run,
    #[serde(default)]
    pub timing: Timing,
    /// Repo paths whose changes count as the script's own; onchange only.
    #[serde(default)]
    pub watch: Vec<String>,
    #[serde(default)]
    pub when: When,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Run {
    /// Once on each host, until it succeeds.
    Once,
    /// Whenever the script or what it watches changed since it last succeeded.
    Onchange,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Timing {
    /// Before shells and dotfiles change.
    Before,
    /// After dotfiles deploy.
    #[default]
    After,
}

/// The repo's declarations as written; none when the file does not exist.
pub fn load(repo_dir: &Path) -> Result<ScriptManifest> {
    let path = repo_dir.join(MANIFEST);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(err) => return Err(Error::Read { path, source: err }),
    };
    validate::parse(&text).map_err(|issue| {
        Error::Usage(format!(
            "{}: {}; run `rsmt validate` for details",
            path.display(),
            issue.message
        ))
    })
}

/// The declarations applying can use, and every other one with its first
/// problem.
pub fn usable(repo_dir: &Path) -> Result<(ScriptManifest, Vec<(Script, String)>)> {
    let mut manifest = load(repo_dir)?;
    let mut first: BTreeMap<usize, String> = BTreeMap::new();
    for (index, problem) in problems(&manifest, repo_dir) {
        first.entry(index).or_insert(problem);
    }
    let (mut kept, mut broken) = (Vec::new(), Vec::new());
    for (index, entry) in std::mem::take(&mut manifest.script).into_iter().enumerate() {
        match first.remove(&index) {
            Some(problem) => broken.push((entry, problem)),
            None => kept.push(entry),
        }
    }
    manifest.script = kept;
    Ok((manifest, broken))
}

/// Problems `rsmt validate` reports, one message each.
pub fn check(manifest: &ScriptManifest, repo_dir: &Path) -> Vec<String> {
    problems(manifest, repo_dir)
        .into_iter()
        .map(|(_, problem)| problem)
        .collect()
}

/// A repo path that exists and is not inside `.git`.
fn repo_path(repo_dir: &Path, path: &str) -> std::result::Result<(), &'static str> {
    match clean(path) {
        None => Err("must be relative to the repo, without `..`"),
        Some(rel) if rel.starts_with(".git") => Err("is inside .git"),
        Some(rel) if std::fs::symlink_metadata(repo_dir.join(&rel)).is_err() => {
            Err("does not exist")
        }
        Some(_) => Ok(()),
    }
}

fn problems(manifest: &ScriptManifest, repo_dir: &Path) -> Vec<(usize, String)> {
    let mut problems = Vec::new();
    for (index, entry) in manifest.script.iter().enumerate() {
        let mut problem = |text: String| problems.push((index, text));
        let name = &entry.source;
        match repo_path(repo_dir, name) {
            Err(why) => problem(format!("script `{name}` {why}")),
            Ok(()) if !repo_dir.join(name).is_file() => {
                problem(format!("script `{name}` is not a file"))
            }
            Ok(()) => {}
        }
        if manifest.script[..index]
            .iter()
            .any(|other| clean(&other.source) == clean(name))
        {
            problem(format!("script `{name}` is declared more than once"));
        }
        if entry.run == Run::Once && !entry.watch.is_empty() {
            problem(format!("script `{name}`: `watch` is for onchange scripts"));
        }
        for watched in &entry.watch {
            if let Err(why) = repo_path(repo_dir, watched) {
                problem(format!("script `{name}`: watched `{watched}` {why}"));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests;
