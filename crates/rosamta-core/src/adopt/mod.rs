//! Move content outside the hook in rc files into modules, so rc files hold
//! only the hook (ADR 0002). Every step can be undone: rc files are
//! snapshotted, repo changes are commits, and adoptions are recorded.

mod recognize;
mod secret;
mod segment;
pub(crate) mod store;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

pub use recognize::{Dialect, Recognizer, Structured, recognize, recognizers};
pub use secret::{looks_secret, secret_rule};
pub use segment::{Source, Watched, watched_files};
pub use store::{Adoption, QueueItem};

use crate::apply::{self, Context};
use crate::config::{AdoptPolicy, HostConfig, Module, OriginalLines, Phase, Shell};
use crate::error::{Error, Result};
use crate::hooks::{self, FileChange};
use crate::{generated, git, host, render};
use segment::Segment;
use store::{Queue, Store, now};

/// Where adopted modules go in the repo.
pub const ADOPTED_DIR: &str = "modules/adopted";

/// What adopting a segment does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Environment, PATH and aliases every shell loads.
    Structured,
    /// Code kept as written, for the shells that read the file.
    Snippet,
    /// Only comments: removed, with no module.
    Comment,
    /// Adopted before (an installer wrote it again): removed, no new module.
    Duplicate,
}

/// Which hosts a new module applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Host,
    All,
}

/// A segment that can be adopted, with what adopting it would create.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub file: PathBuf,
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub hash: String,
    pub kind: Kind,
    pub name: Option<String>,
    /// Repo-relative, or absolute for a module that stays on this host.
    pub module: Option<PathBuf>,
    /// Looks like a secret, so the module stays on this host.
    pub local: bool,
    #[serde(skip)]
    source: Source,
    #[serde(skip)]
    whole_file: bool,
    #[serde(skip)]
    content: Option<Module>,
    #[serde(skip)]
    order: u32,
}

impl Candidate {
    pub fn shell(&self) -> Shell {
        self.source.owner()
    }
}

/// Hold this while reading or writing adoption state and the repo, so the
/// CLI and the background service never interleave.
pub struct Lock(#[allow(dead_code)] std::fs::File);

pub fn lock(ctx: &Context) -> Result<Lock> {
    let dir = ctx.paths.state();
    std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
    let path = dir.join("lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(Error::write(&path))?;
    file.lock().map_err(Error::write(&path))?;
    Ok(Lock(file))
}

fn read(path: &Path) -> Result<String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(Error::Read {
            path: path.to_path_buf(),
            source: err,
        }),
    }
}

/// `~/.zshrc` for files under HOME.
pub fn display(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rel) => format!("~/{}", rel.display()),
        Err(_) => path.display().to_string(),
    }
}

pub(crate) fn host_config(ctx: &Context) -> Result<HostConfig> {
    let path = ctx.paths.host_config();
    if !path.exists() {
        return Err(Error::Usage(
            "this host is not set up; run `rsmt init` first".into(),
        ));
    }
    host::load(&path)
}

/// Every module in the repo and on this host only, by name.
fn existing_modules(ctx: &Context) -> Result<Vec<(String, Module)>> {
    let mut out = apply::load_modules(&ctx.paths.repo())?;
    out.extend(apply::load_local_modules(&ctx.paths)?);
    Ok(out)
}

