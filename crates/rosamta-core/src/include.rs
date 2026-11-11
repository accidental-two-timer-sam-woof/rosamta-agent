//! What other hosts added that does not load here, and adding this host to
//! it: `rsmt recent` and `rsmt include`.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use toml_edit::DocumentMut;

use crate::apply::{self, Context};
use crate::config::{HostFacts, Module, Shell, When};
use crate::error::{Error, Result};
use crate::hooks::FileChange;
use crate::scope::Widening;
use crate::{adopt, dotfiles, git, repo, sync};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Module,
    Dotfile,
}

/// Why an item cannot load here, however far its scope is widened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Blocked {
    /// A snippet only for shells this host has no hook in.
    Shells(Vec<Shell>),
    /// Tags this host does not have.
    Tags(Vec<String>),
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    /// The module's name, or the dotfile's path as `~/…`.
    pub name: String,
    pub kind: Kind,
    /// The file to edit, relative to the repo.
    pub file: PathBuf,
    /// When it was added to the repo; `None` before it is committed.
    pub added: Option<u64>,
    pub when: When,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shells: Vec<Shell>,
    /// What to add so it loads here.
    pub widening: Widening,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<Blocked>,
    /// The `[[dotfile]]` entry, counting from 0.
    #[serde(skip)]
    entry: usize,
}

/// Enabled modules and dotfiles in the repo that do not load on this host,
/// newest first.
pub fn recent(ctx: &Context) -> Result<Vec<Item>> {
    let repo_dir = ctx.paths.repo();
    let facts = HostFacts::current(&adopt::host_config(ctx)?);
    let hooked: Vec<Shell> = apply::shells(ctx)?
        .into_iter()
        .filter(|status| !status.hooked.is_empty())
        .map(|status| status.shell)
        .collect();
    let added = added(&repo_dir)?;
    let added_at = |source: &str| {
        let inside = format!("{source}/");
        added
            .iter()
            .filter(|(file, _)| *file == source || file.starts_with(&inside))
            .map(|(_, at)| *at)
            .min()
    };
    let mut items = Vec::new();
    let files: HashMap<String, PathBuf> = repo::module_files(&repo_dir)?
        .into_iter()
        .map(|path| (repo::module_name(&path), path))
        .collect();
    for (name, module) in apply::load_modules(&repo_dir)? {
        let Some(path) = files.get(&name) else {
            continue;
        };
        let file = path.strip_prefix(&repo_dir).unwrap_or(path).to_path_buf();
        if let Some(mut item) = module_item(name, &module, &facts, &hooked) {
            item.added = added_at(&file.to_string_lossy());
            item.file = file;
            items.push(item);
        }
    }
    items.extend(dotfile_items(&repo_dir, &facts, &added_at)?);
    items.sort_by(|a, b| {
        let newest = |item: &Item| Reverse(item.added.unwrap_or(u64::MAX));
        (newest(a), &a.name).cmp(&(newest(b), &b.name))
    });
    Ok(items)
}

fn module_item(name: String, module: &Module, facts: &HostFacts, hooked: &[Shell]) -> Option<Item> {
    if !module.enabled {
        return None;
    }
    let structured = !module.env.is_empty()
        || !module.path.prepend.is_empty()
        || !module.path.append.is_empty()
        || !module.aliases.is_empty();
    let listed = |shell: &Shell| module.shells.is_empty() || module.shells.contains(shell);
    let runs = |shell: &Shell| listed(shell) && (structured || module.raw.get(*shell).is_some());
    // With no shell hooked there is nothing to judge shells by.
    let shells_ok = hooked.is_empty() || hooked.iter().any(runs);
    if module.when.matches(facts) && shells_ok {
        return None;
    }
    let mut widening = widening(&module.when, facts);
    let mut blocked = tags(&module.when, facts);
    if !shells_ok && blocked.is_none() {
        if structured {
            widening.shells = hooked.to_vec();
        } else {
            let only = Shell::ALL
                .into_iter()
                .filter(|shell| listed(shell) && module.raw.get(*shell).is_some())
                .collect();
            blocked = Some(Blocked::Shells(only));
        }
    }
    Some(Item {
        name,
        kind: Kind::Module,
        file: PathBuf::new(),
        added: None,
        when: module.when.clone(),
        shells: module.shells.clone(),
        widening,
        blocked,
        entry: 0,
    })
}

