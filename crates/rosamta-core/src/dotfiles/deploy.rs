//! Put each declared dotfile in place, or tell why not.
//!
//! The deploy record separates "the repo changed" from "this host changed":
//! a target untouched since the last deploy follows the repo; a target
//! changed while the repo stayed is written back to the repo (links always,
//! copies by the adopt policy); both changed is a conflict nobody overwrites.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::review::{self, DotfileItem, Reason};
use super::state::{self, Deployed, Record, Sides};
use super::tree::{self, Content};
use super::{Choice, crypt, history};

/// Why an encrypted dotfile waits.
pub(crate) const LOCKED: &str = "encrypted; the key is not unlocked on this host";
use crate::apply::Context;
use crate::config::{AdoptPolicy, Deploy, Dotfile, HostFacts};
use crate::error::{Error, Result};
use crate::host;
use crate::snapshot::Snapshots;

/// What happened, or would happen, at one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Already as declared.
    Unchanged,
    /// Now a symlink to the repo.
    Linked,
    /// Copied where nothing was.
    Copied,
    /// The copy follows a change in the repo.
    Updated,
    /// What this host had was written back to the repo.
    Saved,
    /// No longer declared for this host; left in place as a regular file.
    Released,
    /// Differs from the repo, and both sides changed: waits for the user.
    Conflict,
    /// Changed on this host and the adopt policy asks: waits for the user.
    Changed,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    /// Relative to the home directory.
    pub path: PathBuf,
    pub action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A step and the work it takes.
pub struct Planned {
    pub step: Step,
    /// The current and the new content, for a preview of a copy that
    /// changes; `None` on a side that is not text or not there.
    pub preview: Option<(Option<String>, Option<String>)>,
    /// Both sides of a path that waits for the user.
    sides: Option<Sides>,
    work: Work,
}

enum Work {
    /// Nothing on disk; refresh the record and permissions.
    Keep(Dotfile),
    Link(Dotfile),
    Write(Dotfile, Desired),
    /// Write the host's content back to the repo, then deploy it again.
    Save(Dotfile),
    /// Turn a link into a regular copy of what it showed.
    Unlink(Record),
    /// Stop tracking; nothing on disk changes.
    Drop,
    Nothing,
}

/// The content a copy should have.
enum Desired {
    Source(PathBuf),
    Bytes(Vec<u8>),
}

impl Desired {
    fn hash(&self) -> Result<Option<String>> {
        match self {
            Desired::Source(path) => tree::hash(path),
            Desired::Bytes(bytes) => Ok(Some(tree::hash_bytes(bytes))),
        }
    }

    fn content(&self) -> Content<'_> {
        match self {
            Desired::Source(path) => Content::Tree(path),
            Desired::Bytes(bytes) => Content::Bytes(bytes),
        }
    }

    fn text(&self) -> Option<String> {
        match self {
            Desired::Source(path) => std::fs::read_to_string(path).ok(),
            Desired::Bytes(bytes) => String::from_utf8(bytes.clone()).ok(),
        }
    }
}

struct Env<'a> {
    ctx: &'a Context,
    repo: PathBuf,
    policy: AdoptPolicy,
    deployed: Deployed,
}

impl Env<'_> {
    fn target(&self, path: &Path) -> PathBuf {
        self.ctx.home.join(path)
    }

    fn source(&self, entry: &Dotfile) -> PathBuf {
        self.repo.join(entry.source())
    }

    fn ignored(&self, path: &Path, local: &str, repo: &str) -> bool {
        self.deployed
            .ignored
            .get(path)
            .is_some_and(|sides| sides.local == local && sides.repo == repo)
    }
}

fn step(path: &Path, action: Action, reason: Option<String>, work: Work) -> Planned {
    Planned {
        step: Step {
            path: path.to_path_buf(),
            action,
            reason,
        },
        preview: None,
        sides: None,
        work,
    }
}

/// A step that waits for the user, with both sides as they are.
fn wait(path: &Path, action: Action, local: String, repo: String) -> Planned {
    let mut planned = step(path, action, None, Work::Nothing);
    planned.sides = Some(Sides { local, repo });
    planned
}

