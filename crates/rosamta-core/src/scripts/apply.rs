//! Which scripts this host has to run, and running them per who applies:
//! a terminal runs them, anything else follows the host's preference.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{Run, Script, Timing, run, usable};
use crate::adopt::store::Store;
use crate::apply::Context;
use crate::config::{HostConfig, HostFacts, ScriptPolicy};
use crate::dotfiles::{clean, tree};
use crate::error::Result;

const FILE: &str = "scripts.json";

/// `scripts.json`: what ran on this host, by source.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ran {
    #[serde(default)]
    pub entries: BTreeMap<String, Record>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The script and what it watches when it last succeeded, or was
    /// marked as run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    /// The last run failed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub failed: bool,
}

pub fn load(state: &Path) -> Result<Ran> {
    Store::new(state).load(FILE)
}

pub fn save(state: &Path, value: &Ran) -> Result<()> {
    Store::new(state).save(FILE, value)
}

/// Why a script needs to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// It never ran on this host.
    New,
    /// It or what it watches changed since it last ran.
    Changed,
    /// Its last run failed.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub script: Script,
    pub why: Why,
    pub hash: String,
}

/// A script's name: its source as a plain repo path.
pub fn key(script: &Script) -> String {
    plain(&script.source)
}

fn plain(path: &str) -> String {
    clean(path).map_or_else(|| path.to_string(), |path| path.display().to_string())
}

/// The script and everything it watches, so an edit to either counts. How
/// the list is written does not.
pub fn hash(repo: &Path, script: &Script) -> Result<String> {
    let mut watched: Vec<String> = script.watch.iter().map(|path| plain(path)).collect();
    watched.sort();
    watched.dedup();
    let mut parts = vec![followed_hash(&repo.join(&script.source))?];
    for watched in watched {
        let content = followed_hash(&repo.join(&watched))?;
        parts.push(format!("{watched}\0{content}"));
    }
    Ok(tree::hash_bytes(parts.join("\n").as_bytes()))
}

/// What a run reads at `path`: through a link, the file it points at.
fn followed_hash(path: &Path) -> Result<String> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    Ok(tree::hash(&path)?.unwrap_or_default())
}

/// The scripts of this host that need to run, in declaration order.
pub fn pending(ctx: &Context, host: &HostConfig) -> Result<Vec<Pending>> {
    let repo = ctx.paths.repo();
    let (manifest, _) = usable(&repo)?;
    let ran = load(ctx.paths.state())?;
    let facts = HostFacts::current(host);
    let mut out = Vec::new();
    for script in manifest
        .script
        .into_iter()
        .filter(|script| script.when.matches(&facts))
    {
        let hash = hash(&repo, &script)?;
        let record = ran.entries.get(&key(&script));
        let why = match record {
            Some(record) if record.failed => Why::Failed,
            None | Some(Record { hash: None, .. }) => Why::New,
            Some(Record { hash: Some(_), .. }) if script.run == Run::Once => continue,
            Some(Record {
                hash: Some(last), ..
            }) if *last == hash => continue,
            Some(_) => Why::Changed,
        };
        out.push(Pending { script, why, hash });
    }
    Ok(out)
}

/// Remember how a run of `pending` went.
pub(crate) fn remember(ctx: &Context, pending: &Pending, ok: bool) -> Result<()> {
    let mut ran = load(ctx.paths.state())?;
    let record = ran.entries.entry(key(&pending.script)).or_default();
    if ok {
        record.hash = Some(pending.hash.clone());
    }
    record.failed = !ok;
    save(ctx.paths.state(), &ran)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Ran,
    Failed,
    /// Listed in the review queue instead of run.
    Queued,
    /// What a dry run would run.
    WillRun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    /// The script's source in the repo.
    pub source: String,
    pub action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<Why>,
    /// How it ended, or what is wrong with its declaration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Whether this apply runs the script itself. Without a terminal, one that
/// failed waits for the user instead of failing again on every apply.
fn runs(ctx: &Context, host: &HostConfig, why: Why) -> bool {
    ctx.interactive || (host.scripts == ScriptPolicy::Auto && why != Why::Failed)
}

/// What applying would do with each script, running nothing.
pub fn plan(ctx: &Context) -> Result<Vec<Step>> {
    let host = crate::adopt::host_config(ctx)?;
    let mut steps = broken(ctx, &host, None)?;
    steps.extend(pending(ctx, &host)?.into_iter().map(|pending| Step {
        source: key(&pending.script),
        action: if runs(ctx, &host, pending.why) {
            Action::WillRun
        } else {
            Action::Queued
        },
        why: Some(pending.why),
        reason: None,
    }));
    Ok(steps)
}

/// Declarations of this host that cannot run, as failed steps.
fn broken(ctx: &Context, host: &HostConfig, timing: Option<Timing>) -> Result<Vec<Step>> {
    let facts = HostFacts::current(host);
    let (_, broken) = usable(&ctx.paths.repo())?;
    Ok(broken
        .into_iter()
        .filter(|(script, _)| script.when.matches(&facts))
        .filter(|(script, _)| timing.is_none_or(|timing| script.timing == timing))
        .map(|(script, problem)| Step {
            source: key(&script),
            action: Action::Failed,
            why: None,
            reason: Some(problem),
        })
        .collect())
}

/// Run, or queue, the scripts of `timing`. One failing does not stop the
/// rest. The caller holds the adopt lock.
pub fn apply(ctx: &Context, timing: Timing) -> Result<Vec<Step>> {
    let host = crate::adopt::host_config(ctx)?;
    let mut steps = broken(ctx, &host, Some(timing))?;
    for pending in pending(ctx, &host)?
        .into_iter()
        .filter(|pending| pending.script.timing == timing)
    {
        let source = key(&pending.script);
        if !runs(ctx, &host, pending.why) {
            steps.push(Step {
                source,
                action: Action::Queued,
                why: Some(pending.why),
                reason: None,
            });
            continue;
        }
        if ctx.interactive {
            println!("Running {source}");
        }
        let outcome = run::run(ctx, &host.id, &pending.script, ctx.interactive);
        let (ok, reason) = match outcome {
            Ok(outcome) => (outcome.ok, outcome.status),
            Err(err) => (false, err.to_string()),
        };
        remember(ctx, &pending, ok)?;
        steps.push(Step {
            source,
            action: if ok { Action::Ran } else { Action::Failed },
            why: Some(pending.why),
            reason: (!ok).then_some(reason),
        });
    }
    Ok(steps)
}
