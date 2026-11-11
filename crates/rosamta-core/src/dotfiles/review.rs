//! Dotfiles in the review queue: conflicts, and copies changed on this host
//! while the adopt policy asks. `accept` keeps this host's version,
//! `discard` takes the repo's, `ignore` leaves both alone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::state::Sides;
use super::tree;
use super::{Choice, MANIFEST, deploy};
use crate::adopt::store::Store;
use crate::adopt::{self, QueueItem, Scope};
use crate::apply::Context;
use crate::config::{Deploy, Dotfile, HostFacts};
use crate::error::{Error, Result};
use crate::sync;
use crate::time::now;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Both sides changed, or the path held something else before.
    Conflict,
    /// Changed on this host only.
    Changed,
    /// A template that cannot render outside a terminal yet.
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DotfileItem {
    /// Relative to the home directory.
    pub path: PathBuf,
    pub reason: Reason,
    #[serde(flatten)]
    pub sides: Sides,
    /// What a `Waiting` item waits for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<WaitingFor>,
}

/// What a template needs before it renders; `rsmt apply` in a terminal
/// provides each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "for", rename_all = "snake_case")]
pub enum WaitingFor {
    /// A Bitwarden tool to unlock.
    Locked { tool: String },
    /// A secret command, which runs only in a terminal.
    Command { name: String },
    /// Vars this host has no value for.
    Unset { vars: Vec<String> },
}

impl WaitingFor {
    pub fn describe(&self) -> String {
        match self {
            Self::Locked { tool } => {
                format!("waiting for {tool} to unlock; run `rsmt apply` in a terminal")
            }
            Self::Command { name } => {
                format!("secret command `{name}` runs only in `rsmt apply` in a terminal")
            }
            Self::Unset { vars } => {
                format!(
                    "waiting for {} in host.toml; run `rsmt apply` in a terminal",
                    var_list(vars)
                )
            }
        }
    }
}

/// `vars.email, vars.name`, as templates write them.
pub fn var_list(vars: &[String]) -> String {
    let names: Vec<String> = vars.iter().map(|var| format!("vars.{var}")).collect();
    names.join(", ")
}

/// Make the queue's dotfile items exactly `waiting`. An item keeps its ID
/// while neither side changes.
pub(crate) fn sync_queue(ctx: &Context, waiting: Vec<DotfileItem>) -> Result<()> {
    let store = Store::new(ctx.paths.state());
    let mut queue = store.queue()?;
    let (old, mut items): (Vec<QueueItem>, Vec<QueueItem>) = queue
        .items
        .into_iter()
        .partition(|item| item.dotfile.is_some());
    let same = waiting.len() == old.len()
        && waiting
            .iter()
            .all(|item| old.iter().any(|old| old.dotfile.as_ref() == Some(item)));
    if same {
        return Ok(());
    }
    queue.next_id = queue.next_id.max(1);
    for item in waiting {
        // What it waits for may come and go, as a tool locks and unlocks;
        // the item is the same while the path and both sides are.
        let kept = old
            .iter()
            .find(|old| {
                old.dotfile
                    .as_ref()
                    .is_some_and(|old| old.path == item.path && old.sides == item.sides)
            })
            .map(|old| (old.id, old.detected_at));
        let (id, detected_at) = kept.unwrap_or_else(|| {
            queue.next_id += 1;
            (queue.next_id - 1, now())
        });
        items.push(QueueItem {
            id,
            file: ctx.home.join(&item.path),
            start: 0,
            end: 0,
            detected_at,
            hash: String::new(),
            text: String::new(),
            module: None,
            local: false,
            dotfile: Some(item),
            script: None,
        });
    }
    items.sort_by_key(|item| item.id);
    queue.items = items;
    store.save_queue(&queue)?;
    adopt::update_notice(&store, &adopt::host_config(ctx)?, queue.items.len())
}

/// The declaration this host uses for `path`, and both sides as they are now.
fn current(ctx: &Context, path: &Path) -> Result<(Dotfile, Sides)> {
    let repo = ctx.paths.repo();
    let host = adopt::host_config(ctx)?;
    let selected = super::select(&super::usable(&repo)?.0, &HostFacts::current(&host));
    let Some(Choice::Use(entry)) = selected.get(path) else {
        return Err(Error::Usage(format!(
            "{} is no longer a dotfile of this host; run `rsmt review`",
            path.display()
        )));
    };
    let hash = |at: &Path| tree::hash(at).map(Option::unwrap_or_default);
    let repo_side = match entry.deploy {
        Deploy::Template => deploy::template_input(ctx, entry)?
            .ok_or_else(|| Error::Usage(deploy::LOCKED.into()))?,
        _ => hash(&repo.join(entry.source()))?,
    };
    let sides = Sides {
        local: hash(&ctx.home.join(path))?,
        repo: repo_side,
    };
    Ok((entry.clone(), sides))
}