/// Work out what deploying would do, without writing anything.
pub fn plan(ctx: &Context) -> Result<Vec<Planned>> {
    let repo = ctx.paths.repo();
    let (manifest, broken) = super::usable(&repo)?;
    let host = host::load(&ctx.paths.host_config())?;
    let facts = HostFacts::current(&host);
    let selected = super::select(&manifest, &facts);
    // A path with a broken declaration for this host is neither deployed
    // nor released: fixing the declaration picks up where it was.
    let mut failed: BTreeMap<PathBuf, String> = BTreeMap::new();
    for (entry, problem) in broken
        .into_iter()
        .filter(|(entry, _)| entry.when.matches(&facts))
    {
        let path = super::clean(&entry.path).unwrap_or_else(|| PathBuf::from(&entry.path));
        failed.entry(path).or_insert(problem);
    }
    let env = Env {
        ctx,
        repo,
        policy: host.adopt,
        deployed: state::load(ctx.paths.state())?,
    };
    let mut out = Vec::new();
    for (path, problem) in &failed {
        out.push(step(
            path,
            Action::Failed,
            Some(problem.clone()),
            Work::Nothing,
        ));
    }
    for (path, choice) in selected
        .iter()
        .filter(|(path, _)| !failed.contains_key(*path))
    {
        let planned = match choice {
            Choice::Duplicate(count) => step(
                path,
                Action::Skipped,
                Some(format!("declared {count} times for this host")),
                Work::Nothing,
            ),
            Choice::Use(entry) => plan_entry(&env, path, entry).unwrap_or_else(|err| {
                step(path, Action::Failed, Some(err.to_string()), Work::Nothing)
            }),
        };
        out.push(planned);
    }
    for (path, record) in &env.deployed.entries {
        if !selected.contains_key(path) && !failed.contains_key(path) {
            out.push(plan_release(&env, path, record));
        }
    }
    Ok(out)
}

fn plan_entry(env: &Env, path: &Path, entry: &Dotfile) -> Result<Planned> {
    match entry.deploy {
        Deploy::Template => Ok(step(
            path,
            Action::Skipped,
            Some("template deploy is not supported yet".into()),
            Work::Nothing,
        )),
        Deploy::Link => plan_link(env, path, entry),
        Deploy::Copy => plan_copy(env, path, entry),
    }
}

fn source_hash(env: &Env, entry: &Dotfile) -> Result<String> {
    let source = env.source(entry);
    tree::hash(&source)?.ok_or_else(|| {
        Error::Usage(format!(
            "source `{}` does not exist; run `rsmt validate`",
            entry.source()
        ))
    })
}

fn plan_link(env: &Env, path: &Path, entry: &Dotfile) -> Result<Planned> {
    let target = env.target(path);
    let source = env.source(entry);
    let repo_hash = source_hash(env, entry)?;
    let link = tree::link_of(&target);
    if link.as_deref() == Some(source.as_path()) {
        return Ok(step(
            path,
            Action::Unchanged,
            None,
            Work::Keep(entry.clone()),
        ));
    }
    let Some(local) = tree::hash(&target)? else {
        return Ok(step(path, Action::Linked, None, Work::Link(entry.clone())));
    };
    if local == repo_hash {
        return Ok(step(path, Action::Linked, None, Work::Link(entry.clone())));
    }
    if env.ignored(path, &local, &repo_hash) {
        return Ok(ignored(path));
    }
    let record = env.deployed.entries.get(path);
    // An editor that saves by renaming replaces the link with a file.
    let replaced = link.is_none()
        && record
            .is_some_and(|record| record.deploy == Deploy::Link && record.source_hash == repo_hash);
    if replaced {
        return Ok(step(path, Action::Saved, None, Work::Save(entry.clone())));
    }
    Ok(wait(path, Action::Conflict, local, repo_hash))
}