/// Dotfiles with no entry for their path that deploys here; of several
/// entries for one path, the newest.
fn dotfile_items(
    repo_dir: &Path,
    facts: &HostFacts,
    added_at: &dyn Fn(&str) -> Option<u64>,
) -> Result<Vec<Item>> {
    let manifest = dotfiles::load(repo_dir)?;
    let path_of =
        |path: &str| dotfiles::clean(path).map(|path| path.to_string_lossy().into_owned());
    let here: HashSet<String> = manifest
        .dotfile
        .iter()
        .filter(|entry| entry.when.matches(facts))
        .filter_map(|entry| path_of(&entry.path))
        .collect();
    let mut items: Vec<Item> = Vec::new();
    for (index, entry) in manifest.dotfile.iter().enumerate() {
        let Some(path) = path_of(&entry.path).filter(|path| !here.contains(path)) else {
            continue;
        };
        let item = Item {
            name: format!("~/{path}"),
            kind: Kind::Dotfile,
            file: PathBuf::from(dotfiles::MANIFEST),
            added: added_at(&entry.source()),
            when: entry.when.clone(),
            shells: Vec::new(),
            widening: widening(&entry.when, facts),
            blocked: tags(&entry.when, facts),
            entry: index,
        };
        match items.iter_mut().find(|old| old.name == item.name) {
            Some(old) if old.added.unwrap_or(u64::MAX) < item.added.unwrap_or(u64::MAX) => {
                *old = item;
            }
            Some(_) => {}
            None => items.push(item),
        }
    }
    Ok(items)
}

/// The values `when` lacks for this host, for each limit it has.
fn widening(when: &When, facts: &HostFacts) -> Widening {
    let lacks = |values: bool, has: bool| values && !has;
    Widening {
        host: lacks(!when.hosts.is_empty(), when.hosts.contains(&facts.id))
            .then(|| facts.id.clone()),
        os: lacks(!when.os.is_empty(), when.os.contains(&facts.os)).then_some(facts.os),
        arch: lacks(!when.arch.is_empty(), when.arch.contains(&facts.arch)).then_some(facts.arch),
        shells: Vec::new(),
    }
}

fn tags(when: &When, facts: &HostFacts) -> Option<Blocked> {
    let missing = !when.tags.is_empty() && !when.tags.iter().any(|tag| facts.tags.contains(tag));
    missing.then(|| Blocked::Tags(when.tags.clone()))
}

/// When each file in the repo was last added, from one `git log`.
fn added(repo_dir: &Path) -> Result<HashMap<String, u64>> {
    let log = git::run(
        repo_dir,
        &[
            "-c",
            "core.quotePath=false",
            "log",
            "--diff-filter=A",
            "--name-only",
            "--format=@%ct",
        ],
    )?;
    let mut added = HashMap::new();
    let mut at = 0;
    for line in log.lines().filter(|line| !line.is_empty()) {
        match line.strip_prefix('@').and_then(|time| time.parse().ok()) {
            Some(time) => at = time,
            // Newest first: a file added, removed and added again counts from the last time.
            None => {
                added.entry(line.to_string()).or_insert(at);
            }
        }
    }
    Ok(added)
}

/// `name` as `rsmt recent` shows it: a module's name, or a dotfile as
/// `~/…` however its path was typed.
pub fn shown(ctx: &Context, name: &str) -> String {
    let rel = name
        .strip_prefix("~/")
        .map(PathBuf::from)
        .or_else(|| {
            Path::new(name)
                .strip_prefix(&ctx.home)
                .ok()
                .map(Path::to_path_buf)
        })
        .or_else(|| name.starts_with('.').then(|| PathBuf::from(name)));
    match rel.and_then(|rel| dotfiles::clean(&rel.to_string_lossy())) {
        Some(rel) => format!("~/{}", rel.display()),
        None => name.to_string(),
    }
}