fn unchanged(ctx: &Context, id: u32, item: &DotfileItem) -> Result<Dotfile> {
    if let Some(waiting) = &item.waiting {
        return Err(Error::Usage(format!(
            "review item {id} is not a decision: {}",
            waiting.describe()
        )));
    }
    let (entry, sides) = current(ctx, &item.path)?;
    if sides != item.sides {
        return Err(Error::Stale(id));
    }
    Ok(entry)
}

/// `rsmt review accept`: keep this host's version, in a declaration for
/// this host alone (`Scope::Host`) or for every host using the declaration.
/// A template's output becomes a copy for this host alone.
pub(crate) fn keep(ctx: &Context, id: u32, item: &DotfileItem, scope: Scope) -> Result<()> {
    let entry = unchanged(ctx, id, item)?;
    let template = entry.deploy == Deploy::Template;
    if template && scope == Scope::All {
        return Err(Error::Usage(
            "a template's output is kept for this host only; use `--scope host`".into(),
        ));
    }
    let target = ctx.home.join(&item.path);
    if tree::link_of(&target).is_some_and(|link| link.starts_with(ctx.paths.repo())) {
        return Err(Error::Usage(format!(
            "{} links into the repo; there is nothing of this host's to keep",
            target.display()
        )));
    }
    if template && !entry.encrypted && secret_template(ctx, &entry)? {
        return Err(Error::Usage(format!(
            "{} holds secrets from its template; it stays out of the repo",
            target.display()
        )));
    }
    if !entry.encrypted
        && let Some((file, rule)) = secret_in(&target)?
    {
        return Err(Error::Usage(format!(
            "{} looks like it contains a secret ({rule}); it stays out of the repo",
            file.display()
        )));
    }
    let host = adopt::host_config(ctx)?;
    let repo = ctx.paths.repo();
    let own = entry.when.hosts == [host.id.clone()];
    if template && own {
        return Err(Error::Usage(
            "this template is for this host only; edit the template in the repo instead".into(),
        ));
    }
    let mut paths = Vec::new();
    let source = if scope == Scope::All || own {
        entry.source()
    } else {
        let variant = Dotfile {
            source: Some(format!("hosts/{}/{}", host.id, item.path.display())),
            deploy: if template { Deploy::Copy } else { entry.deploy },
            when: crate::config::When {
                hosts: vec![host.id.clone()],
                ..Default::default()
            },
            ..entry.clone()
        };
        append(&repo, &variant)?;
        paths.push(MANIFEST.to_string());
        variant.source()
    };
    deploy::save(ctx, &entry, &target, &repo.join(&source))?;
    paths.push(source);
    let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
    sync::commit_scanned(ctx, &paths, None)?;
    deploy::deploy(ctx).map(drop)
}

/// `rsmt review discard`: put the repo's version in place; what this host
/// had stays in a snapshot.
pub fn discard(ctx: &Context, id: u32) -> Result<PathBuf> {
    let (item, _) = adopt::take_item(ctx, id)?;
    let Some(dotfile) = item.dotfile else {
        return Err(Error::Usage(if item.script.is_some() {
            "discard applies only to dotfiles; `rsmt review ignore` counts a script as run".into()
        } else {
            "discard applies only to dotfiles; use `rsmt review ignore` for rc file content".into()
        }));
    };
    let entry = unchanged(ctx, id, &dotfile)?;
    deploy::force(ctx, &dotfile.path, &entry)?;
    deploy::deploy(ctx)?;
    Ok(dotfile.path)
}

/// `rsmt review ignore`: leave both sides as they are until either changes.
pub(crate) fn ignore(ctx: &Context, id: u32, item: &DotfileItem) -> Result<()> {
    unchanged(ctx, id, item)?;
    deploy::ignore(ctx, &item.path, item.sides.clone())?;
    deploy::deploy(ctx).map(drop)
}

