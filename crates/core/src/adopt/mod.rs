//! Move content outside the hook in rc files into modules, so rc files hold
//! only the hook (ADR 0002). Every step can be undone: rc files are
//! snapshotted, repo changes are commits, and adoptions are recorded.

mod recognize;
mod secret;
mod segment;
mod shared;
pub(crate) mod store;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

pub use recognize::{Dialect, Recognizer, Structured, recognize, recognizers};
pub use secret::{looks_secret, secret_rule};
pub use segment::{Source, Watched, watched_files};
pub use store::{Adoption, Limits, QueueItem};

use crate::apply::{self, Context};
use crate::config::{AdoptPolicy, HostConfig, HostFacts, Module, OriginalLines, Phase, Shell};
use crate::error::{Error, Result};
use crate::hooks::{self, FileChange};
use crate::{generated, git, host, render, repo};
use segment::Segment;
use store::{Queue, Store, now};

/// Where adopted modules go in the repo.
pub const ADOPTED_DIR: &str = "modules/adopted";

/// What adopting a segment does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Environment, PATH and aliases, for the shells that read the file.
    Structured,
    /// Code kept as written, for the shells that read the file.
    Snippet,
    /// Only comments: removed, with no module.
    Comment,
    /// Adopted before (an installer wrote it again): removed, no new module.
    Duplicate,
    /// What another host's module does: this host joins that module.
    Include,
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
    shells: Vec<Shell>,
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