/// Content outside the hook in every watched file whose shell is hooked.
pub fn scan(ctx: &Context) -> Result<Vec<Candidate>> {
    let installed = ctx.installed();
    let env = ctx.hook_env(&installed);
    let store = Store::new(ctx.paths.state());
    let adoptions = store.adoptions()?;
    let ignored = store.ignored()?;
    let existing = existing_modules(ctx)?;
    let mut taken: Vec<String> = existing.iter().map(|(name, _)| name.clone()).collect();
    let recognizers = recognizers();
    let mut out = Vec::new();
    for watched in segment::watched_files(&env) {
        // Moving content out of a file only works if the shell loads rosamta.
        if hooks::hooked_files(watched.source.owner(), &env)?.is_empty() {
            continue;
        }
        let text = read(&watched.path)?;
        let exe = installed
            .iter()
            .find(|found| found.shell == watched.source.owner())
            .map(|found| found.path.clone());
        let complete = |code: &str| {
            exe.as_deref()
                .is_none_or(|exe| generated::parses(exe, watched.source.owner(), code))
        };
        let segments: Vec<Segment> = if watched.whole_file {
            segment::whole(&text).into_iter().collect()
        } else {
            segment::segments(&text, &complete)
        };
        let mut order = adoptions
            .adoptions
            .iter()
            .filter(|adoption| adoption.order / 1000 == watched.rank)
            .map(|adoption| adoption.order)
            .max()
            .unwrap_or(watched.rank * 1000);
        // Structured PATH entries load in the env phase and are put back in
        // front at the end of `.zprofile`, while snippets keep their place.
        // A snippet may change PATH unseen (eval, source), so PATH changes
        // stay snippets where moving them would reorder PATH: after a snippet
        // in a late file, and anywhere in a login file that has one.
        let mut barrier = watched.phase == Phase::Login
            && segments.iter().any(|segment| {
                !segment.comment_only()
                    && structured(ctx, segment, &watched, &recognizers, false).is_none()
            });
        for segment in segments {
            let text = segment.text();
            let hash = segment::hash(&segment::normalize(&text, &ctx.home));
            // Lines rosamta or fish wrote themselves are not the user's.
            let seed = watched.path.ends_with(".bash_profile") && text == hooks::BASH_PROFILE_SEED;
            let fish_default =
                watched.path.ends_with("config.fish") && segment::fish_default(&text);
            if seed || fish_default || ignored.hashes.contains(&hash) {
                continue;
            }
            let mut candidate = Candidate {
                file: watched.path.clone(),
                start: segment.start,
                end: segment.end,
                text: text.clone(),
                hash: hash.clone(),
                kind: Kind::Comment,
                name: None,
                module: None,
                local: false,
                source: watched.source,
                whole_file: watched.whole_file,
                content: None,
                order: 0,
            };
            if let Some(adoption) = adoptions.adoptions.iter().find(|a| a.hash == hash) {
                candidate.kind = Kind::Duplicate;
                candidate.module = adoption.module.clone();
                out.push(candidate);
                continue;
            }
            if segment.comment_only() {
                out.push(candidate);
                continue;
            }
            let module = structured(ctx, &segment, &watched, &recognizers, barrier);
            if let Some(module) = &module
                && let Some((name, _)) = existing.iter().find(|(_, m)| covers(m, module))
            {
                candidate.kind = Kind::Duplicate;
                candidate.module = Some(PathBuf::from(name));
                out.push(candidate);
                continue;
            }
            let module = match module {
                Some(module) => {
                    candidate.kind = Kind::Structured;
                    module
                }
                None => {
                    candidate.kind = Kind::Snippet;
                    barrier |= watched.phase != Phase::Env;
                    snippet(ctx, &text, &watched)
                }
            };
            order += 1;
            let name = derive_name(&segment, &watched, &module, &ctx.home);
            let name = unique(&name, &mut taken);
            candidate.local = secret::looks_secret(&text);
            candidate.module = Some(if candidate.local {
                local_dir(ctx).join(format!("{name}.toml"))
            } else {
                PathBuf::from(format!("{ADOPTED_DIR}/{name}.toml"))
            });
            candidate.name = Some(name);
            candidate.order = order;
            candidate.content = Some(module);
            out.push(candidate);
        }
    }
    Ok(out)
}

fn local_dir(ctx: &Context) -> PathBuf {
    ctx.paths.local_modules()
}

/// The segment as a structured module, if every line is understood and
/// moving it to the env phase keeps its meaning.
fn structured(
    ctx: &Context,
    segment: &Segment,
    watched: &Watched,
    recognizers: &[Box<dyn Recognizer>],
    barrier: bool,
) -> Option<Module> {
    let dialect = match watched.source {
        Source::Profile | Source::Bash => Dialect::Posix,
        Source::Zsh => Dialect::Zsh,
        Source::Fish => Dialect::Fish,
    };
    let lines: Vec<&str> = segment
        .lines
        .iter()
        .map(|line| line.trim_end_matches(['\n', '\r']))
        .collect();
    let mut module = recognize(&lines, dialect, recognizers)?;
    if !render::check(&module).is_empty() {
        return None;
    }
    if watched.phase != Phase::Env {
        let touches_path = !module.path.prepend.is_empty() || !module.path.append.is_empty();
        if (barrier && touches_path) || reads_late_variables(&module) {
            return None;
        }
    }
    let home = ctx.home.to_string_lossy();
    let rewrite = |value: &mut String| *value = home_value(value, &home);
    module.env.values_mut().for_each(rewrite);
    module.path.prepend.iter_mut().for_each(rewrite);
    module.path.append.iter_mut().for_each(rewrite);
    module.aliases.values_mut().for_each(rewrite);
    Some(module)
}

