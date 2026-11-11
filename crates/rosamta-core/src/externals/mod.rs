//! External content declared in `externals.toml`: git repos and archives
//! downloaded into the home directory, every host getting the version the
//! repo locks.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;

use crate::config::{DotfileManifest, HostFacts, When};
use crate::dotfiles::{self, Choice, clean};
use crate::error::{Error, Result};
use crate::validate;

pub const MANIFEST: &str = "externals.toml";

/// `externals.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalManifest {
    #[serde(default)]
    pub external: Vec<External>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct External {
    /// Where it goes, relative to the home directory.
    pub path: String,
    /// A git repository, checked out at `rev`.
    #[serde(default)]
    pub git: Option<String>,
    /// The full commit `git` is locked to; `rsmt externals update` writes it.
    #[serde(default)]
    #[schemars(regex(pattern = r"^[0-9a-f]{40}([0-9a-f]{24})?$"))]
    pub rev: Option<String>,
    /// The branch `rsmt externals update` follows; the remote's default
    /// branch when unset.
    #[serde(default)]
    pub branch: Option<String>,
    /// A .tar.gz, .tar or .zip archive, extracted at `path`.
    #[serde(default)]
    pub archive: Option<String>,
    /// The archive's SHA-256; `rsmt externals update` writes it.
    #[serde(default)]
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"))]
    pub sha256: Option<String>,
    /// Leading directories dropped from each path in the archive.
    #[serde(default)]
    pub strip: u32,
    #[serde(default)]
    pub when: When,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Git,
    Archive,
}

impl External {
    /// `None` unless exactly one of `git` and `archive` is set.
    pub fn kind(&self) -> Option<Kind> {
        match (&self.git, &self.archive) {
            (Some(_), None) => Some(Kind::Git),
            (None, Some(_)) => Some(Kind::Archive),
            _ => None,
        }
    }

    pub fn url(&self) -> &str {
        self.git
            .as_deref()
            .or(self.archive.as_deref())
            .unwrap_or_default()
    }

    /// The version every host gets: `rev` or `sha256`.
    pub fn lock(&self) -> Option<&str> {
        match self.kind()? {
            Kind::Git => self.rev.as_deref(),
            Kind::Archive => self.sha256.as_deref(),
        }
    }
}

/// The repo's declarations as written; none when the file does not exist.
pub fn load(repo_dir: &Path) -> Result<ExternalManifest> {
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
pub fn usable(repo_dir: &Path) -> Result<(ExternalManifest, Vec<(External, String)>)> {
    let mut manifest = load(repo_dir)?;
    // A broken dotfiles.toml is reported on its own; nesting is then unchecked.
    let dotfiles = dotfiles::load(repo_dir).unwrap_or_default();
    let mut first: BTreeMap<usize, String> = BTreeMap::new();
    for (index, problem) in problems(&manifest, &dotfiles) {
        first.entry(index).or_insert(problem);
    }
    let (mut kept, mut broken) = (Vec::new(), Vec::new());
    for (index, entry) in std::mem::take(&mut manifest.external)
        .into_iter()
        .enumerate()
    {
        match first.remove(&index) {
            Some(problem) => broken.push((entry, problem)),
            None => kept.push(entry),
        }
    }
    manifest.external = kept;
    Ok((manifest, broken))
}

/// Problems `rsmt validate` reports, one message each.
pub fn check(manifest: &ExternalManifest, dotfiles: &DotfileManifest) -> Vec<String> {
    problems(manifest, dotfiles)
        .into_iter()
        .map(|(_, problem)| problem)
        .collect()
}

/// The declaration each path uses on this host.
pub fn select(
    manifest: &ExternalManifest,
    host: &HostFacts,
) -> BTreeMap<PathBuf, Choice<External>> {
    dotfiles::select_by(&manifest.external, |entry| (&entry.path, &entry.when), host)
}

fn is_hex(text: &str, lens: &[usize]) -> bool {
    lens.contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Each problem with the index of the declaration it belongs to. Paths may
/// nest only inside a git external, whose own files git tells apart.
fn problems(manifest: &ExternalManifest, dotfiles: &DotfileManifest) -> Vec<(usize, String)> {
    let mut problems = Vec::new();
    let paths: Vec<Option<PathBuf>> = manifest
        .external
        .iter()
        .map(|entry| clean(&entry.path))
        .collect();
    let dotfile_paths: Vec<PathBuf> = dotfiles
        .dotfile
        .iter()
        .filter_map(|entry| clean(&entry.path))
        .collect();
    for (index, entry) in manifest.external.iter().enumerate() {
        let mut problem = |text: String| problems.push((index, text));
        let name = &entry.path;
        let Some(path) = &paths[index] else {
            problem(format!(
                "external `{name}`: path must be relative to the home directory, without `..`"
            ));
            continue;
        };
        if let Some(file) = dotfiles::startup_file(path) {
            problem(format!(
                "external `{name}`: `{file}` is a shell startup file; keep shell settings in modules"
            ));
        }
        match entry.kind() {
            None if entry.git.is_some() => problem(format!(
                "external `{name}`: set `git` or `archive`, not both"
            )),
            None => problem(format!("external `{name}`: set `git` or `archive`")),
            Some(Kind::Git) => {
                if entry.sha256.is_some() || entry.strip != 0 {
                    problem(format!(
                        "external `{name}`: `sha256` and `strip` are for archives"
                    ));
                }
                // Read as an option by git otherwise.
                if entry.url().starts_with('-')
                    || entry.branch.as_deref().is_some_and(|b| b.starts_with('-'))
                {
                    problem(format!(
                        "external `{name}`: `git` and `branch` cannot start with `-`"
                    ));
                }
            }
            Some(Kind::Archive) => {
                if entry.rev.is_some() || entry.branch.is_some() {
                    problem(format!("external `{name}`: `rev` and `branch` are for git"));
                }
                if !entry.url().starts_with("https://") && !entry.url().starts_with("http://") {
                    problem(format!(
                        "external `{name}`: `archive` must be an http or https URL"
                    ));
                }
            }
        }
        match (entry.kind(), entry.lock()) {
            (None, _) => {}
            (Some(_), None) => problem(format!(
                "external `{name}` is not locked yet; run `rsmt externals update`"
            )),
            (Some(Kind::Git), Some(rev)) if !is_hex(rev, &[40, 64]) => problem(format!(
                "external `{name}`: `rev` must be a full commit hash"
            )),
            (Some(Kind::Archive), Some(sum)) if !is_hex(sum, &[64]) => problem(format!(
                "external `{name}`: `sha256` must be 64 lowercase hex digits"
            )),
            _ => {}
        }
        let is_git = entry.kind() == Some(Kind::Git);
        // The same path twice is a host variant, which selecting settles.
        for (other_index, other) in paths.iter().enumerate() {
            let Some(other) = other.as_ref().filter(|other| *other != path) else {
                continue;
            };
            if path.starts_with(other) && manifest.external[other_index].kind() != Some(Kind::Git) {
                problem(format!(
                    "external `{name}` is inside external `{}`, which is not a git repo",
                    other.display()
                ));
            }
        }
        for dotfile in &dotfile_paths {
            if path.starts_with(dotfile) {
                problem(format!(
                    "external `{name}` is inside dotfile `{}`",
                    dotfile.display()
                ));
            } else if dotfile.starts_with(path) && !is_git {
                problem(format!(
                    "dotfile `{}` is inside external `{name}`, which is not a git repo",
                    dotfile.display()
                ));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests;