/// Whether the repo has a module or a dotfile by this `shown` name.
pub fn in_repo(ctx: &Context, shown: &str) -> Result<bool> {
    let repo_dir = ctx.paths.repo();
    if repo::module_files(&repo_dir)?
        .iter()
        .any(|path| repo::module_name(path) == shown)
    {
        return Ok(true);
    }
    let Some(rel) = shown.strip_prefix("~/") else {
        return Ok(false);
    };
    Ok(dotfiles::load(&repo_dir)?
        .dotfile
        .iter()
        .any(|entry| dotfiles::clean(&entry.path) == dotfiles::clean(rel)))
}

/// The edits that widen `items`, one per file.
pub fn plan(ctx: &Context, items: &[Item]) -> Result<Vec<FileChange>> {
    let repo_dir = ctx.paths.repo();
    let mut changes: Vec<FileChange> = Vec::new();
    for item in items {
        let path = repo_dir.join(&item.file);
        let at = match changes.iter().position(|change| change.path == path) {
            Some(at) => at,
            None => {
                let text = std::fs::read_to_string(&path).map_err(Error::read(&path))?;
                changes.push(FileChange {
                    path: path.clone(),
                    before: Some(text.clone()),
                    after: Some(text),
                });
                changes.len() - 1
            }
        };
        let change = &mut changes[at];
        let mut doc: DocumentMut = change
            .after
            .as_deref()
            .unwrap_or_default()
            .parse()
            .map_err(|err: toml_edit::TomlError| {
                Error::Usage(format!("{}: {}", path.display(), err.message().trim()))
            })?;
        match item.kind {
            Kind::Module => crate::scope::widen(doc.as_table_mut(), &item.widening),
            Kind::Dotfile => {
                if let Some(entry) = doc
                    .get_mut("dotfile")
                    .and_then(|list| list.as_array_of_tables_mut())
                    .and_then(|list| list.get_mut(item.entry))
                {
                    crate::scope::widen(entry, &item.widening);
                }
            }
        }
        change.after = Some(doc.to_string());
    }
    Ok(changes)
}

/// Write `changes`, load the result, and commit it as one change. A shell
/// that no longer starts puts the files back. The caller holds the adopt
/// lock.
pub fn include(ctx: &Context, items: &[Item], changes: &[FileChange]) -> Result<()> {
    let host = adopt::host_config(ctx)?;
    let mut written = false;
    let write = || -> Result<()> {
        for change in changes {
            let now = std::fs::read_to_string(&change.path).map_err(Error::read(&change.path))?;
            if Some(now) != change.before {
                return Err(Error::Usage(format!(
                    "{} changed while rosamta was reading it; try again",
                    change.path.display()
                )));
            }
        }
        written = true;
        for change in changes {
            crate::edit::write_atomic(&change.path, change.after.as_deref().unwrap_or_default())?;
        }
        Ok(())
    };
    if let Err(err) = apply::apply_after(ctx, write) {
        if written {
            for change in changes {
                let _ = crate::edit::write_atomic(
                    &change.path,
                    change.before.as_deref().unwrap_or_default(),
                );
            }
        }
        return Err(err);
    }
    let repo_dir = ctx.paths.repo();
    let files: Vec<String> = changes
        .iter()
        .map(|change| {
            let rel = change.path.strip_prefix(&repo_dir).unwrap_or(&change.path);
            rel.to_string_lossy().into_owned()
        })
        .collect();
    let files: Vec<&str> = files.iter().map(String::as_str).collect();
    let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
    let message = format!("include: {} on {}", names.join(", "), host.id);
    sync::commit_scanned(ctx, &files, Some(&message))?;
    if items.iter().any(|item| item.kind == Kind::Dotfile) {
        dotfiles::deploy::deploy(ctx)?;
    }
    Ok(())
}