/// `/Users/me/x` -> `$HOME/x` in a module value.
fn home_value(value: &str, home: &str) -> String {
    if home.len() < 2 {
        return value.to_string();
    }
    let mut out = String::new();
    let mut rest = value;
    while let Some(at) = rest.find(home) {
        let after = &rest[at + home.len()..];
        let boundary = after
            .chars()
            .next()
            .is_none_or(|next| !(next.is_alphanumeric() || "-_.".contains(next)));
        out.push_str(&rest[..at]);
        out.push_str(if boundary { "$HOME" } else { home });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Variables a late file's values read that the env phase may not have yet:
/// anything but HOME, USER, LOGNAME, PATH and the module's own variables.
fn reads_late_variables(module: &Module) -> bool {
    let known = ["HOME", "USER", "LOGNAME", "PATH"];
    let values = module
        .env
        .values()
        .chain(&module.path.prepend)
        .chain(&module.path.append);
    values
        .flat_map(|value| variables(value))
        .any(|name| !known.contains(&name.as_str()) && !module.env.contains_key(&name))
}

fn variables(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(at) = rest.find('$') {
        rest = &rest[at + 1..];
        let body = rest.strip_prefix('{').unwrap_or(rest);
        let len = body
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(body.len());
        if len > 0 {
            out.push(body[..len].to_string());
        }
    }
    out
}

fn snippet(ctx: &Context, text: &str, watched: &Watched) -> Module {
    let fish = watched.source == Source::Fish;
    let text = segment::rewrite_home(text, &ctx.home, fish);
    let mut module: Module = toml::from_str("").expect("an empty module parses");
    for shell in watched.source.shells() {
        let slot = match shell {
            Shell::Sh => &mut module.raw.sh,
            Shell::Bash => &mut module.raw.bash,
            Shell::Zsh => &mut module.raw.zsh,
            Shell::Fish => &mut module.raw.fish,
        };
        *slot = Some(text.clone());
    }
    if watched.phase != Phase::Late {
        module.phase = Some(watched.phase);
    }
    module
}

/// True when `existing` already sets everything `new` does.
fn covers(existing: &Module, new: &Module) -> bool {
    new.env
        .iter()
        .all(|(name, value)| existing.env.get(name) == Some(value))
        && new
            .aliases
            .iter()
            .all(|(name, value)| existing.aliases.get(name) == Some(value))
        && new
            .path
            .prepend
            .iter()
            .all(|dir| existing.path.prepend.contains(dir))
        && new
            .path
            .append
            .iter()
            .all(|dir| existing.path.append.contains(dir))
}

const GENERIC: [&str; 9] = [
    "local", "config", "bin", "share", "lib", "etc", "opt", "library", "shell",
];

/// From a marker, an `Added by` comment, a tool's path, then the file name.
fn derive_name(segment: &Segment, watched: &Watched, module: &Module, home: &Path) -> String {
    let file_stem = || {
        let name = watched
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        name.trim_start_matches('.')
            .trim_end_matches(".fish")
            .to_string()
    };
    if watched.whole_file {
        return sanitize(&file_stem());
    }
    if let Some(marker) = &segment.marker {
        return sanitize(marker);
    }
    let text = segment.text();
    for line in text.lines() {
        let lower = line.to_lowercase();
        if let Some(at) = lower.find("added by ") {
            let name = &line[at + "added by ".len()..];
            let name = name.split([':', '.', '(']).next().unwrap_or_default();
            if !sanitize(name).is_empty() {
                return sanitize(name);
            }
        }
    }
    let home = format!("{}/", home.display());
    for home in ["$HOME/", "${HOME}/", "~/", home.as_str()] {
        for (at, _) in text.match_indices(home) {
            let path = &text[at + home.len()..];
            let path = path
                .split(|c: char| c.is_whitespace() || "\"':;)$".contains(c))
                .next()
                .unwrap_or_default();
            let found = path
                .split('/')
                .map(|part| part.trim_start_matches('.'))
                .find(|part| !part.is_empty() && !GENERIC.contains(&part.to_lowercase().as_str()));
            if let Some(found) = found {
                return sanitize(found);
            }
        }
    }
    if let Some(at) = text.find("/bin/") {
        let tool: String = text[at + 5..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !tool.is_empty() {
            return sanitize(&tool);
        }
    }
    if let Some(name) = module.env.keys().chain(module.aliases.keys()).next() {
        return sanitize(name);
    }
    sanitize(&file_stem())
}

fn sanitize(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "adopted".into()
    } else {
        out
    }
}

fn unique(name: &str, taken: &mut Vec<String>) -> String {
    let mut candidate = name.to_string();
    let mut n = 2;
    while taken.contains(&candidate) {
        candidate = format!("{name}-{n}");
        n += 1;
    }
    taken.push(candidate.clone());
    candidate
}

/// The module file as TOML, written by hand for readable raw blocks.
pub fn module_toml(module: &Module, origin: &str) -> String {
    let string = |value: &str| toml::Value::String(value.into()).to_string();
    let key = |name: &str| {
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            name.to_string()
        } else {
            string(name)
        }
    };
    let list = |values: &[String]| {
        let items: Vec<String> = values.iter().map(|value| string(value)).collect();
        format!("[{}]", items.join(", "))
    };
    let mut out = format!("# Adopted from {origin}.\n");
    if let Some(phase) = module.phase {
        let phase = toml::Value::try_from(phase).expect("phase serializes");
        writeln!(out, "phase = {phase}").unwrap();
    }
    writeln!(out, "order = {}", module.order).unwrap();
    if !module.when.hosts.is_empty() {
        writeln!(out, "\n[when]\nhosts = {}", list(&module.when.hosts)).unwrap();
    }
    if !module.env.is_empty() {
        out.push_str("\n[env]\n");
        for (name, value) in &module.env {
            writeln!(out, "{} = {}", key(name), string(value)).unwrap();
        }
    }
    if !module.path.prepend.is_empty() || !module.path.append.is_empty() {
        out.push_str("\n[path]\n");
        if !module.path.prepend.is_empty() {
            writeln!(out, "prepend = {}", list(&module.path.prepend)).unwrap();
        }
        if !module.path.append.is_empty() {
            writeln!(out, "append = {}", list(&module.path.append)).unwrap();
        }
    }
    if !module.aliases.is_empty() {
        out.push_str("\n[aliases]\n");
        for (name, value) in &module.aliases {
            writeln!(out, "{} = {}", key(name), string(value)).unwrap();
        }
    }
    let raw = [
        ("sh", &module.raw.sh),
        ("bash", &module.raw.bash),
        ("zsh", &module.raw.zsh),
        ("fish", &module.raw.fish),
    ];
    if raw.iter().any(|(_, text)| text.is_some()) {
        out.push_str("\n[raw]\n");
        for (shell, text) in raw {
            let Some(text) = text else { continue };
            // A literal block reads as the original code; fall back to an
            // escaped string for text a literal block cannot hold.
            let literal = !text.contains("'''")
                && text
                    .chars()
                    .all(|c| !c.is_control() || c == '\n' || c == '\t')
                && text.ends_with('\n');
            if literal {
                writeln!(out, "{shell} = '''\n{text}'''").unwrap();
            } else {
                writeln!(out, "{shell} = {}", string(text)).unwrap();
            }
        }
    }
    out
}

/// What adopting a list of candidates writes.
#[derive(Debug, Default, Serialize)]
pub struct Plan {
    pub files: Vec<FileChange>,
    /// New module files: absolute path and content.
    pub modules: Vec<(PathBuf, String)>,
    #[serde(skip)]
    records: Vec<Adoption>,
    #[serde(skip)]
    candidates: Vec<Candidate>,
}

/// Work out the rc file edits and module files, writing nothing.
pub fn plan(ctx: &Context, candidates: &[Candidate], scope: Scope) -> Result<Plan> {
    let host = host_config(ctx)?;
    let mut plan = Plan::default();
    let mut files: Vec<&Path> = candidates.iter().map(|c| c.file.as_path()).collect();
    files.dedup();
    for file in files {
        let ours: Vec<&Candidate> = candidates.iter().filter(|c| c.file == file).collect();
        let before = read(file)?;
        let (after, records) = edit_file(ctx, &before, &ours, host.adopt_original)?;
        plan.files.push(FileChange {
            path: file.to_path_buf(),
            before: file.exists().then_some(before),
            after,
        });
        plan.records.extend(records);
    }
    for candidate in candidates {
        let (Some(content), Some(module)) = (&candidate.content, &candidate.module) else {
            continue;
        };
        let mut content = content.clone();
        content.order = candidate.order;
        if scope == Scope::Host {
            content.when.hosts = vec![host.id.clone()];
        }
        let origin = format!("{} on {}", display(&candidate.file, &ctx.home), host.id);
        let path = if candidate.local {
            module.clone()
        } else {
            ctx.paths.repo().join(module)
        };
        plan.modules.push((path, module_toml(&content, &origin)));
    }
    plan.candidates = candidates.to_vec();
    Ok(plan)
}

/// The file with every candidate's lines removed (or commented out), and a
/// record per candidate of where they were.
fn edit_file(
    ctx: &Context,
    text: &str,
    candidates: &[&Candidate],
    mode: OriginalLines,
) -> Result<(Option<String>, Vec<Adoption>)> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut replaced: Vec<Option<String>> = vec![None; lines.len()];
    let mut removed = vec![false; lines.len()];
    let mut ranges = Vec::new();
    for candidate in candidates {
        let (first, last) = (candidate.start - 1, candidate.end - 1);
        if lines.get(first..=last).map(|range| range.concat()) != Some(candidate.text.clone())
            && lines
                .get(first..=last)
                .map(|range| format!("{}\n", range.concat()))
                != Some(candidate.text.clone())
        {
            return Err(Error::Usage(format!(
                "{} changed while rosamta was reading it; try again",
                candidate.file.display()
            )));
        }
        let comment = mode == OriginalLines::Comment
            && matches!(
                candidate.kind,
                Kind::Structured | Kind::Snippet | Kind::Duplicate
            );
        let mut start = first;
        if comment {
            let target = candidate
                .module
                .as_deref()
                .map(|module| display(module, &ctx.home))
                .unwrap_or_default();
            let mut block = format!("{} → {target}\n", segment::COMMENTED);
            for line in &lines[first..=last] {
                let body = line.trim_end_matches(['\n', '\r']);
                if body.is_empty() {
                    block.push_str("#\n");
                } else {
                    writeln!(block, "# {body}").unwrap();
                }
            }
            replaced[first] = Some(block);
        } else if first > 0 && lines[first - 1].trim().is_empty() && !removed[first - 1] {
            // Take the blank line above along, so files do not fill with gaps.
            start = first - 1;
        }
        removed[start..=last].fill(true);
        ranges.push((candidate, start, last));
    }
    let kept: Vec<usize> = (0..lines.len()).filter(|&index| !removed[index]).collect();
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        if let Some(block) = &replaced[index] {
            out.push_str(block);
        } else if !removed[index] {
            out.push_str(line);
        }
    }
    let records = ranges
        .into_iter()
        .map(|(candidate, start, last)| {
            let text = |indexes: &[usize]| -> Vec<String> {
                indexes
                    .iter()
                    .map(|&index| lines[index].to_string())
                    .collect()
            };
            let above: Vec<usize> = kept
                .iter()
                .copied()
                .filter(|&index| index < start)
                .collect();
            let below: Vec<usize> = kept.iter().copied().filter(|&index| index > last).collect();
            let original_after: Vec<usize> = (last + 1..lines.len().min(last + 3)).collect();
            Adoption {
                id: 0,
                shell: candidate.shell(),
                file: candidate.file.clone(),
                line: start + 1,
                before: text(&(start.saturating_sub(2)..start).collect::<Vec<_>>()),
                after: text(&original_after),
                kept_before: text(&above[above.len().saturating_sub(2)..]),
                kept_after: text(&below[..below.len().min(2)]),
                removed: lines[start..=last].concat(),
                replacement: replaced[candidate.start - 1].clone(),
                hash: candidate.hash.clone(),
                name: candidate.name.clone(),
                module: candidate.module.clone(),
                local: candidate.local,
                commit: None,
                order: candidate.order,
                at: now(),
            }
        })
        .collect();
    let whole = candidates.iter().any(|candidate| candidate.whole_file);
    let after = if whole && out.trim().is_empty() {
        None
    } else {
        Some(out)
    };
    Ok((after, records))
}

