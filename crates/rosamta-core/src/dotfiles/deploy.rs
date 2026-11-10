//! Put each declared dotfile in place, or tell why not.
//!
//! The deploy record separates "the repo changed" from "this host changed":
//! a target untouched since the last deploy follows the repo; a target
//! changed while the repo stayed is written back to the repo (links always,
//! copies by the adopt policy); both changed is a conflict nobody overwrites.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use super::review::{self, DotfileItem, Reason, WaitingFor};
use super::state::{self, Deployed, Record, Sides};
use super::tree::{self, Content};
use super::{Choice, crypt, history};

/// Why an encrypted dotfile waits.
pub(crate) const LOCKED: &str = "encrypted; the key is not unlocked on this host";
use crate::apply::Context;
use crate::config::{AdoptPolicy, Deploy, Dotfile, HostConfig, HostFacts};
use crate::error::{Error, Result};
use crate::secrets::{self, Session, Waiting};
use crate::snapshot::Snapshots;
use crate::{host, template};

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
    /// What a template waits for before it can render.
    waiting: Option<WaitingFor>,
    work: Work,
}

enum Work {
    /// Nothing on disk; refresh the record and permissions.
    Keep(Dotfile),
    Link(Dotfile),
    Write(Dotfile, Desired),
    /// Write the host's content back to the repo, then deploy it again.
    Save(Dotfile),
    /// Record a template's input, writing its output when there is one.
    Template(Dotfile, String, Option<Vec<u8>>),
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
    host: HostConfig,
    policy: AdoptPolicy,
    deployed: Deployed,
    /// For templates; a broken `secrets.toml` fails them alone.
    secrets: std::result::Result<Arc<Session>, String>,
}

impl<'a> Env<'a> {
    fn new(ctx: &'a Context) -> Result<Self> {
        let host = host::load(&ctx.paths.host_config())?;
        let secrets = Session::new(ctx, &host, ctx.interactive)
            .map(Arc::new)
            .map_err(|err| err.to_string());
        Ok(Self {
            ctx,
            repo: ctx.paths.repo(),
            policy: host.adopt,
            host,
            deployed: state::load(ctx.paths.state())?,
            secrets,
        })
    }

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
        waiting: None,
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
    let env = Env::new(ctx)?;
    let (manifest, broken) = super::usable(&env.repo)?;
    let facts = HostFacts::current(&env.host);
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
        Deploy::Template => plan_template(env, path, entry),
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

fn plan_template(env: &Env, path: &Path, entry: &Dotfile) -> Result<Planned> {
    let target = env.target(path);
    let Some(text) = template_text(env.ctx, entry)? else {
        return Ok(step(
            path,
            Action::Skipped,
            Some(LOCKED.into()),
            Work::Nothing,
        ));
    };
    let input = input_hash(env, entry, &text)?;
    let local = tree::hash(&target)?;
    let record = env
        .deployed
        .entries
        .get(path)
        .filter(|record| record.deploy == Deploy::Template);
    let untouched = record.is_some_and(|record| Some(&record.target_hash) == local.as_ref());
    let same_input = record.is_some_and(|record| record.source_hash == input);
    let keep = || Work::Template(entry.clone(), input.clone(), None);
    // Outside a terminal only a changed input renders again, so an
    // unchanged template never asks a secret source.
    if untouched && same_input && !env.ctx.interactive {
        return Ok(step(path, Action::Unchanged, None, keep()));
    }
    // Output edited on this host cannot go back into the template.
    if let (Some(local), Some(_), false) = (&local, record, untouched) {
        if env.ignored(path, local, &input) {
            return Ok(ignored(path));
        }
        return Ok(match (same_input, env.policy) {
            (true, AdoptPolicy::Off) => step(
                path,
                Action::Skipped,
                Some("changed on this host".into()),
                Work::Nothing,
            ),
            (true, _) => wait(path, Action::Changed, local.clone(), input),
            (false, _) => wait(path, Action::Conflict, local.clone(), input),
        });
    }
    let (bytes, secret) = match render(env, entry, &text)? {
        Rendered::Done { bytes, secret } => (bytes, secret),
        Rendered::Waiting(waiting) => {
            let mut planned = step(
                path,
                Action::Skipped,
                Some(waiting.describe()),
                Work::Nothing,
            );
            planned.sides = Some(Sides {
                local: local.unwrap_or_default(),
                repo: input,
            });
            planned.waiting = Some(waiting);
            return Ok(planned);
        }
    };
    let action = match &local {
        None => Action::Copied,
        Some(local) if *local == tree::hash_bytes(&bytes) => {
            return Ok(step(path, Action::Unchanged, None, keep()));
        }
        Some(_) if untouched => Action::Updated,
        Some(local) if env.ignored(path, local, &input) => return Ok(ignored(path)),
        Some(local) => return Ok(wait(path, Action::Conflict, local.clone(), input)),
    };
    let mut planned = step(
        path,
        action,
        None,
        Work::Template(entry.clone(), input, None),
    );
    // A preview of output holding a secret would print the secret.
    if !secret {
        let before = local.and_then(|_| std::fs::read_to_string(&target).ok());
        planned.preview = Some((before, String::from_utf8(bytes.clone()).ok()));
    }
    if let Work::Template(_, _, output) = &mut planned.work {
        *output = Some(bytes);
    }
    Ok(planned)
}

/// A template's text, decrypted when encrypted; `None` while this host has
/// no key.
fn template_text(ctx: &Context, entry: &Dotfile) -> Result<Option<String>> {
    let source = ctx.paths.repo().join(entry.source());
    let bytes = std::fs::read(&source).map_err(Error::read(&source))?;
    let bytes = match entry.encrypted {
        false => bytes,
        true => match crypt::decrypt(ctx, &bytes)? {
            Some(bytes) => bytes,
            None => return Ok(None),
        },
    };
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| Error::Usage(format!("template `{}` is not UTF-8 text", entry.source())))
}