/// Fails on a file that is not UTF-8, so its bytes are never rewritten.
fn read(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
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

/// Content outside the hook in every watched file whose shell is hooked.
pub fn scan(ctx: &Context) -> Result<Vec<Candidate>> {
    let installed = ctx.installed();
    let env = ctx.hook_env(&installed);
    let store = Store::new(ctx.paths.state());
    let adoptions = store.adoptions()?;
    let ignored = store.ignored()?;
    let repo_modules = apply::load_modules(&ctx.paths.repo())?;
    let mut existing = repo_modules.clone();
    existing.extend(apply::load_local_modules(&ctx.paths)?);
    let mut taken: Vec<String> = existing.iter().map(|(name, _)| name.clone()).collect();
    let facts = HostFacts::current(&host_config(ctx)?);
    // Only modules that load on this host stand in for its lines.
    let loaded = render::select(existing, &facts);
    let module_files = repo::module_files(&ctx.paths.repo())?;
    let recognizers = recognizers();
    let mut out = Vec::new();
    for watched in segment::watched_files(&env) {
        // Moving content out of a file only works if the shell loads rosamta.
        if hooks::hooked_files(watched.source.owner(), &env)?.is_empty() {
            continue;
        }
        let text = match read(&watched.path) {
            Err(Error::Read { source, .. }) if source.kind() == std::io::ErrorKind::InvalidData => {
                continue;
            }
            text => text?,
        };
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
        // Structured env and PATH load in the env phase (PATH is put back in
        // front at the end of `.zprofile`), while snippets keep their place.
        // A snippet may change either unseen (eval, source), so they stay
        // snippets where moving them would change the order: after a snippet
        // or ignored lines in a late file, and anywhere in a login file that
        // has a snippet.
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
            if seed || fish_default {
                continue;
            }
            if ignored.hashes.contains(&hash) {
                // Ignored lines stay and run in place, like a snippet.
                barrier |= watched.phase != Phase::Env && !segment.comment_only();
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
                shells: watched.shells.clone(),
                whole_file: watched.whole_file,
                content: None,
                order: 0,
            };
            let shells = &watched.shells[..];
            let recorded = adoptions.adoptions.iter().filter(|a| a.hash == hash);
            if segment.comment_only() {
                // Comments adopted from this file before run nothing; drop them again.
                if let Some(adoption) = recorded.into_iter().find(|a| a.file == watched.path) {
                    candidate.kind = Kind::Duplicate;
                    candidate.module = adoption.module.clone();
                    candidate.local = adoption.local;
                }
                out.push(candidate);
                continue;
            }
            let module = structured(ctx, &segment, &watched, &recognizers, barrier);
            if let Some(module) = &module
                && settles(&loaded, shells, module)
                && let Some((name, _)) = loaded
                    .iter()
                    .find(|(_, m)| runs_in(m, shells) && covers(m, module))
            {
                candidate.kind = Kind::Duplicate;
                // Where restore finds it: in the repo, else this host's own.
                candidate.module = module_files
                    .iter()
                    .find(|path| repo::module_name(path) == *name)
                    .and_then(|path| path.strip_prefix(ctx.paths.repo()).ok())
                    .map(Path::to_path_buf);
                if candidate.module.is_none() {
                    candidate.local = true;
                    candidate.module = Some(local_dir(ctx).join(format!("{name}.toml")));
                }
                out.push(candidate);
                continue;
            }
            // Code adopted before is a duplicate only while its module still
            // loads here and holds that code, comments and blank lines aside
            // as when another host's module was joined.
            let code = snippet(ctx, &text, &watched);
            let still_held = |adoption: &&Adoption| {
                loaded.iter().any(|(name, m)| {
                    adoption
                        .module
                        .as_deref()
                        .is_some_and(|path| repo::module_name(path) == *name)
                        && runs_in(m, shells)
                        && shells.iter().all(|&shell| {
                            m.raw.get(shell).map(|text| shared::code(shell, text))
                                == code.raw.get(shell).map(|text| shared::code(shell, text))
                        })
                })
            };
            if let Some(adoption) = recorded.into_iter().find(still_held) {
                candidate.kind = Kind::Duplicate;
                candidate.module = adoption.module.clone();
                candidate.local = adoption.local;
                out.push(candidate);
                continue;
            }
            let mut module = match module {
                Some(module) => {
                    candidate.kind = Kind::Structured;
                    module
                }
                None => {
                    candidate.kind = Kind::Snippet;
                    barrier |= watched.phase != Phase::Env;
                    code
                }
            };
            // Only the shells that read the file ran it before.
            module.shells = shells.to_vec();
            let local = secret::looks_secret(&text);
            if let Some((name, old)) =
                shared::find(&repo_modules, &module, &facts, order).filter(|_| !local)
            {
                // What this file adopts next runs after the joined module.
                order = old.order;
                candidate.kind = Kind::Include;
                candidate.module = module_files
                    .iter()
                    .find(|path| repo::module_name(path) == *name)
                    .and_then(|path| path.strip_prefix(ctx.paths.repo()).ok())
                    .map(Path::to_path_buf);
                candidate.name = Some(name.clone());
                out.push(candidate);
                continue;
            }
            order += 1;
            let name = derive_name(&segment, &watched, &module, &ctx.home);
            let name = unique(&name, &mut taken);
            candidate.local = local;
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
        let moves_to_env = !module.env.is_empty()
            || !module.path.prepend.is_empty()
            || !module.path.append.is_empty();
        if (barrier && moves_to_env) || reads_late_variables(&module) {
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
/// anything but HOME, USER, LOGNAME, PATH and the module's other variables.
/// `X="a:$X"` reads the old X, which is not the module's.
fn reads_late_variables(module: &Module) -> bool {
    let known = ["HOME", "USER", "LOGNAME", "PATH"];
    let env = module
        .env
        .iter()
        .map(|(name, value)| (Some(name.as_str()), value));
    let path = module
        .path
        .prepend
        .iter()
        .chain(&module.path.append)
        .map(|value| (None, value));
    env.chain(path).any(|(own, value)| {
        variables(value).iter().any(|name| {
            !known.contains(&name.as_str())
                && (own == Some(name.as_str()) || !module.env.contains_key(name))
        })
    })
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
    for shell in &watched.shells {
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

/// Whether `module` does its work in each of `shells`: structured content
/// wherever the module renders, raw code only where it is written.
fn runs_in(module: &Module, shells: &[Shell]) -> bool {
    let reached = module.reaches();
    shells.iter().all(|shell| reached.contains(shell))
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
        && in_order(&existing.path.prepend, &new.path.prepend)
        && in_order(&existing.path.append, &new.path.append)
}

/// Whether `want` appears in `have` in the same order, gaps allowed.
fn in_order(have: &[String], want: &[String]) -> bool {
    let mut have = have.iter();
    want.iter().all(|dir| have.any(|entry| entry == dir))
}

/// True when the modules that load here end up with each variable and alias
/// `new` sets as `new` sets it: a later module may set another value.
fn settles(loaded: &[(String, Module)], shells: &[Shell], new: &Module) -> bool {
    let last = |get: &dyn Fn(&Module) -> Option<&String>| {
        loaded
            .iter()
            .filter(|(_, module)| runs_in(module, shells))
            .filter_map(|(_, module)| get(module))
            .last()
    };
    new.env
        .iter()
        .all(|(name, value)| last(&|module| module.env.get(name)) == Some(value))
        && new
            .aliases
            .iter()
            .all(|(name, value)| last(&|module| module.aliases.get(name)) == Some(value))
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
    let names = |values: Vec<toml::Value>| {
        let items: Vec<String> = values.iter().map(toml::Value::to_string).collect();
        format!("[{}]", items.join(", "))
    };
    fn value<T: Serialize>(value: T) -> toml::Value {
        toml::Value::try_from(value).expect("an enum serializes")
    }
    let mut out = format!("# Adopted from {origin}.\n");
    if let Some(phase) = module.phase {
        writeln!(out, "phase = {}", value(phase)).unwrap();
    }
    writeln!(out, "order = {}", module.order).unwrap();
    if !module.shells.is_empty() {
        let shells = module.shells.iter().map(|&shell| value(shell)).collect();
        writeln!(out, "shells = {}", names(shells)).unwrap();
    }
    if !module.when.hosts.is_empty() || !module.when.os.is_empty() {
        out.push_str("\n[when]\n");
        if !module.when.hosts.is_empty() {
            writeln!(out, "hosts = {}", list(&module.when.hosts)).unwrap();
        }
        if !module.when.os.is_empty() {
            let os = module.when.os.iter().map(|&os| value(os)).collect();
            writeln!(out, "os = {}", names(os)).unwrap();
        }
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
    /// Other hosts' modules this host joins.
    pub edits: Vec<FileChange>,
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
    let facts = HostFacts::current(&host);
    for candidate in candidates {
        if candidate.kind == Kind::Include
            && let Some(module) = &candidate.module
        {
            let limits = join(
                &mut plan.edits,
                &ctx.paths.repo().join(module),
                &facts,
                scope,
            )?;
            for record in plan.records.iter_mut() {
                if record.file == candidate.file && record.hash == candidate.hash {
                    record.limits = limits.clone();
                }
            }
            continue;
        }
        let (Some(content), Some(module)) = (&candidate.content, &candidate.module) else {
            continue;
        };
        let mut content = content.clone();
        content.order = candidate.order;
        if scope == Scope::Host {
            content.when.hosts = vec![host.id.clone()];
            content.when.os = vec![facts.os];
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

/// Add this host (and its OS, where the module lists one) to another
/// host's module, or drop both limits for every host, returning what they
/// were. Several candidates may join the same module.
fn join(
    edits: &mut Vec<FileChange>,
    path: &Path,
    facts: &HostFacts,
    scope: Scope,
) -> Result<Option<Limits>> {
    let at = match edits.iter().position(|edit| edit.path == path) {
        Some(at) => at,
        None => {
            let text = read(path)?;
            edits.push(FileChange {
                path: path.to_path_buf(),
                before: Some(text.clone()),
                after: Some(text),
            });
            edits.len() - 1
        }
    };
    let edit = &mut edits[at];
    let limits = match scope {
        Scope::Host => None,
        Scope::All => {
            let before = edit.before.as_deref().unwrap_or_default();
            let old: Module = toml::from_str(before).map_err(|err| {
                Error::Usage(format!("{}: {}", path.display(), err.message().trim()))
            })?;
            Some(Limits {
                hosts: old.when.hosts,
                os: old.when.os,
            })
        }
    };
    let text = edit.after.as_deref().unwrap_or_default();
    edit.after = Some(edit_module(path, text, |doc| match scope {
        Scope::Host => crate::scope::widen(
            doc,
            &crate::scope::Widening {
                host: Some(facts.id.clone()),
                os: Some(facts.os),
                ..Default::default()
            },
        ),
        Scope::All => crate::scope::every_host(doc),
    })?);
    Ok(limits)
}

fn edit_module(
    path: &Path,
    text: &str,
    change: impl FnOnce(&mut dyn toml_edit::TableLike),
) -> Result<String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|err: toml_edit::TomlError| {
        Error::Usage(format!("{}: {}", path.display(), err.message().trim()))
    })?;
    change(doc.as_table_mut());
    Ok(doc.to_string())
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
            return Err(changed(&candidate.file));
        }
        let comment = mode == OriginalLines::Comment
            && matches!(
                candidate.kind,
                Kind::Structured | Kind::Snippet | Kind::Duplicate | Kind::Include
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
                include: candidate.kind == Kind::Include,
                limits: None,
                duplicate: candidate.kind == Kind::Duplicate,
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

fn changed(path: &Path) -> Error {
    Error::Usage(format!(
        "{} changed while rosamta was reading it; try again",
        path.display()
    ))
}

/// Fails when `path` no longer holds `before`, so an edit made since the
/// plan is never overwritten.
pub(crate) fn unchanged(path: &Path, before: Option<&str>) -> Result<()> {
    let now = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(Error::Read {
                path: path.to_path_buf(),
                source: err,
            });
        }
    };
    if now.as_deref() != before.map(str::as_bytes) {
        return Err(changed(path));
    }
    Ok(())
}

/// Writes `text` (or removes the file for `None`) only while `path` still
/// holds `before`, checked at the last moment: an installer may append to
/// an rc file at any time.
pub(crate) fn replace(path: &Path, before: Option<&str>, text: Option<&str>) -> Result<()> {
    match text {
        Some(text) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
            }
            crate::edit::write_atomic_if(path, text, || unchanged(path, before))
        }
        None => {
            unchanged(path, before)?;
            write_text(path, None)
        }
    }
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

/// Local modules can hold secrets, so only their owner may read them.
fn write_module(ctx: &Context, path: &Path, text: &str) -> Result<()> {
    if path.starts_with(local_dir(ctx)) {
        crate::edit::write_private(path, text)
    } else {
        write_text(path, Some(text))
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
    let mut written = false;
    let (mut edited, mut replaced) = (Vec::new(), Vec::new());
    let write = || -> Result<()> {
        for change in plan.files.iter().chain(&plan.edits) {
            unchanged(&change.path, change.before.as_deref())?;
        }
        written = true;
        for (path, text) in &plan.modules {
            write_module(ctx, path, text)?;
        }
        for (changes, done) in [(&plan.edits, &mut edited), (&plan.files, &mut replaced)] {
            for change in changes {
                replace(
                    &change.path,
                    change.before.as_deref(),
                    change.after.as_deref(),
                )?;
                done.push(change);
            }
        }
        Ok(())
    };
    let shells: Vec<Shell> = plan
        .candidates
        .iter()
        .flat_map(|candidate| candidate.shells.iter().copied())
        .collect();
    if let Err(err) = apply::apply_after(ctx, &shells, write) {
        // All or none: lines put back in one file and kept in a module for
        // another would run twice.
        let restorable = replaced
            .iter()
            .all(|change| unchanged(&change.path, change.after.as_deref()).is_ok());
        if written && restorable {
            let mut restored = true;
            for change in &replaced {
                restored &= replace(
                    &change.path,
                    change.after.as_deref(),
                    change.before.as_deref(),
                )
                .is_ok();
            }
            // An rc file edited since rosamta wrote it keeps the adopted
            // lines out, so the modules holding them, new or joined, stay.
            if restored {
                for change in &edited {
                    let _ = replace(
                        &change.path,
                        change.after.as_deref(),
                        change.before.as_deref(),
                    );
                }
                for (path, _) in &plan.modules {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        return Err(err);
    }
    let store = Store::new(ctx.paths.state());
    let mut adoptions = store.adoptions()?;
    let mut done = Vec::new();
    let candidate_of = |record: &Adoption| {
        plan.candidates
            .iter()
            .find(|candidate| candidate.hash == record.hash && candidate.file == record.file)
            .expect("each record comes from a candidate")
    };
    for mut record in plan.records {
        // Duplicates are recorded too: restoring the module's own adoption
        // must know these lines rely on it, and they can be put back alone.
        if candidate_of(&record).kind != Kind::Comment {
            adoptions.next_id = adoptions.next_id.max(1);
            record.id = adoptions.next_id;
            adoptions.next_id += 1;
            adoptions.adoptions.push(record.clone());
        }
        done.push(record);
    }
    // Saved before committing: the files are written whatever git does.
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
    let repo_dir = ctx.paths.repo();
    // A duplicate changes no module, and an edit in progress there is not ours to commit.
    for record in done
        .iter_mut()
        .filter(|record| record.id > 0 && !record.local && !record.duplicate)
    {
        let Some(module) = &record.module else {
            continue;
        };
        let message = commit_message(candidate_of(record), &ctx.home, &host.id);
        // After a failed commit, the rest stays uncommitted too.
        match git::commit(&repo_dir, &message, &[&module.to_string_lossy()]) {
            Ok(git::Commit::Created) => {
                record.commit = Some(
                    git::run(&repo_dir, &["rev-parse", "HEAD"])?
                        .trim()
                        .to_string(),
                )
            }
            Ok(git::Commit::Clean) => {}
            Ok(git::Commit::SigningFailed(reason)) => {
                crate::sync::signing_blocked(ctx, &reason)?;
                break;
            }
            Err(err) => {
                crate::sync::commit_blocked(ctx, &err)?;
                break;
            }
        }
    }
    for record in done.iter().filter(|record| record.commit.is_some()) {
        if let Some(saved) = adoptions.adoptions.iter_mut().find(|a| a.id == record.id) {
            saved.commit.clone_from(&record.commit);
        }
    }
    store.save_adoptions(&adoptions)?;
    Ok(done)
}

/// `chore(adopt): add bun PATH from ~/.zshrc (zsh) on mbp`.
fn commit_message(candidate: &Candidate, home: &Path, host: &str) -> String {
    let name = candidate.name.as_deref().unwrap_or("adopted");
    let file = display(&candidate.file, home);
    if candidate.kind == Kind::Include {
        return format!(
            "chore(adopt): include {name} from {file} ({}) on {host}",
            candidate.shell()
        );
    }
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
        "chore(adopt): add {name} {what} from {file} ({}) on {host}",
        candidate.shell()
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
    // A broken scripts.toml leaves its items as they were; validate reports it.
    let _ = crate::scripts::review::sync_queue(ctx);
    let host = host_config(ctx)?;
    let store = Store::new(ctx.paths.state());
    if host.adopt == AdoptPolicy::Off {
        let others = store
            .queue()?
            .items
            .iter()
            .filter(|item| !item.is_rc())
            .count();
        update_notice(&store, &host, others)?;
        return Ok(Reviewed::default());
    }
    let candidates = scan(ctx)?;
    let (now_adopt, ask): (Vec<Candidate>, Vec<Candidate>) =
        candidates.into_iter().partition(|candidate| {
            host.adopt == AdoptPolicy::Auto || candidate.kind == Kind::Duplicate
        });
    let adopted = adopt(ctx, &now_adopt, Scope::Host)?;
    let old = store.queue()?;
    // Dotfile and script items are kept up to date by applying.
    let mut queue = Queue {
        next_id: old.next_id.max(1),
        items: old
            .items
            .iter()
            .filter(|item| !item.is_rc())
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
            include: candidate.kind == Kind::Include,
            dotfile: None,
            script: None,
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

/// Write the shell reminder again after the `shell_notice` setting changed.
pub fn sync_notice(ctx: &Context) -> Result<()> {
    let store = Store::new(ctx.paths.state());
    let host = crate::host::load(&ctx.paths.host_config())?;
    update_notice(&store, &host, store.queue()?.items.len())
}

pub(crate) fn take_item(ctx: &Context, id: u32) -> Result<(QueueItem, Queue)> {
    let store = Store::new(ctx.paths.state());
    let mut queue = store.queue()?;
    let index = queue
        .items
        .iter()
        .position(|item| item.id == id)
        .ok_or(Error::NoItem(id))?;
    let item = queue.items.remove(index);
    Ok((item, queue))
}

/// The rc file content an item stands for, as it is now.
fn candidate_of(ctx: &Context, id: u32, item: &QueueItem) -> Result<Candidate> {
    scan(ctx)?
        .into_iter()
        .find(|candidate| candidate.file == item.file && candidate.hash == item.hash)
        .ok_or(Error::Stale(id))
}

/// What accepting an rc file item would write, writing nothing.
pub fn preview(ctx: &Context, id: u32, scope: Scope) -> Result<Plan> {
    let (item, _) = take_item(ctx, id)?;
    if !item.is_rc() {
        return Err(Error::Usage(format!(
            "review item {id} is not rc file content"
        )));
    }
    plan(ctx, &[candidate_of(ctx, id, &item)?], scope)
}

/// `rsmt review accept <id>`.
pub fn accept(ctx: &Context, id: u32, scope: Scope) -> Result<Vec<Adoption>> {
    let (item, queue) = take_item(ctx, id)?;
    if let Some(dotfile) = &item.dotfile {
        crate::dotfiles::review::keep(ctx, id, dotfile, scope)?;
        return Ok(Vec::new());
    }
    if item.script.is_some() {
        return Err(Error::Usage(format!(
            "review item {id} is a script; run it with `rsmt review accept`"
        )));
    }
    let candidate = candidate_of(ctx, id, &item)?;
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
    if let Some(script) = &item.script {
        crate::scripts::review::ignore(ctx, id, script)?;
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
    /// The module file with its content now.
    pub module: Option<(PathBuf, String)>,
    /// What the module becomes; `None` removes it.
    pub module_after: Option<String>,
}

/// Plan `rsmt restore <id>`: put the lines back where they were, and take
/// the module away from this host.
pub fn plan_undo(ctx: &Context, id: u32) -> Result<Undo> {
    let store = Store::new(ctx.paths.state());
    let adoptions = store.adoptions()?.adoptions;
    let adoption = adoptions
        .iter()
        .find(|adoption| adoption.id == id)
        .cloned()
        .ok_or_else(|| Error::Usage(format!("no adoption {id}")))?;
    let exists = adoption.file.exists();
    let text = read(&adoption.file)?;
    let after = put_back(&text, &adoption);
    // Lines from another file on this host may still need the module.
    let in_use = adoptions.iter().any(|other| {
        other.id != id && other.module == adoption.module && other.local == adoption.local
    });
    let module = match &adoption.module {
        Some(_) if in_use || adoption.duplicate => None,
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
    let module_after = match &module {
        Some((path, text)) => {
            let registered = repo::read_manifest(&ctx.paths.repo())?.hosts;
            // A local module is this host's alone.
            let registered = if adoption.local {
                Vec::new()
            } else {
                registered
            };
            released(path, text, &adoption, &host_config(ctx)?.id, &registered)?
        }
        None => None,
    };
    Ok(Undo {
        file: FileChange {
            path: adoption.file.clone(),
            before: exists.then_some(text),
            after: Some(after),
        },
        adoption,
        module,
        module_after,
    })
}

/// The module once this host no longer uses it: with the limits it had
/// before this host joined it for every host, without this host while
/// another host loads it, and otherwise gone.
fn released(
    path: &Path,
    text: &str,
    adoption: &Adoption,
    host: &str,
    registered: &[String],
) -> Result<Option<String>> {
    let mut others = true;
    let after = edit_module(path, text, |doc| match &adoption.limits {
        Some(limits) => crate::scope::limit(doc, &limits.hosts, &limits.os),
        None => others = crate::scope::remove_host(doc, host, registered),
    })?;
    Ok(others.then_some(after))
}

/// The file with the adopted lines back in place.
fn put_back(text: &str, adoption: &Adoption) -> String {
    // A file's last line had no newline; it needs one if anything follows now.
    let removed = |rest: &str| {
        let mut removed = adoption.removed.clone();
        if !rest.is_empty() && !removed.ends_with('\n') {
            removed.push('\n');
        }
        removed
    };
    if let Some(replacement) = &adoption.replacement
        && let Some(at) = text.find(replacement.as_str())
    {
        let rest = &text[at + replacement.len()..];
        return format!("{}{}{rest}", &text[..at], removed(rest));
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
    let rest = lines[at..].concat();
    out.push_str(&removed(&rest));
    out.push_str(&rest);
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
    let (mut written, mut narrowed) = (false, false);
    let write = || -> Result<()> {
        unchanged(&plan.file.path, plan.file.before.as_deref())?;
        if let Some((path, text)) = &plan.module {
            unchanged(path, Some(text))?;
        }
        written = true;
        replace(
            &plan.file.path,
            plan.file.before.as_deref(),
            plan.file.after.as_deref(),
        )?;
        if let Some((path, text)) = &plan.module {
            replace(path, Some(text), plan.module_after.as_deref())?;
            narrowed = true;
        }
        Ok(())
    };
    // The shells that read the file the lines go back to.
    let shells: Vec<Shell> = watched_files(&ctx.hook_env(&ctx.installed()))
        .into_iter()
        .find(|watched| watched.path == plan.adoption.file)
        .map_or_else(|| vec![plan.adoption.shell], |watched| watched.shells);
    if let Err(err) = apply::apply_after(ctx, &shells, write) {
        // The lines stay in the rc file while a module edited since rosamta
        // narrowed it cannot take them back; the module takes them only
        // once the rc file gave them up.
        let module = plan.module.as_ref().filter(|_| narrowed);
        let module_free =
            module.is_none_or(|(path, _)| unchanged(path, plan.module_after.as_deref()).is_ok());
        if written
            && module_free
            && replace(
                &plan.file.path,
                plan.file.after.as_deref(),
                plan.file.before.as_deref(),
            )
            .is_ok()
            && let Some((path, text)) = module
        {
            let _ = write_module(ctx, path, text);
        }
        return Err(err);
    }
    // Saved before committing: the files are written whatever git does.
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
    store.save_ignored(&ignored)?;
    // Only a module this restore changed is committed; an edit in progress
    // in one it left alone is not ours.
    if let (Some(_), Some(module), false) =
        (&plan.module, &plan.adoption.module, plan.adoption.local)
    {
        let rel = module.to_string_lossy();
        let name = plan.adoption.name.as_deref().unwrap_or("adopted");
        let message = format!(
            "chore(restore): put back {name} in {} ({}) on {}",
            display(&plan.adoption.file, &ctx.home),
            plan.adoption.shell,
            host.id
        );
        match git::commit(&ctx.paths.repo(), &message, &[&rel]) {
            Ok(git::Commit::SigningFailed(reason)) => crate::sync::signing_blocked(ctx, &reason)?,
            Ok(_) => {}
            Err(err) => crate::sync::commit_blocked(ctx, &err)?,
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn module(text: &str) -> Module {
        toml::from_str(text).unwrap()
    }

    fn adoption(removed: &str, replacement: Option<&str>, before: &[&str]) -> Adoption {
        let lines: Vec<String> = before.iter().map(|line| line.to_string()).collect();
        Adoption {
            id: 1,
            shell: Shell::Zsh,
            file: PathBuf::from("/home/me/.zshrc"),
            line: 2,
            before: lines.clone(),
            after: Vec::new(),
            kept_before: lines,
            kept_after: Vec::new(),
            removed: removed.into(),
            replacement: replacement.map(str::to_string),
            hash: String::new(),
            name: None,
            module: None,
            local: false,
            commit: None,
            order: 0,
            at: 0,
            include: false,
            limits: None,
            duplicate: false,
        }
    }

    #[test]
    fn a_last_line_without_a_newline_goes_back_on_its_own_line() {
        let deleted = adoption("x", None, &["a\n"]);
        assert_eq!(put_back("a\nb\n", &deleted), "a\nx\nb\n");
        assert_eq!(put_back("a\n", &deleted), "a\nx");
        let block = format!("{} → m\n# x\n", segment::COMMENTED);
        let commented = adoption("x", Some(&block), &[]);
        assert_eq!(put_back(&format!("a\n{block}b\n"), &commented), "a\nx\nb\n");
        assert_eq!(put_back(&format!("a\n{block}"), &commented), "a\nx");
    }

    #[test]
    fn a_value_that_reads_its_own_old_value_is_late() {
        assert!(reads_late_variables(&module("[env]\nX = \"a:$X\"")));
        assert!(!reads_late_variables(&module(
            "[env]\nA = \"$HOME/a\"\nB = \"$A/b\"\n[path]\nprepend = [\"$B/bin\"]"
        )));
    }
}