fn write_text(path: &Path, text: Option<&str>) -> Result<()> {
    match text {
        Some(text) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
            }
            crate::edit::write_atomic(path, text)
        }
        None => match std::fs::remove_file(path) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(Error::Write {
                path: path.to_path_buf(),
                source: err,
            }),
            _ => Ok(()),
        },
    }
}

/// Adopt: snapshot rc files, write modules and rc files, apply, then commit
/// each adoption. If apply fails, every file goes back as it was.
pub fn adopt(ctx: &Context, candidates: &[Candidate], scope: Scope) -> Result<Vec<Adoption>> {
    let plan = plan(ctx, candidates, scope)?;
    execute(ctx, plan)
}

/// Carry out a plan from [`plan`].
pub fn execute(ctx: &Context, plan: Plan) -> Result<Vec<Adoption>> {
    if plan.candidates.is_empty() {
        return Ok(Vec::new());
    }
    let host = host_config(ctx)?;
    let snapshots = crate::snapshot::Snapshots::new(ctx.paths.state(), &ctx.home);
    let changed: Vec<PathBuf> = plan
        .files
        .iter()
        .filter(|change| !change.is_noop())
        .map(|change| change.path.clone())
        .collect();
    snapshots.save(&changed, "adopt")?;
    let write = || -> Result<()> {
        for (path, text) in &plan.modules {
            write_text(path, Some(text))?;
        }
        for change in &plan.files {
            write_text(&change.path, change.after.as_deref())?;
        }
        Ok(())
    };
    if let Err(err) = apply::apply_after(ctx, write) {
        for change in &plan.files {
            let _ = write_text(&change.path, change.before.as_deref());
        }
        for (path, _) in &plan.modules {
            let _ = std::fs::remove_file(path);
        }
        return Err(err);
    }
    let store = Store::new(ctx.paths.state());
    let mut adoptions = store.adoptions()?;
    let mut done = Vec::new();
    // After a signing failure, the rest stays uncommitted too.
    let mut can_commit = true;
    for mut record in plan.records {
        let candidate = plan
            .candidates
            .iter()
            .find(|candidate| candidate.hash == record.hash && candidate.file == record.file)
            .expect("each record comes from a candidate");
        if matches!(candidate.kind, Kind::Comment | Kind::Duplicate) {
            done.push(record);
            continue;
        }
        if can_commit
            && !record.local
            && let Some(module) = &record.module
        {
            let repo_dir = ctx.paths.repo();
            let rel = module.to_string_lossy();
            let message = commit_message(candidate, &ctx.home, &host.id);
            match git::commit(&repo_dir, &message, &[&rel])? {
                git::Commit::Created => {
                    record.commit = Some(
                        git::run(&repo_dir, &["rev-parse", "HEAD"])?
                            .trim()
                            .to_string(),
                    )
                }
                git::Commit::Clean => {}
                git::Commit::SigningFailed(reason) => {
                    crate::sync::signing_blocked(ctx, &reason)?;
                    can_commit = false;
                }
            }
        }
        adoptions.next_id = adoptions.next_id.max(1);
        record.id = adoptions.next_id;
        adoptions.next_id += 1;
        adoptions.adoptions.push(record.clone());
        done.push(record);
    }
    store.save_adoptions(&adoptions)?;
    let mut queue = store.queue()?;
    let listed = queue.items.len();
    queue.items.retain(|item| {
        !plan
            .candidates
            .iter()
            .any(|candidate| candidate.file == item.file && candidate.hash == item.hash)
    });
    if queue.items.len() != listed {
        store.save_queue(&queue)?;
    }
    update_notice(&store, &host, queue.items.len())?;
    Ok(done)
}

