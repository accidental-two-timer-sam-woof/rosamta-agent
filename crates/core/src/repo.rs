//! The shared repo: layout, format version, creating and joining.

use std::path::{Path, PathBuf};

use crate::config::{CURRENT_FORMAT, RepoManifest};
use crate::edit::{edit_toml, set_value};
use crate::error::{Error, Result};
use crate::git::Commit;
use crate::paths::Paths;
use crate::{git, host, validate};

pub const MANIFEST: &str = "rosamta.toml";
pub const MODULES: &str = "modules";

/// Every `*.toml` under `modules/`, subdirectories included, sorted. Hidden
/// entries, such as an editor's lock file, are skipped.
pub fn module_files(repo_dir: &Path) -> Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir).map_err(Error::read(dir))? {
            let entry = entry.map_err(Error::read(dir))?;
            if entry.file_name().as_encoded_bytes().starts_with(b".") {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path.is_file() && path.extension().is_some_and(|ext| ext == "toml") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    let dir = repo_dir.join(MODULES);
    if dir.is_dir() {
        walk(&dir, &mut files)?;
    }
    files.sort();
    Ok(files)
}

/// Subdirectories only organize files; the name is the file stem.
pub fn module_name(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn read_manifest(repo_dir: &Path) -> Result<RepoManifest> {
    let path = repo_dir.join(MANIFEST);
    let text = std::fs::read_to_string(&path).map_err(Error::read(&path))?;
    validate::parse(&text).map_err(|issue| {
        let line = issue
            .line
            .map(|line| format!(":{line}"))
            .unwrap_or_default();
        Error::Usage(format!("{}{line}: {}", path.display(), issue.message))
    })
}

/// Refuse a repo written by a newer rosamta rather than misread it.
pub fn ensure_supported(format: u32) -> Result<()> {
    ensure_supported_by(format, CURRENT_FORMAT)
}

fn ensure_supported_by(format: u32, supported: u32) -> Result<()> {
    if format > supported {
        return Err(Error::Usage(format!(
            "this repo uses format {format}, but this rosamta supports up to {supported}; upgrade rosamta"
        )));
    }
    Ok(())
}

/// Turns a repo of format `from` into format `from + 1`.
pub struct Migration {
    pub from: u32,
    pub run: fn(&Path) -> Result<()>,
}

pub const MIGRATIONS: &[Migration] = &[];

/// Bring the repo up to `target`, one step at a time, and record it in one
/// commit. Returns the formats migrated from and to.
pub fn migrate(
    repo_dir: &Path,
    target: u32,
    migrations: &[Migration],
) -> Result<Option<(u32, u32)>> {
    let from = read_manifest(repo_dir)?.format;
    if from >= target {
        return Ok(None);
    }
    for step in from..target {
        let migration = migrations
            .iter()
            .find(|migration| migration.from == step)
            .ok_or_else(|| Error::Usage(format!("no migration from repo format {step}")))?;
        (migration.run)(repo_dir)?;
    }
    edit_toml(&repo_dir.join(MANIFEST), |doc| {
        set_value(&mut doc["format"], i64::from(target))
    })?;
    git::commit_all(
        repo_dir,
        &format!("chore(repo): migrate format {from} to {target}"),
    )?;
    Ok(Some((from, target)))
}

/// Add `id` to the host list and commit. Returns whether it was missing.
pub fn register_host(repo_dir: &Path, id: &str) -> Result<bool> {
    if read_manifest(repo_dir)?.hosts.iter().any(|host| host == id) {
        return Ok(false);
    }
    edit_toml(&repo_dir.join(MANIFEST), |doc| {
        match doc["hosts"].as_array_mut() {
            Some(hosts) => hosts.push(id),
            None => set_value(&mut doc["hosts"], toml_edit::Array::from_iter([id])),
        }
    })?;
    commit_files(
        repo_dir,
        &format!("chore(repo): add host {id}"),
        &[MANIFEST],
    )?;
    Ok(true)
}

/// Commit only `files`, relative to the repo, so edits made by hand stay out.
pub(crate) fn commit_files(repo_dir: &Path, message: &str, files: &[&str]) -> Result<bool> {
    if files.is_empty() {
        return Ok(false);
    }
    match git::commit(repo_dir, message, files)? {
        Commit::Clean => Ok(false),
        Commit::Created => Ok(true),
        Commit::SigningFailed(message) => Err(Error::Git {
            command: "commit".into(),
            message,
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InitOutcome {
    pub repo: PathBuf,
    pub host_id: String,
    pub host_created: bool,
    /// The ID generated for this host, when another host in the repo has it.
    pub host_id_taken: Option<String>,
    pub cloned: bool,
    /// The repo was already here; only this host was set up.
    pub existing: bool,
    pub migrated: Option<(u32, u32)>,
}

/// Refuse a repo that has `rosamta.toml`; a directory without it, such as
/// one left by an interrupted init, is not a repo yet.
pub(crate) fn refuse_existing(repo_dir: &Path) -> Result<()> {
    if repo_dir.join(MANIFEST).exists() {
        return Err(Error::Usage(format!(
            "a repo already exists at {}",
            repo_dir.display()
        )));
    }
    Ok(())
}

/// Remove the repo directory if `result` failed, so a retry starts clean.
fn or_remove<T>(repo_dir: &Path, result: Result<T>) -> Result<T> {
    if result.is_err() {
        let _ = std::fs::remove_dir_all(repo_dir);
    }
    result
}

/// `rsmt init`: a new repo holding only `rosamta.toml`, or, when the repo is
/// here but `host.toml` is not, this host set up for it. Touches no rc file.
pub fn init_new(paths: &Paths) -> Result<InitOutcome> {
    let repo_dir = paths.repo();
    if repo_dir.join(MANIFEST).is_file() && !paths.host_config().exists() {
        return add_this_host(paths);
    }
    refuse_existing(&repo_dir)?;
    let (host, generated) = host::load_or_create(paths, &[])?;
    // Never remove a directory this init did not create.
    let fresh = !repo_dir.exists();
    std::fs::create_dir_all(&repo_dir).map_err(Error::write(&repo_dir))?;
    let result = (|| {
        git::run(&repo_dir, &["init", "--quiet"])?;
        let manifest = RepoManifest {
            format: CURRENT_FORMAT,
            hosts: vec![host.id.clone()],
        };
        let text = toml::to_string(&manifest).expect("RepoManifest serializes");
        std::fs::write(repo_dir.join(MANIFEST), text)
            .map_err(Error::write(repo_dir.join(MANIFEST)))?;
        commit_files(&repo_dir, "chore(repo): initialize", &[MANIFEST])
    })();
    if fresh {
        or_remove(&repo_dir, result)
    } else {
        result
    }?;
    Ok(InitOutcome {
        repo: repo_dir,
        host_id: host.id,
        host_created: generated.is_some(),
        host_id_taken: None,
        cloned: false,
        existing: false,
        migrated: None,
    })
}

/// The repo is here but `host.toml` is not: create it and add this host.
fn add_this_host(paths: &Paths) -> Result<InitOutcome> {
    let repo_dir = paths.repo();
    ensure_supported(read_manifest(&repo_dir)?.format)?;
    // An ID already in this local repo's list is most likely this host's own.
    let (host, _) = host::load_or_create(paths, &[])?;
    register_host(&repo_dir, &host.id)?;
    Ok(InitOutcome {
        repo: repo_dir,
        host_id: host.id,
        host_created: true,
        host_id_taken: None,
        cloned: false,
        existing: true,
        migrated: None,
    })
}

/// `rsmt init <url>`: clone an existing repo and register this host.
/// Touches no rc file or dotfile until the user applies.
pub fn join(paths: &Paths, url: &str) -> Result<InitOutcome> {
    join_with(paths, url, &[], CURRENT_FORMAT, MIGRATIONS)
}

/// [`join`] with `git -c` settings for the clone, such as a credential helper.
pub(crate) fn join_with_config(paths: &Paths, url: &str, config: &[&str]) -> Result<InitOutcome> {
    join_with(paths, url, config, CURRENT_FORMAT, MIGRATIONS)
}

pub(crate) fn join_with(
    paths: &Paths,
    url: &str,
    config: &[&str],
    target: u32,
    migrations: &[Migration],
) -> Result<InitOutcome> {
    let repo_dir = paths.repo();
    refuse_existing(&repo_dir)?;
    if let Some(parent) = repo_dir.parent() {
        std::fs::create_dir_all(parent).map_err(Error::write(parent))?;
    }
    git::clone(url, &repo_dir, config)?;
    or_remove(
        &repo_dir,
        (|| {
            if !repo_dir.join(MANIFEST).is_file() {
                return Err(Error::Usage(format!(
                    "{url} is not a rosamta repo: missing {MANIFEST}"
                )));
            }
            let manifest = read_manifest(&repo_dir)?;
            ensure_supported_by(manifest.format, target)?;
            let (host, generated) = host::load_or_create(paths, &manifest.hosts)?;
            let migrated = migrate(&repo_dir, target, migrations)?;
            register_host(&repo_dir, &host.id)?;
            let host_created = generated.is_some();
            Ok(InitOutcome {
                repo: repo_dir.clone(),
                host_id_taken: generated.filter(|id| *id != host.id),
                host_id: host.id,
                host_created,
                cloned: true,
                existing: false,
                migrated,
            })
        })(),
    )
}

/// The URL of the remote a sync uses, if the repo has one.
pub fn remote(repo_dir: &Path) -> Option<String> {
    let name = crate::sync::remote_name(repo_dir)?;
    git::run(repo_dir, &["remote", "get-url", &name])
        .ok()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

/// A commit as status screens list it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub at: u64,
    pub hash: String,
    pub subject: String,
    /// Paths it changed, relative to the repo.
    pub files: Vec<String>,
}

/// The newest `count` commits of the git repo at `dir`; none when it has none.
pub fn recent_commits(dir: &Path, count: usize) -> Vec<CommitInfo> {
    let count = format!("--max-count={count}");
    let format = "--format=%x1e%ct%x09%h%x09%s";
    let Ok(text) = git::run(dir, &["log", &count, "--name-only", format]) else {
        return Vec::new();
    };
    text.split('\x1e')
        .filter_map(|entry| {
            let mut lines = entry.lines();
            let mut parts = lines.next()?.splitn(3, '\t');
            Some(CommitInfo {
                at: parts.next()?.parse().ok()?,
                hash: parts.next()?.to_string(),
                subject: parts.next()?.to_string(),
                files: lines
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect(),
            })
        })
        .collect()
}