fn plan_copy(env: &Env, path: &Path, entry: &Dotfile) -> Result<Planned> {
    let target = env.target(path);
    let repo_hash = source_hash(env, entry)?;
    let local = tree::hash(&target)?;
    let record = env.deployed.entries.get(path);
    let untouched = record.is_some_and(|record| {
        record.deploy == Deploy::Copy && Some(&record.target_hash) == local.as_ref()
    });
    if untouched && record.is_some_and(|record| record.source_hash == repo_hash) {
        return Ok(step(
            path,
            Action::Unchanged,
            None,
            Work::Keep(entry.clone()),
        ));
    }
    let Some(desired) = desired(env.ctx, entry)? else {
        return Ok(step(
            path,
            Action::Skipped,
            Some(LOCKED.into()),
            Work::Nothing,
        ));
    };
    let Some(local) = local else {
        return Ok(write(path, entry, desired, Action::Copied, None));
    };
    if desired.hash()?.as_ref() == Some(&local) {
        return Ok(step(
            path,
            Action::Unchanged,
            None,
            Work::Keep(entry.clone()),
        ));
    }
    if untouched {
        let before = std::fs::read_to_string(&target).ok();
        return Ok(write(path, entry, desired, Action::Updated, Some(before)));
    }
    if env.ignored(path, &local, &repo_hash) {
        return Ok(ignored(path));
    }
    if record.is_some_and(|record| record.source_hash == repo_hash) {
        return Ok(match env.policy {
            AdoptPolicy::Auto => step(path, Action::Saved, None, Work::Save(entry.clone())),
            AdoptPolicy::Ask => wait(path, Action::Changed, local, repo_hash),
            AdoptPolicy::Off => step(
                path,
                Action::Skipped,
                Some("changed on this host".into()),
                Work::Nothing,
            ),
        });
    }
    Ok(wait(path, Action::Conflict, local, repo_hash))
}

/// What a copy should hold; `None` when it is encrypted and this host has
/// no key yet.
fn desired(ctx: &Context, entry: &Dotfile) -> Result<Option<Desired>> {
    let source = ctx.paths.repo().join(entry.source());
    if !entry.encrypted {
        return Ok(Some(Desired::Source(source)));
    }
    let sealed = std::fs::read(&source).map_err(Error::read(&source))?;
    Ok(crypt::decrypt(ctx, &sealed)?.map(Desired::Bytes))
}

fn write(
    path: &Path,
    entry: &Dotfile,
    desired: Desired,
    action: Action,
    before: Option<Option<String>>,
) -> Planned {
    let after = desired.text();
    let mut planned = step(path, action, None, Work::Write(entry.clone(), desired));
    planned.preview = Some((before.flatten(), after));
    planned
}

fn ignored(path: &Path) -> Planned {
    step(path, Action::Skipped, Some("ignored".into()), Work::Nothing)
}

fn plan_release(env: &Env, path: &Path, record: &Record) -> Planned {
    let ours = record.deploy == Deploy::Link
        && tree::link_of(&env.target(path)) == Some(env.repo.join(&record.source));
    let work = if ours {
        Work::Unlink(record.clone())
    } else {
        Work::Drop
    };
    step(path, Action::Released, None, work)
}

/// Deploy every dotfile. Problems with one path are reported in its step
/// and do not stop the others. The caller holds the adopt lock.
pub fn deploy(ctx: &Context) -> Result<Vec<Step>> {
    let planned = plan(ctx)?;
    let state_dir = ctx.paths.state();
    let mut deployed = state::load(state_dir)?;
    let repo = ctx.paths.repo();
    let snapshots = Snapshots::new(state_dir, &ctx.home);
    let mut keep = Vec::new();
    for item in &planned {
        if matches!(
            item.work,
            Work::Link(_) | Work::Write(..) | Work::Save(_) | Work::Unlink(_)
        ) {
            keep.extend(tree::files(&ctx.home.join(&item.step.path))?);
        }
    }
    snapshots.save(&keep, "deploy dotfiles")?;
    let mut steps = Vec::new();
    let mut waiting = Vec::new();
    for item in planned {
        if let Some(sides) = item.sides {
            let reason = match item.step.action {
                Action::Changed => Reason::Changed,
                _ => Reason::Conflict,
            };
            waiting.push(DotfileItem {
                path: item.step.path.clone(),
                reason,
                sides,
            });
        }
        let mut step = item.step;
        if let Err(err) = run(ctx, &repo, &mut deployed, &step.path, item.work) {
            step.action = Action::Failed;
            step.reason = Some(err.to_string());
        }
        steps.push(step);
    }
    state::save(state_dir, &deployed)?;
    review::sync_queue(ctx, waiting)?;
    Ok(steps)
}