/// `adopt(zsh): add bun PATH from ~/.zshrc on mbp`.
fn commit_message(candidate: &Candidate, home: &Path, host: &str) -> String {
    let name = candidate.name.as_deref().unwrap_or("adopted");
    let what = match &candidate.content {
        Some(module) if candidate.kind == Kind::Structured => {
            let mut kinds = Vec::new();
            if !module.env.is_empty() {
                kinds.push("env");
            }
            if !module.path.prepend.is_empty() || !module.path.append.is_empty() {
                kinds.push("PATH");
            }
            if !module.aliases.is_empty() {
                kinds.push("aliases");
            }
            match kinds.split_last() {
                Some((last, [])) => last.to_string(),
                Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
                None => String::new(),
            }
        }
        _ => "snippet".into(),
    };
    format!(
        "adopt({}): add {name} {what} from {} on {host}",
        candidate.shell(),
        display(&candidate.file, home)
    )
}

/// What a review pass did.
#[derive(Debug, Default, Serialize)]
pub struct Reviewed {
    pub items: Vec<QueueItem>,
    /// Adopted without asking: the policy is `auto`, or an installer wrote
    /// something already adopted.
    pub adopted: Vec<Adoption>,
}

/// Scan and act on the host's adopt policy: `auto` adopts, `ask` queues,
/// `off` does nothing. Content adopted before is removed under `auto` and `ask`.
pub fn refresh(ctx: &Context) -> Result<Reviewed> {
    let host = host_config(ctx)?;
    let store = Store::new(ctx.paths.state());
    if host.adopt == AdoptPolicy::Off {
        store.set_notice(None)?;
        return Ok(Reviewed::default());
    }
    let candidates = scan(ctx)?;
    let (now_adopt, ask): (Vec<Candidate>, Vec<Candidate>) =
        candidates.into_iter().partition(|candidate| {
            host.adopt == AdoptPolicy::Auto || candidate.kind == Kind::Duplicate
        });
    let adopted = adopt(ctx, &now_adopt, Scope::Host)?;
    let old = store.queue()?;
    // Dotfile items are kept up to date by deploying.
    let mut queue = Queue {
        next_id: old.next_id.max(1),
        items: old
            .items
            .iter()
            .filter(|item| item.dotfile.is_some())
            .cloned()
            .collect(),
    };
    for candidate in &ask {
        let kept = old
            .items
            .iter()
            .find(|item| item.file == candidate.file && item.hash == candidate.hash);
        let (id, detected_at) = match kept {
            Some(item) => (item.id, item.detected_at),
            None => {
                queue.next_id += 1;
                (queue.next_id - 1, now())
            }
        };
        queue.items.push(QueueItem {
            id,
            file: candidate.file.clone(),
            start: candidate.start,
            end: candidate.end,
            detected_at,
            hash: candidate.hash.clone(),
            text: candidate.text.clone(),
            module: candidate.module.clone(),
            local: candidate.local,
            dotfile: None,
        });
    }
    store.save_queue(&queue)?;
    update_notice(&store, &host, queue.items.len())?;
    Ok(Reviewed {
        items: queue.items,
        adopted,
    })
}

