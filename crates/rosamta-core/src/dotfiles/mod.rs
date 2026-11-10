//! Dotfiles declared in `dotfiles.toml` and deployed into the home
//! directory (ADR 0007).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::config::{Deploy, Dotfile, DotfileManifest, HostFacts};
use crate::error::{Error, Result};
use crate::validate;

pub mod crypt;
pub mod deploy;
mod history;
pub mod manage;
pub mod review;
pub mod state;
pub mod tree;

pub use deploy::{Action, Step, deploy, plan};

pub const MANIFEST: &str = "dotfiles.toml";

/// Shell startup files in their usual places. Shell settings live in
/// modules (GLOSSARY: shell 環境不算 dotfile), and hooks and adopt rewrite
/// these files per host, which a shared copy in the repo would undo.
const STARTUP_FILES: &[&str] = &[
    ".profile",
    ".bash_profile",
    ".bash_login",
    ".bashrc",
    ".zshenv",
    ".zprofile",
    ".zshrc",
    ".zlogin",
    ".config/fish/config.fish",
    ".config/fish/conf.d",
];

/// The startup file `path` is or holds, if any.
fn startup_file(path: &Path) -> Option<&'static str> {
    STARTUP_FILES
        .iter()
        .copied()
        .find(|file| Path::new(file).starts_with(path))
}

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

/// The repo's declarations as written; none when the file does not exist.
pub fn load(repo_dir: &Path) -> Result<DotfileManifest> {
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

/// The declarations deploying can use, and every other one with its first
/// problem: one broken declaration leaves the rest deploying.
pub fn usable(repo_dir: &Path) -> Result<(DotfileManifest, Vec<(Dotfile, String)>)> {
    let mut manifest = load(repo_dir)?;
    let mut first: BTreeMap<usize, String> = BTreeMap::new();
    for (index, problem) in problems(&manifest, repo_dir) {
        first.entry(index).or_insert(problem);
    }
    let mut broken = Vec::new();
    let mut kept = Vec::new();
    for (index, entry) in std::mem::take(&mut manifest.dotfile)
        .into_iter()
        .enumerate()
    {
        match first.remove(&index) {
            Some(problem) => broken.push((entry, problem)),
            None => kept.push(entry),
        }
    }
    manifest.dotfile = kept;
    Ok((manifest, broken))
}

/// Problems `rsmt validate` reports, one message each.
pub fn check(manifest: &DotfileManifest, repo_dir: &Path) -> Vec<String> {
    problems(manifest, repo_dir)
        .into_iter()
        .map(|(_, problem)| problem)
        .collect()
}

/// Each problem with the index of the declaration it belongs to.
fn problems(manifest: &DotfileManifest, repo_dir: &Path) -> Vec<(usize, String)> {
    let mut problems = Vec::new();
    let paths: Vec<Option<PathBuf>> = manifest
        .dotfile
        .iter()
        .map(|entry| clean(&entry.path))
        .collect();
    for (index, entry) in manifest.dotfile.iter().enumerate() {
        let mut problem = |text: String| problems.push((index, text));
        let name = &entry.path;
        match &paths[index] {
            None => problem(format!(
                "dotfile `{name}`: path must be relative to the home directory, without `..`"
            )),
            Some(path) => {
                if let Some(file) = startup_file(path) {
                    problem(format!(
                        "dotfile `{name}`: `{file}` is a shell startup file; keep shell settings in modules"
                    ));
                }
            }
        }
        let source = entry.source();
        match clean(&source) {
            Some(rel) if rel.starts_with(".git") => problem(format!(
                "dotfile `{name}`: source `{source}` is inside .git"
            )),
            Some(rel) if !repo_dir.join(&rel).exists() => problem(format!(
                "dotfile `{name}`: source `{source}` does not exist"
            )),
            Some(_) => {}
            None => problem(format!(
                "dotfile `{name}`: source must be relative to the repo, without `..`"
            )),
        }
        if entry.permissions.is_some() && entry.mode().is_none() {
            problem(format!(
                "dotfile `{name}`: permissions must be an octal mode such as \"0600\""
            ));
        }
        if entry.encrypted && repo_dir.join(entry.source()).is_dir() {
            problem(format!(
                "dotfile `{name}`: an encrypted dotfile must be a file"
            ));
        }
        if entry.deploy == Deploy::Template {
            let path = repo_dir.join(&source);
            if path.is_dir() {
                problem(format!("dotfile `{name}`: a template must be a file"));
            } else if !entry.encrypted
                && let Ok(bytes) = std::fs::read(&path)
            {
                // Encrypted templates are checked when they are rendered.
                match String::from_utf8(bytes) {
                    Ok(text) => {
                        if let Err(err) = crate::template::check(&source, &text) {
                            problem(format!("dotfile `{name}`: {err}"));
                        }
                    }
                    Err(_) => problem(format!("dotfile `{name}`: a template must be UTF-8 text")),
                }
            }
        }
        if entry.encrypted && entry.deploy == Deploy::Link {
            problem(format!(
                "dotfile `{name}`: an encrypted dotfile cannot be a link; set deploy = \"copy\""
            ));
        }
        // The inner of two nested paths is the one left out.
        if let Some(inner) = &paths[index]
            && let Some(outer) = paths
                .iter()
                .flatten()
                .filter(|outer| *outer != inner)
                .find(|outer| inner.starts_with(outer))
        {
            problem(format!(
                "dotfile `{}` is inside dotfile `{}`",
                inner.display(),
                outer.display()
            ));
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

/// `vars.*` that this host's templates use and `host.toml` lacks, sorted.
/// Templates still encrypted for want of the key, and broken ones, which
/// `problems` reports, are left out.
pub fn missing_vars(ctx: &crate::apply::Context) -> Result<Vec<String>> {
    let host = crate::adopt::host_config(ctx)?;
    let repo_dir = ctx.paths.repo();
    let (manifest, _) = usable(&repo_dir)?;
    let mut missing = std::collections::BTreeSet::new();
    for (_, choice) in select(&manifest, &HostFacts::current(&host)) {
        let Choice::Use(entry) = choice else {
            continue;
        };
        if entry.deploy != Deploy::Template {
            continue;
        }
        let source = entry.source();
        let path = repo_dir.join(&source);
        let bytes = std::fs::read(&path).map_err(|err| Error::Read { path, source: err })?;
        let bytes = match entry.encrypted {
            false => bytes,
            true => match crypt::decrypt(ctx, &bytes)? {
                Some(bytes) => bytes,
                None => continue,
            },
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if let Ok(names) = crate::template::missing_vars(&source, &text, &host.vars) {
            missing.extend(names);
        }
    }
    Ok(missing.into_iter().collect())
}

/// Whether an encrypted dotfile of this host waits for the passphrase.
pub fn needs_key(ctx: &crate::apply::Context) -> Result<bool> {
    if !crypt::locked(ctx)? {
        return Ok(false);
    }
    let host = crate::adopt::host_config(ctx)?;
    Ok(
        select(&load(&ctx.paths.repo())?, &HostFacts::current(&host))
            .values()
            .any(|choice| matches!(choice, Choice::Use(entry) if entry.encrypted)),
    )
}

#[cfg(test)]
mod tests;
