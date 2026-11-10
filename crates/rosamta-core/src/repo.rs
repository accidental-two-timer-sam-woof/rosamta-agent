//! The shared repo: layout, format version, creating and joining.

use std::path::{Path, PathBuf};

use crate::config::{CURRENT_FORMAT, RepoManifest};
use crate::edit::{edit_toml, set_value};
use crate::error::{Error, Result};
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
    git::commit_all(repo_dir, &format!("chore(repo): add host {id}"))?;
    Ok(true)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InitOutcome {
    pub repo: PathBuf,
    pub host_id: String,
    pub host_created: bool,
    pub cloned: bool,
    pub migrated: Option<(u32, u32)>,
}

pub(crate) fn refuse_existing(repo_dir: &Path) -> Result<()> {
    let occupied = std::fs::read_dir(repo_dir).is_ok_and(|mut entries| entries.next().is_some());
    if occupied {
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

/// `rsmt init`: a new repo holding only `rosamta.toml`. Touches no rc file.
pub fn init_new(paths: &Paths) -> Result<InitOutcome> {
    let repo_dir = paths.repo();
    refuse_existing(&repo_dir)?;
    let (host, host_created) = host::load_or_create(paths)?;
    std::fs::create_dir_all(&repo_dir).map_err(Error::write(&repo_dir))?;
    or_remove(
        &repo_dir,
        (|| {
            git::run(&repo_dir, &["init", "--quiet"])?;
            let manifest = RepoManifest {
                format: CURRENT_FORMAT,
                hosts: vec![host.id.clone()],
            };
            let text = toml::to_string(&manifest).expect("RepoManifest serializes");
            std::fs::write(repo_dir.join(MANIFEST), text)
                .map_err(Error::write(repo_dir.join(MANIFEST)))?;
            git::commit_all(&repo_dir, "chore(repo): initialize")
        })(),
    )?;
    Ok(InitOutcome {
        repo: repo_dir,
        host_id: host.id,
        host_created,
        cloned: false,
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
            ensure_supported_by(read_manifest(&repo_dir)?.format, target)?;
            let (host, host_created) = host::load_or_create(paths)?;
            let migrated = migrate(&repo_dir, target, migrations)?;
            register_host(&repo_dir, &host.id)?;
            Ok(InitOutcome {
                repo: repo_dir.clone(),
                host_id: host.id,
                host_created,
                cloned: true,
                migrated,
            })
        })(),
    )
}