pub(crate) fn update_notice(store: &Store, host: &HostConfig, pending: usize) -> Result<()> {
    if pending == 0 || !host.shell_notice {
        return store.set_notice(None);
    }
    let noun = if pending == 1 { "change" } else { "changes" };
    store.set_notice(Some(&format!(
        "rosamta: {pending} {noun} to review. Run: rsmt review"
    )))
}

pub(crate) fn take_item(ctx: &Context, id: u32) -> Result<(QueueItem, Queue)> {
    let store = Store::new(ctx.paths.state());
    let mut queue = store.queue()?;
    let index = queue
        .items
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| Error::Usage(format!("no review item {id}; run `rsmt review`")))?;
    let item = queue.items.remove(index);
    Ok((item, queue))
}

/// `rsmt review accept <id>`.
pub fn accept(ctx: &Context, id: u32, scope: Scope) -> Result<Vec<Adoption>> {
    let (item, queue) = take_item(ctx, id)?;
    if let Some(dotfile) = &item.dotfile {
        crate::dotfiles::review::keep(ctx, id, dotfile, scope)?;
        return Ok(Vec::new());
    }
    let candidate = scan(ctx)?
        .into_iter()
        .find(|candidate| candidate.file == item.file && candidate.hash == item.hash)
        .ok_or_else(|| {
            Error::Usage(format!(
                "review item {id} changed since it was listed; run `rsmt review`"
            ))
        })?;
    let adopted = adopt(ctx, &[candidate], scope)?;
    let store = Store::new(ctx.paths.state());
    store.save_queue(&queue)?;
    update_notice(&store, &host_config(ctx)?, queue.items.len())?;
    Ok(adopted)
}