/// What a template's output depends on, secrets aside: its text, this host,
/// the vars it uses and, when it runs commands, their declarations.
fn input_hash(env: &Env, entry: &Dotfile, text: &str) -> Result<String> {
    let used = template::used_vars(&entry.source(), text).map_err(Error::Usage)?;
    let vars: BTreeMap<&String, Option<&String>> = used
        .iter()
        .map(|name| (name, env.host.vars.get(name)))
        .collect();
    let commands = if text.contains("secret(") {
        std::fs::read_to_string(env.repo.join(secrets::MANIFEST)).unwrap_or_default()
    } else {
        String::new()
    };
    let input = serde_json::json!({
        "text": text,
        "id": env.host.id,
        "tags": env.host.tags,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "home": env.ctx.home,
        "vars": vars,
        "commands": commands,
    });
    Ok(tree::hash_bytes(input.to_string().as_bytes()))
}

/// The input hash of the template `entry` as it is now; `None` while this
/// host has no key.
pub(crate) fn template_input(ctx: &Context, entry: &Dotfile) -> Result<Option<String>> {
    let env = Env::new(ctx)?;
    match template_text(ctx, entry)? {
        Some(text) => input_hash(&env, entry, &text).map(Some),
        None => Ok(None),
    }
}

enum Rendered {
    /// The output, and whether a secret went into it.
    Done {
        bytes: Vec<u8>,
        secret: bool,
    },
    Waiting(WaitingFor),
}

fn render(env: &Env, entry: &Dotfile, text: &str) -> Result<Rendered> {
    let name = entry.source();
    let missing = template::missing_vars(&name, text, &env.host.vars).map_err(Error::Usage)?;
    if !missing.is_empty() {
        return Ok(Rendered::Waiting(WaitingFor::Unset { vars: missing }));
    }
    let session = env
        .secrets
        .as_ref()
        .map_err(|err| Error::Usage(err.clone()))?;
    let before = session.used();
    match template::render(&name, text, &env.host, &env.ctx.home, session) {
        Ok(output) => Ok(Rendered::Done {
            bytes: output.into_bytes(),
            secret: session.used() > before,
        }),
        Err(err) => match session.take_waiting().into_iter().next() {
            Some(Waiting::Locked(tool)) => Ok(Rendered::Waiting(WaitingFor::Locked {
                tool: tool.name().into(),
            })),
            Some(Waiting::Command(name)) => Ok(Rendered::Waiting(WaitingFor::Command { name })),
            None => Err(Error::Usage(err)),
        },
    }
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
            Work::Link(_)
                | Work::Write(..)
                | Work::Save(_)
                | Work::Unlink(_)
                | Work::Template(_, _, Some(_))
        ) {
            keep.extend(tree::files(&ctx.home.join(&item.step.path))?);
        }
    }
    snapshots.save(&keep, "deploy dotfiles")?;
    let mut steps = Vec::new();
    let mut waiting = Vec::new();
    for item in planned {
        if let Some(sides) = item.sides {
            let reason = match (&item.waiting, item.step.action) {
                (Some(_), _) => Reason::Waiting,
                (None, Action::Changed) => Reason::Changed,
                (None, _) => Reason::Conflict,
            };
            waiting.push(DotfileItem {
                path: item.step.path.clone(),
                reason,
                sides,
                waiting: item.waiting,
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
            let env = Env::new(ctx)?;
            let text = template_text(ctx, entry)?.ok_or_else(|| Error::Usage(LOCKED.into()))?;
            let input = input_hash(&env, entry, &text)?;
            match render(&env, entry, &text)? {
                Rendered::Done { bytes, .. } => Work::Template(entry.clone(), input, Some(bytes)),
                Rendered::Waiting(waiting) => return Err(Error::Usage(waiting.describe())),
            }
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
    let mut input = None;
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
            let previous = tree::mode_of(&target);
            tree::put(desired.content(), &target)?;
            // Bytes land private; a file that was there keeps its mode.
            if let (Desired::Bytes(_), None, Some(mode)) = (&desired, entry.mode(), previous) {
                tree::set_mode(&target, mode)?;
            }
            entry
        }
        Work::Template(entry, hash, output) => {
            if let Some(bytes) = output {
                let previous = tree::mode_of(&target);
                tree::put(Content::Bytes(&bytes), &target)?;
                if let (None, Some(mode)) = (entry.mode(), previous) {
                    tree::set_mode(&target, mode)?;
                }
            }
            input = Some(hash);
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
    let source_hash = match input {
        Some(input) => input,
        None => tree::hash(&source)?.unwrap_or_default(),
    };
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
