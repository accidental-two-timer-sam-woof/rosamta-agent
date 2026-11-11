//! Putting each external of this host at its locked version, never over
//! what the host changed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::state::{self, Downloaded, Record};
use super::{External, Kind, archive, git, select, usable};
use crate::apply::Context;
use crate::config::HostFacts;
use crate::dotfiles::{Choice, tree};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Downloaded where nothing was.
    Downloaded,
    /// Moved to the locked version.
    Updated,
    Unchanged,
    /// Changed on this host since it was downloaded; left alone.
    Changed,
    /// Something else is at the path; left alone.
    Conflict,
    Skipped,
    Failed,
    /// No longer declared: left in place, no longer managed.
    Released,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    /// Relative to the home directory.
    pub path: PathBuf,
    pub action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

enum Work {
    Nothing,
    Download(External),
    /// Fetch and check out in place, first pointing `origin` at the
    /// declared URL when it changed.
    Checkout(External, bool),
    /// Replace an unchanged archive with the new version.
    Replace(External),
    /// Take over a clone that is already at `rev`.
    Record(External),
    Drop,
}

struct Planned {
    step: Step,
    work: Work,
}

fn planned(path: &Path, action: Action, reason: Option<&str>, work: Work) -> Planned {
    Planned {
        step: Step {
            path: path.to_path_buf(),
            action,
            reason: reason.map(str::to_string),
        },
        work,
    }
}

/// What applying would do, without downloading anything.
pub fn plan(ctx: &Context) -> Result<Vec<Step>> {
    Ok(plan_all(ctx)?.into_iter().map(|item| item.step).collect())
}

/// Download or update every external of this host. A problem with one is
/// reported in its step and does not stop the others. The caller holds the
/// adopt lock.
pub fn apply(ctx: &Context) -> Result<Vec<Step>> {
    let planned = plan_all(ctx)?;
    let mut records = state::load(ctx.paths.state())?;
    let gits = git_paths(ctx)?;
    let mut steps = Vec::new();
    for item in planned {
        let mut step = item.step;
        let target = ctx.home.join(&step.path);
        // Parents sort first; one that is not there holds back what goes inside.
        let waiting = matches!(item.work, Work::Download(_) | Work::Replace(_))
            .then(|| missing_parent(ctx, &gits, &step.path))
            .flatten();
        if let Some(outer) = waiting {
            step.action = Action::Skipped;
            step.reason = Some(format!("waits for external `{}`", outer.display()));
        } else if let Err(err) = run(&target, &step.path, item.work, &mut records) {
            step.action = Action::Failed;
            step.reason = Some(err.to_string());
        }
        steps.push(step);
    }
    state::save(ctx.paths.state(), &records)?;
    Ok(steps)
}

/// The git externals of this host, by path.
pub(crate) fn git_paths(ctx: &Context) -> Result<Vec<PathBuf>> {
    let host = crate::adopt::host_config(ctx)?;
    let (manifest, _) = usable(&ctx.paths.repo())?;
    Ok(select(&manifest, &HostFacts::current(&host))
        .into_iter()
        .filter(
            |(_, choice)| matches!(choice, Choice::Use(entry) if entry.kind() == Some(Kind::Git)),
        )
        .map(|(path, _)| path)
        .collect())
}

/// The git external around `path` that is not checked out, if any.
pub(crate) fn missing_parent(ctx: &Context, gits: &[PathBuf], path: &Path) -> Option<PathBuf> {
    gits.iter()
        .filter(|outer| path != *outer && path.starts_with(outer))
        .find(|outer| !ctx.home.join(outer).join(".git").exists())
        .cloned()
}

fn plan_all(ctx: &Context) -> Result<Vec<Planned>> {
    let host = crate::adopt::host_config(ctx)?;
    let (manifest, broken) = usable(&ctx.paths.repo())?;
    let records = state::load(ctx.paths.state())?;
    let mut failed = BTreeMap::new();
    for (entry, problem) in broken {
        let path =
            crate::dotfiles::clean(&entry.path).unwrap_or_else(|| PathBuf::from(&entry.path));
        failed.entry(path).or_insert(problem);
    }
    let selected = select(&manifest, &HostFacts::current(&host));
    let mut out = Vec::new();
    for (path, problem) in &failed {
        out.push(planned(path, Action::Failed, Some(problem), Work::Nothing));
    }
    for (path, choice) in selected
        .iter()
        .filter(|(path, _)| !failed.contains_key(*path))
    {
        out.push(match choice {
            Choice::Duplicate(count) => planned(
                path,
                Action::Skipped,
                Some(&format!("declared {count} times for this host")),
                Work::Nothing,
            ),
            Choice::Use(entry) => {
                let target = ctx.home.join(path);
                let record = records.entries.get(path);
                plan_one(&target, path, entry, record).unwrap_or_else(|err| {
                    planned(path, Action::Failed, Some(&err.to_string()), Work::Nothing)
                })
            }
        });
    }
    for path in records.entries.keys() {
        if !selected.contains_key(path) && !failed.contains_key(path) {
            out.push(planned(path, Action::Released, None, Work::Drop));
        }
    }
    out.sort_by(|a, b| a.step.path.cmp(&b.step.path));
    Ok(out)
}