/// `rsmt review ignore <id>`: keep it in the rc file and stop asking,
/// until its content changes.
pub fn ignore(ctx: &Context, id: u32) -> Result<QueueItem> {
    let (item, queue) = take_item(ctx, id)?;
    if let Some(dotfile) = &item.dotfile {
        crate::dotfiles::review::ignore(ctx, id, dotfile)?;
        return Ok(item);
    }
    let store = Store::new(ctx.paths.state());
    let mut ignored = store.ignored()?;
    if !ignored.hashes.contains(&item.hash) {
        ignored.hashes.push(item.hash.clone());
    }
    store.save_ignored(&ignored)?;
    store.save_queue(&queue)?;
    update_notice(&store, &host_config(ctx)?, queue.items.len())?;
    Ok(item)
}

/// What undoing one adoption writes.
#[derive(Debug, Serialize)]
pub struct Undo {
    pub adoption: Adoption,
    pub file: FileChange,
    /// The module file that goes away, with its content.
    pub module: Option<(PathBuf, String)>,
}

/// Plan `rsmt restore <id>`: put the lines back where they were, and remove
/// the module.
pub fn plan_undo(ctx: &Context, id: u32) -> Result<Undo> {
    let store = Store::new(ctx.paths.state());
    let adoption = store
        .adoptions()?
        .adoptions
        .into_iter()
        .find(|adoption| adoption.id == id)
        .ok_or_else(|| Error::Usage(format!("no adoption {id}")))?;
    let exists = adoption.file.exists();
    let text = read(&adoption.file)?;
    let after = put_back(&text, &adoption);
    let module = match &adoption.module {
        Some(module) if adoption.local => Some(module.clone()),
        Some(module) => Some(ctx.paths.repo().join(module)),
        None => None,
    }
    .filter(|path| path.exists())
    .map(|path| {
        let text = read(&path)?;
        Ok::<_, Error>((path, text))
    })
    .transpose()?;
    Ok(Undo {
        file: FileChange {
            path: adoption.file.clone(),
            before: exists.then_some(text),
            after: Some(after),
        },
        adoption,
        module,
    })
}

