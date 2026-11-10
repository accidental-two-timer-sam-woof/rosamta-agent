//! Dotfiles declared in `dotfiles.toml` and deployed into the home
//! directory (ADR 0007).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::config::{Deploy, Dotfile, DotfileManifest, HostFacts};
use crate::error::{Error, Result};
use crate::validate;

pub mod deploy;
mod history;
pub mod state;
pub mod tree;

pub use deploy::{Action, Step, deploy, plan};

pub const MANIFEST: &str = "dotfiles.toml";

/// `path` as plain relative components; `None` when it is absolute,
/// empty, or steps out with `..`.
pub fn clean(path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// The repo's declarations; none when the file does not exist.
pub fn load(repo_dir: &Path) -> Result<DotfileManifest> {
    let path = repo_dir.join(MANIFEST);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(err) => return Err(Error::Read { path, source: err }),
    };
    let manifest: DotfileManifest = validate::parse(&text).map_err(|issue| {
        Error::Usage(format!(
            "{}: {}; run `rsmt validate` for details",
            path.display(),
            issue.message
        ))
    })?;
    if let Some(problem) = check(&manifest, repo_dir).into_iter().next() {
        return Err(Error::Usage(format!("{}: {problem}", path.display())));
    }
    Ok(manifest)
}

/// Problems `rsmt validate` reports, one message each.
pub fn check(manifest: &DotfileManifest, repo_dir: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let mut paths = Vec::new();
    for entry in &manifest.dotfile {
        let name = &entry.path;
        match clean(name) {
            Some(path) => paths.push(path),
            None => problems.push(format!(
                "dotfile `{name}`: path must be relative to the home directory, without `..`"
            )),
        }
        let source = entry.source();
        match clean(&source) {
            Some(rel) if rel.starts_with(".git") => problems.push(format!(
                "dotfile `{name}`: source `{source}` is inside .git"
            )),
            Some(rel) if !repo_dir.join(&rel).exists() => problems.push(format!(
                "dotfile `{name}`: source `{source}` does not exist"
            )),
            Some(_) => {}
            None => problems.push(format!(
                "dotfile `{name}`: source must be relative to the repo, without `..`"
            )),
        }
        if entry.permissions.is_some() && entry.mode().is_none() {
            problems.push(format!(
                "dotfile `{name}`: permissions must be an octal mode such as \"0600\""
            ));
        }
        if entry.encrypted && entry.deploy == Deploy::Link {
            problems.push(format!(
                "dotfile `{name}`: an encrypted dotfile cannot be a link; set deploy = \"copy\""
            ));
        }
    }
    paths.sort();
    paths.dedup();
    for outer in &paths {
        for inner in paths.iter().filter(|inner| *inner != outer) {
            if inner.starts_with(outer) {
                problems.push(format!(
                    "dotfile `{}` is inside dotfile `{}`",
                    inner.display(),
                    outer.display()
                ));
            }
        }
    }
    problems
}

/// What a path uses on this host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Use(Dotfile),
    /// This many declarations match and none names this host alone.
    Duplicate(usize),
}

/// The declaration each path uses on this host. Among several that match,
/// the one naming this host ID in `when.hosts` wins.
pub fn select(manifest: &DotfileManifest, host: &HostFacts) -> BTreeMap<PathBuf, Choice> {
    let mut matching: BTreeMap<PathBuf, Vec<&Dotfile>> = BTreeMap::new();
    for entry in manifest
        .dotfile
        .iter()
        .filter(|entry| entry.when.matches(host))
    {
        if let Some(path) = clean(&entry.path) {
            matching.entry(path).or_default().push(entry);
        }
    }
    matching
        .into_iter()
        .map(|(path, entries)| {
            let named: Vec<&Dotfile> = entries
                .iter()
                .copied()
                .filter(|entry| entry.when.hosts.contains(&host.id))
                .collect();
            let choice = match (entries.as_slice(), named.as_slice()) {
                ([only], _) | (_, [only]) => Choice::Use((*only).clone()),
                (_, []) => Choice::Duplicate(entries.len()),
                (_, named) => Choice::Duplicate(named.len()),
            };
            (path, choice)
        })
        .collect()
}

#[cfg(test)]
mod tests;