fn plan_one(
    target: &Path,
    path: &Path,
    entry: &External,
    record: Option<&Record>,
) -> Result<Planned> {
    if std::fs::symlink_metadata(target).is_err() {
        return Ok(planned(
            path,
            Action::Downloaded,
            None,
            Work::Download(entry.clone()),
        ));
    }
    let lock = entry.lock().unwrap_or_default();
    if entry.kind() == Some(Kind::Archive) {
        let Some(record) = record else {
            return Ok(planned(
                path,
                Action::Conflict,
                Some("something else is there"),
                Work::Nothing,
            ));
        };
        if record.lock == lock {
            return Ok(planned(path, Action::Unchanged, None, Work::Nothing));
        }
        return Ok(if tree::hash(target)? == record.tree {
            planned(path, Action::Updated, None, Work::Replace(entry.clone()))
        } else {
            planned(
                path,
                Action::Changed,
                Some("changed on this host"),
                Work::Nothing,
            )
        });
    }
    if !target.join(".git").exists() {
        return Ok(planned(
            path,
            Action::Conflict,
            Some("something else is there"),
            Work::Nothing,
        ));
    }
    let origin = git::origin(target).unwrap_or_default();
    let declared = git::same_url(&origin, entry.url());
    // The declared URL changed since rosamta checked this out.
    let moved = !declared && record.is_some_and(|record| git::same_url(&origin, &record.url));
    if !declared && !moved {
        return Ok(planned(
            path,
            Action::Conflict,
            Some(&format!("a clone of {origin} is there")),
            Work::Nothing,
        ));
    }
    let head = git::head(target);
    if declared && head.as_deref() == Some(lock) {
        let work = match record {
            Some(record) if record.lock == lock && record.url == entry.url() => Work::Nothing,
            _ => Work::Record(entry.clone()),
        };
        return Ok(planned(path, Action::Unchanged, None, work));
    }
    if !git::is_clean(target)? {
        return Ok(planned(
            path,
            Action::Changed,
            Some("has uncommitted changes"),
            Work::Nothing,
        ));
    }
    // The lock did not change, so something else moved HEAD, such as the
    // tool updating itself.
    let reason = record
        .is_some_and(|record| record.lock == lock && head.is_some())
        .then_some("moved back to the locked commit");
    Ok(planned(
        path,
        Action::Updated,
        reason,
        Work::Checkout(entry.clone(), moved),
    ))
}

fn run(target: &Path, path: &Path, work: Work, records: &mut Downloaded) -> Result<()> {
    let entry = match work {
        Work::Nothing => return Ok(()),
        Work::Drop => {
            records.entries.remove(path);
            return Ok(());
        }
        Work::Record(entry) => entry,
        Work::Checkout(entry, moved) => {
            let rev = entry.lock().unwrap_or_default();
            if moved {
                git::set_origin(target, entry.url())?;
            }
            git::fetch(target, rev)?;
            git::checkout(target, rev)?;
            entry
        }
        Work::Download(entry) => {
            let new = Aside::new(target, "new")?;
            fetch_into(&entry, new.path())?;
            std::fs::rename(new.path(), target).map_err(Error::write(target))?;
            entry
        }
        Work::Replace(entry) => {
            let new = Aside::new(target, "new")?;
            fetch_into(&entry, new.path())?;
            let old = Aside::new(target, "old")?;
            std::fs::rename(target, old.path()).map_err(Error::write(target))?;
            std::fs::rename(new.path(), target).map_err(Error::write(target))?;
            entry
        }
    };
    let tree = match entry.kind() {
        Some(Kind::Archive) => tree::hash(target)?,
        _ => None,
    };
    records.entries.insert(
        path.to_path_buf(),
        Record {
            url: entry.url().to_string(),
            lock: entry.lock().unwrap_or_default().to_string(),
            tree,
        },
    );
    Ok(())
}

/// Download `entry` into `into`, which does not exist yet.
fn fetch_into(entry: &External, into: &Path) -> Result<()> {
    let lock = entry.lock().unwrap_or_default();
    if entry.kind() == Some(Kind::Git) {
        return git::clone(entry.url(), lock, into);
    }
    let file = Aside::new(into, "download")?;
    let sum = archive::download(entry.url(), file.path())?;
    if sum != lock {
        return Err(Error::External(format!(
            "the download's SHA-256 is {sum}, not the locked {lock}"
        )));
    }
    std::fs::create_dir(into).map_err(Error::write(into))?;
    archive::extract(file.path(), entry.strip, into)
}

/// A path beside `target`, on the same file system so a rename is atomic,
/// cleared before use and removed when dropped. One left by an interrupted
/// apply is cleared the next time.
struct Aside(PathBuf);

impl Aside {
    fn new(target: &Path, purpose: &str) -> Result<Self> {
        let parent = target.parent().unwrap_or(target);
        std::fs::create_dir_all(parent).map_err(Error::write(parent))?;
        let name = target.file_name().unwrap_or_default().to_string_lossy();
        let path = parent.join(format!(".{name}.rosamta-{purpose}"));
        tree::remove(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Aside {
    fn drop(&mut self) {
        let _ = tree::remove(&self.0);
    }
}