/// Write what `target` holds to `source` in the repo, encrypted when declared so.
pub(crate) fn save(ctx: &Context, entry: &Dotfile, target: &Path, source: &Path) -> Result<()> {
    if !entry.encrypted {
        return tree::put(Content::Tree(target), source);
    }
    let plain = std::fs::read(target).map_err(Error::read(target))?;
    tree::put(Content::Bytes(&crypt::encrypt(ctx, &plain)?), source)
}

/// Put the repo's version at `path`, keeping what was there in a snapshot.
pub(crate) fn force(ctx: &Context, path: &Path, entry: &Dotfile) -> Result<()> {
    let repo = ctx.paths.repo();
    let work = match entry.deploy {
        Deploy::Link => Work::Link(entry.clone()),
        Deploy::Copy => match desired(ctx, entry)? {
            Some(desired) => Work::Write(entry.clone(), desired),
            None => return Err(Error::Usage(LOCKED.into())),
        },
        Deploy::Template => {
            return Err(Error::Usage("template deploy is not supported yet".into()));
        }
    };
    let target = ctx.home.join(path);
    Snapshots::new(ctx.paths.state(), &ctx.home).save(
        &tree::files(&target)?,
        &format!("discard {}", path.display()),
    )?;
    let mut deployed = state::load(ctx.paths.state())?;
    run(ctx, &repo, &mut deployed, path, work)?;
    state::save(ctx.paths.state(), &deployed)
}

fn run(ctx: &Context, repo: &Path, deployed: &mut Deployed, path: &Path, work: Work) -> Result<()> {
    let target = ctx.home.join(path);
    let entry = match work {
        Work::Nothing => return Ok(()),
        Work::Drop => {
            deployed.entries.remove(path);
            return Ok(());
        }
        Work::Unlink(record) => {
            let source = repo.join(&record.source);
            if std::fs::symlink_metadata(&source).is_ok() {
                tree::put(Content::Tree(&source), &target)?;
            } else {
                history::recover(repo, &record.source, &target)?;
            }
            deployed.entries.remove(path);
            return Ok(());
        }
        Work::Keep(entry) => entry,
        Work::Link(entry) => {
            tree::link(&repo.join(entry.source()), &target)?;
            entry
        }
        Work::Write(entry, desired) => {
            tree::put(desired.content(), &target)?;
            entry
        }
        Work::Save(entry) => {
            let source = repo.join(entry.source());
            save(ctx, &entry, &target, &source)?;
            if entry.deploy == Deploy::Link {
                tree::link(&source, &target)?;
            }
            entry
        }
    };
    let source = repo.join(entry.source());
    if let Some(mode) = entry.mode() {
        let at = if entry.deploy == Deploy::Link {
            &source
        } else {
            &target
        };
        tree::set_mode(at, mode)?;
    }
    let source_hash = tree::hash(&source)?.unwrap_or_default();
    let target_hash = match entry.deploy {
        Deploy::Link => source_hash.clone(),
        _ => tree::hash(&target)?.unwrap_or_default(),
    };
    deployed.entries.insert(
        path.to_path_buf(),
        Record {
            deploy: entry.deploy,
            source: entry.source(),
            source_hash,
            target_hash,
        },
    );
    deployed.ignored.remove(path);
    Ok(())
}

/// Keep what this host has at `path` and stop asking until either side
/// changes.
pub fn ignore(ctx: &Context, path: &Path, sides: Sides) -> Result<()> {
    let mut deployed = state::load(ctx.paths.state())?;
    deployed.ignored.insert(path.to_path_buf(), sides);
    state::save(ctx.paths.state(), &deployed)
}