/// Whether the template `entry` asks a secret source.
fn secret_template(ctx: &Context, entry: &Dotfile) -> Result<bool> {
    let text = deploy::template_text(ctx, entry)?.unwrap_or_default();
    let names = crate::template::names(&entry.source(), &text).map_err(Error::Usage)?;
    Ok(crate::template::uses_secrets(&names))
}

/// The first file under `path` whose content looks like a secret, and the rule.
pub(crate) fn secret_in(path: &Path) -> Result<Option<(PathBuf, &'static str)>> {
    for file in tree::files(path)? {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        if let Some(rule) = adopt::secret_rule(&text) {
            return Ok(Some((file, rule)));
        }
    }
    Ok(None)
}

/// Add a declaration at the end of `dotfiles.toml`, leaving the rest as written.
pub(crate) fn append(repo: &Path, entry: &Dotfile) -> Result<()> {
    use toml_edit::{Array, ArrayOfTables, InlineTable, Table, value};
    let path = repo.join(MANIFEST);
    if !path.exists() {
        crate::edit::write_atomic(&path, "")?;
    }
    let mut table = Table::new();
    table["path"] = value(&entry.path);
    if let Some(source) = &entry.source {
        table["source"] = value(source);
    }
    if entry.deploy != crate::config::Deploy::Link {
        let deploy = serde_json::to_value(entry.deploy).expect("a deploy mode serializes");
        table["deploy"] = value(deploy.as_str().unwrap_or_default());
    }
    if let Some(permissions) = &entry.permissions {
        table["permissions"] = value(permissions);
    }
    if entry.encrypted {
        table["encrypted"] = value(true);
    }
    let when = serde_json::to_value(&entry.when).expect("a condition serializes");
    let mut inline = InlineTable::new();
    for (key, values) in when.as_object().into_iter().flatten() {
        let values: Array = values
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
            .collect();
        if !values.is_empty() {
            inline.insert(key, values.into());
        }
    }
    if !inline.is_empty() {
        table["when"] = value(inline);
    }
    crate::edit::edit_toml(&path, |doc| {
        if !doc.contains_key("dotfile") {
            doc["dotfile"] = toml_edit::Item::ArrayOfTables(ArrayOfTables::new());
        }
        if let Some(entries) = doc["dotfile"].as_array_of_tables_mut() {
            entries.push(table);
        }
    })
    .map(drop)
}

/// A unified diff from this host's version of `item` to the repo's. A
/// template's side is its output, or its text when rendering would show a
/// secret.
pub fn diff(ctx: &Context, item: &DotfileItem) -> Result<String> {
    let (entry, _) = current(ctx, &item.path)?;
    let target = ctx.home.join(&item.path);
    let local = contents(&target)?;
    let source = ctx.paths.repo().join(entry.source());
    let output = match entry.deploy {
        Deploy::Template => deploy::output(ctx, &entry)?,
        _ => None,
    };
    let repo = if let Some(output) = output {
        BTreeMap::from([(PathBuf::new(), output)])
    } else if entry.encrypted {
        let sealed = std::fs::read(&source).map_err(Error::read(&source))?;
        let plain = super::crypt::decrypt(ctx, &sealed)?
            .ok_or_else(|| Error::Usage(deploy::LOCKED.into()))?;
        BTreeMap::from([(PathBuf::new(), plain)])
    } else {
        contents(&source)?
    };
    let mut out = String::new();
    for rel in local
        .keys()
        .chain(repo.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let (before, after) = (local.get(rel), repo.get(rel));
        if before == after {
            continue;
        }
        let at = if rel.as_os_str().is_empty() {
            target.clone()
        } else {
            target.join(rel)
        };
        let name = at.display().to_string();
        let text = |bytes: Option<&Vec<u8>>| {
            bytes.map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        };
        out.push_str(&crate::apply::unified(
            &name,
            text(before).as_deref(),
            text(after).as_deref(),
        ));
    }
    Ok(out)
}

/// Each file under `root` by its path relative to `root`.
fn contents(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut out = BTreeMap::new();
    if std::fs::symlink_metadata(root).is_err() {
        return Ok(out);
    }
    for file in tree::files(root)? {
        let rel = file
            .strip_prefix(root)
            .unwrap_or(Path::new(""))
            .to_path_buf();
        let bytes = match tree::link_of(&file) {
            Some(target) => format!("-> {}\n", target.display()).into_bytes(),
            None => std::fs::read(&file).map_err(Error::read(&file))?,
        };
        out.insert(rel, bytes);
    }
    Ok(out)
}