/// The file with the adopted lines back in place.
fn put_back(text: &str, adoption: &Adoption) -> String {
    if let Some(replacement) = &adoption.replacement
        && text.contains(replacement.as_str())
    {
        return text.replacen(replacement.as_str(), &adoption.removed, 1);
    }
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let matches_at = |at: usize, context: &[String]| {
        lines.get(at..at + context.len()).is_some_and(|window| {
            window
                .iter()
                .zip(context)
                .all(|(line, want)| line.trim_end() == want.trim_end())
        })
    };
    // The original neighbours first (other adopted lines may be back by
    // now), then the lines that stayed, then above the hook.
    let find = |before: &[String], after: &[String]| {
        let both = (before.len()..=lines.len())
            .find(|&at| matches_at(at - before.len(), before) && matches_at(at, after));
        let only_before = || {
            (before.len()..=lines.len())
                .find(|&at| !before.is_empty() && matches_at(at - before.len(), before))
        };
        let only_after = || (0..lines.len()).find(|&at| !after.is_empty() && matches_at(at, after));
        both.or_else(only_before).or_else(only_after)
    };
    let hook = || hooks::find_block(text).map(|(start, _)| text[..start].matches('\n').count());
    let at = find(&adoption.before, &adoption.after)
        .or_else(|| find(&adoption.kept_before, &adoption.kept_after))
        .or_else(hook)
        .unwrap_or(lines.len());
    let mut out: String = lines[..at].concat();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&adoption.removed);
    out.push_str(&lines[at..].concat());
    out
}

/// Undo one adoption. The lines stay in the rc file from now on: their
/// content is added to the ignored list.
pub fn undo(ctx: &Context, plan: Undo) -> Result<()> {
    let host = host_config(ctx)?;
    let snapshots = crate::snapshot::Snapshots::new(ctx.paths.state(), &ctx.home);
    snapshots.save(
        std::slice::from_ref(&plan.file.path),
        &format!("restore adoption {}", plan.adoption.id),
    )?;
    let write = || -> Result<()> {
        write_text(&plan.file.path, plan.file.after.as_deref())?;
        if let Some((path, _)) = &plan.module {
            write_text(path, None)?;
        }
        Ok(())
    };
    if let Err(err) = apply::apply_after(ctx, write) {
        let _ = write_text(&plan.file.path, plan.file.before.as_deref());
        if let Some((path, text)) = &plan.module {
            let _ = write_text(path, Some(text));
        }
        return Err(err);
    }
    if let (Some(module), false) = (&plan.adoption.module, plan.adoption.local) {
        let rel = module.to_string_lossy();
        let name = plan.adoption.name.as_deref().unwrap_or("adopted");
        let message = format!(
            "restore({}): put back {name} in {} on {}",
            plan.adoption.shell,
            display(&plan.adoption.file, &ctx.home),
            host.id
        );
        if let git::Commit::SigningFailed(reason) =
            git::commit(&ctx.paths.repo(), &message, &[&rel])?
        {
            crate::sync::signing_blocked(ctx, &reason)?;
        }
    }
    let store = Store::new(ctx.paths.state());
    let mut adoptions = store.adoptions()?;
    adoptions
        .adoptions
        .retain(|adoption| adoption.id != plan.adoption.id);
    store.save_adoptions(&adoptions)?;
    let mut ignored = store.ignored()?;
    if !ignored.hashes.contains(&plan.adoption.hash) {
        ignored.hashes.push(plan.adoption.hash.clone());
    }
    store.save_ignored(&ignored)
}

pub fn queue(ctx: &Context) -> Result<Vec<QueueItem>> {
    Ok(Store::new(ctx.paths.state()).queue()?.items)
}

pub fn notice_path(ctx: &Context) -> PathBuf {
    Store::new(ctx.paths.state()).notice_path()
}

/// For tests and the GUI: the adoption records.
pub fn adoptions(ctx: &Context) -> Result<Vec<Adoption>> {
    Ok(Store::new(ctx.paths.state()).adoptions()?.adoptions)
}
