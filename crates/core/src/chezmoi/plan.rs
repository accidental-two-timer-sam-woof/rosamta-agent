//! Reading a chezmoi source into what the import writes, and the report.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use globset::{GlobBuilder, GlobMatcher};
use serde_json::{Map, Value};

use super::Chezmoi;
use super::attrs::{self, DirAttrs, FileAttrs, FileKind, ScriptAttrs};
use super::convert::{self, ALL, Converted, IgnoreRule, Machine, Platforms, Scope};
use super::report::{Outcome, Reason, Report};
use crate::apply::Context;
use crate::config::{Deploy, Dotfile, DotfileManifest, HostConfig, HostFacts, When};
use crate::error::{Error, Result};
use crate::externals::{External, ExternalManifest};

mod externals;
mod scripts;

/// What chezmoi's config and data say about this machine.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// chezmoi's `mode = "symlink"`.
    pub symlink: bool,
    pub umask: u32,
    /// The config's `[data]`, which wins over `.chezmoidata`.
    pub config_data: Map<String, Value>,
    /// `chezmoi data` without `.chezmoi`: this machine's values.
    pub data: Map<String, Value>,
    pub hostname: Option<String>,
    /// chezmoi encrypts with gpg, whose files end in `.asc`.
    pub gpg: bool,
}

impl Config {
    pub(crate) fn read(chezmoi: &Chezmoi) -> Result<Self> {
        let config = chezmoi.json(&["dump-config", "--format", "json"])?;
        let mut data = chezmoi.json(&["data", "--format", "json"])?;
        let hostname = data
            .pointer("/chezmoi/hostname")
            .and_then(Value::as_str)
            .map(str::to_string);
        let data = match data.as_object_mut() {
            Some(data) => {
                data.remove("chezmoi");
                std::mem::take(data)
            }
            None => Map::new(),
        };
        Ok(Self {
            symlink: config.get("mode").and_then(Value::as_str) == Some("symlink"),
            umask: config
                .get("umask")
                .and_then(Value::as_u64)
                .map_or(0o022, |umask| umask as u32),
            config_data: config
                .get("data")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            data,
            hostname,
            gpg: config.get("encryption").and_then(Value::as_str) == Some("gpg"),
        })
    }
}

pub(crate) struct NewDotfile {
    pub entry: Dotfile,
    pub content: Vec<u8>,
    pub executable: bool,
}

pub(crate) struct NewScript {
    /// Repo path, `scripts/…`.
    pub source: String,
    pub content: Vec<u8>,
    pub before: bool,
    pub watch: Vec<String>,
    pub when: When,
}

#[derive(Default)]
pub(crate) struct Plan {
    pub report: Report,
    pub dotfiles: Vec<NewDotfile>,
    pub scripts: Vec<NewScript>,
    pub externals: Vec<External>,
    /// Source paths to keep unconverted in the repo.
    pub aside: Vec<String>,
    pub vars: BTreeMap<String, String>,
    pub op: bool,
    /// Encrypted files that were not imported for want of the repo key.
    pub needs_key: bool,
}

/// One item of the source, before deciding what it becomes.
enum Entry {
    File {
        source: String,
        target: String,
        attrs: FileAttrs,
    },
    Dir {
        source: String,
        target: String,
        attrs: DirAttrs,
        /// Holds nothing chezmoi manages, such as only `.keep` or `.gitkeep`:
        /// chezmoi creates it empty.
        only_keep: bool,
    },
    Ignore {
        source: String,
        target_dir: String,
    },
    Externals {
        source: String,
        target_dir: String,
    },
    Special {
        source: String,
        reason: Reason,
    },
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// The names in `dir`, sorted, with whether each is a directory and whether
/// it is a symlink (never followed).
fn names(dir: &Path) -> Result<Vec<(String, bool, bool)>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(Error::read(dir))? {
        let entry = entry.map_err(Error::read(dir))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type().map_err(Error::read(dir))?;
        names.push((name, kind.is_dir(), kind.is_symlink()));
    }
    names.sort();
    Ok(names)
}

/// chezmoi refuses a source whose names make `.`, `..` or nothing.
fn valid(name: &str, source: &str) -> Result<()> {
    if matches!(name, "" | "." | "..") {
        return Err(Error::Usage(format!("{source}: invalid name")));
    }
    Ok(())
}

/// Every item under `source` (relative to the root), in chezmoi's order.
fn collect(
    root: &Path,
    source: &str,
    target: &str,
    external: bool,
    suffix: &str,
    out: &mut Vec<Entry>,
) -> Result<bool> {
    let mut managed = false;
    for (name, is_dir, is_link) in names(&root.join(source))? {
        let source_path = join(source, &name);
        let special = name.starts_with('.') && !external;
        // Its content is outside the source: not read, not copied.
        if is_link && !special {
            out.push(Entry::Special {
                source: source_path,
                reason: Reason::Link,
            });
            managed = true;
            continue;
        }
        if special {
            match (name.as_str(), is_dir) {
                (".chezmoiscripts", true) => {
                    let scripts = join(target, ".chezmoiscripts");
                    managed |= collect(root, &source_path, &scripts, false, suffix, out)?;
                }
                (".chezmoiignore", false) => out.push(Entry::Ignore {
                    source: source_path,
                    target_dir: target.to_string(),
                }),
                (".chezmoiexternals", true) => {
                    for (file, is_dir, is_link) in names(&root.join(&source_path))? {
                        if !is_dir && !is_link {
                            out.push(Entry::Externals {
                                source: join(&source_path, &file),
                                target_dir: target.to_string(),
                            });
                        }
                    }
                }
                (name, false) if name.starts_with(".chezmoiexternal.") => {
                    out.push(Entry::Externals {
                        source: source_path,
                        target_dir: target.to_string(),
                    })
                }
                (".chezmoiremove", _) | (".chezmoitemplates", _) => out.push(Entry::Special {
                    source: source_path,
                    reason: Reason::Special,
                }),
                _ => {}
            }
            continue;
        }
        if is_dir {
            let (name, attrs) = if external {
                (name.clone(), DirAttrs::default())
            } else {
                attrs::parse_dir(&name)
            };
            valid(&name, &source_path)?;
            let dir_target = join(target, &name);
            let inner = collect(
                root,
                &source_path,
                &dir_target,
                external || attrs.external,
                suffix,
                out,
            )?;
            managed = true;
            out.push(Entry::Dir {
                source: source_path,
                target: dir_target,
                attrs,
                only_keep: !inner,
            });
            continue;
        }
        let (name, attrs) = if external {
            let plain = FileAttrs {
                kind: FileKind::Regular,
                encrypted: false,
                private: false,
                readonly: false,
                empty: false,
                executable: false,
                template: false,
            };
            (name.clone(), plain)
        } else {
            attrs::parse_file(&name, suffix)
        };
        valid(&name, &source_path)?;
        managed = true;
        out.push(Entry::File {
            source: source_path,
            target: join(target, &name),
            attrs,
        });
    }
    Ok(managed)
}

/// `.chezmoidata.toml`/`.json` and `.chezmoidata/`; other formats are
/// reported.
fn shared_data(root: &Path, report: &mut Report) -> Result<Map<String, Value>> {
    let mut files: Vec<(String, bool)> = names(root)?
        .into_iter()
        .filter(|(name, is_dir, _)| !is_dir && name.starts_with(".chezmoidata."))
        .map(|(name, _, is_link)| (name, is_link))
        .collect();
    if root.join(".chezmoidata").is_dir() {
        files.extend(
            names(&root.join(".chezmoidata"))?
                .into_iter()
                .filter(|(_, is_dir, _)| !is_dir)
                .map(|(name, _, is_link)| (format!(".chezmoidata/{name}"), is_link)),
        );
    }
    let mut data = Map::new();
    for (file, is_link) in files {
        // Its values could end up in the repo.
        if is_link {
            report.push(&file, None, Outcome::Manual, Reason::Link, None);
            continue;
        }
        match read_data(&root.join(&file))? {
            Some(Value::Object(map)) => data.extend(map),
            _ => report.push(&file, None, Outcome::Manual, Reason::Format, None),
        }
    }
    Ok(data)
}

/// A TOML or JSON file as JSON; `None` for other formats.
fn read_data(path: &Path) -> Result<Option<Value>> {
    let text = std::fs::read_to_string(path).map_err(Error::read(path))?;
    let name = path.to_string_lossy();
    if name.ends_with(".toml") {
        let table: toml::Table = toml::from_str(&text)
            .map_err(|err| Error::Usage(format!("{}: {err}", path.display())))?;
        return Ok(serde_json::to_value(table).ok());
    }
    if name.ends_with(".json") {
        return serde_json::from_str(&text)
            .map(Some)
            .map_err(|err| Error::Usage(format!("{}: {err}", path.display())));
    }
    Ok(None)
}

struct Rule {
    matcher: GlobMatcher,
    negated: bool,
    rule: IgnoreRule,
}

pub(crate) struct Inputs<'a> {
    pub ctx: &'a Context,
    pub host: &'a HostConfig,
    pub root: &'a Path,
    pub config: &'a Config,
    pub chezmoi: Option<&'a Chezmoi>,
    /// Whether encrypted files can be re-encrypted here.
    pub can_encrypt: bool,
}

struct Planner<'a> {
    inputs: &'a Inputs<'a>,
    machine: Machine<'a>,
    rules: Vec<Rule>,
    declared: BTreeSet<String>,
    plan: Plan,
    /// Source path → repo path of the dotfile it became, for `watch`.
    imported: BTreeMap<String, String>,
}

pub(crate) fn plan(inputs: &Inputs) -> Result<Plan> {
    let root = inputs.root;
    let mut report = Report::default();
    let mut shared = shared_data(root, &mut report)?;
    shared.retain(|name, _| !inputs.config.config_data.contains_key(name));
    let mut entries = Vec::new();
    let suffix = if inputs.config.gpg { ".asc" } else { ".age" };
    collect(root, "", "", false, suffix, &mut entries)?;
    let repo = inputs.ctx.paths.repo();
    let declared = crate::dotfiles::load(&repo)?
        .dotfile
        .into_iter()
        .map(|entry| entry.path)
        .collect();
    let machine = Machine {
        shared: &shared,
        data: &inputs.config.data,
        hostname: inputs.config.hostname.as_deref(),
        host_id: &inputs.host.id,
    };
    let mut planner = Planner {
        inputs,
        machine,
        rules: Vec::new(),
        declared,
        plan: Plan {
            report,
            ..Default::default()
        },
        imported: BTreeMap::new(),
    };
    for entry in &entries {
        if let Entry::Ignore { source, target_dir } = entry {
            planner.ignore_file(source, target_dir)?;
        }
    }
    let mut scripts = Vec::new();
    for entry in &entries {
        match entry {
            Entry::File {
                source,
                target,
                attrs,
            } => match attrs.kind {
                FileKind::Script(script) => scripts.push((source, target, *attrs, script)),
                _ => planner.file(source, target, attrs)?,
            },
            Entry::Dir {
                source,
                target,
                attrs,
                only_keep,
            } => planner.dir(source, target, attrs, *only_keep),
            Entry::Externals { source, target_dir } => planner.externals(source, target_dir)?,
            Entry::Special { source, reason } => {
                planner
                    .plan
                    .report
                    .push(source, None, Outcome::Manual, *reason, None)
            }
            Entry::Ignore { .. } => {}
        }
    }
    // chezmoi runs scripts in the order of their names.
    scripts.sort_by(|a, b| a.1.cmp(b.1));
    for (source, target, attrs, script) in scripts {
        planner.script(source, target, &attrs, script)?;
    }
    planner.git_ignored()?;
    planner.nesting()?;
    Ok(planner.plan)
}

impl Planner<'_> {
    fn ignore_file(&mut self, source: &str, target_dir: &str) -> Result<()> {
        let path = self.inputs.root.join(source);
        let text = std::fs::read_to_string(&path).map_err(Error::read(&path))?;
        let rules = convert::ignore(&text, &self.machine).map_err(|err| {
            Error::Usage(format!(
                "{source} line {}: {} is not supported; simplify it and import again",
                err.line, err.what
            ))
        })?;
        for rule in rules {
            let (negated, pattern) = match rule.pattern.strip_prefix('!') {
                Some(pattern) => (true, pattern),
                None => (false, rule.pattern.as_str()),
            };
            let pattern = join(target_dir, pattern.trim_start_matches('/'));
            // chezmoi's `dir/**` also matches `dir` itself.
            let base = pattern.strip_suffix("/**").map(str::to_string);
            for pattern in std::iter::once(pattern.clone()).chain(base) {
                let matcher = GlobBuilder::new(&pattern)
                    .literal_separator(true)
                    .build()
                    .map_err(|err| Error::Usage(format!("{source}: {err}")))?
                    .compile_matcher();
                self.rules.push(Rule {
                    matcher,
                    negated,
                    rule: rule.clone(),
                });
            }
        }
        Ok(())
    }

    /// Where `target` applies once `.chezmoiignore` is taken into account.
    /// As in chezmoi, a `!` rule only keeps the path it matches, and an
    /// ignored directory takes everything in it.
    fn scope(&self, target: &str) -> Scope {
        let mut scope = Scope::default();
        let paths = std::iter::successors(Some(target), |path| {
            path.rsplit_once('/').map(|(parent, _)| parent)
        });
        for path in paths {
            let mut ignored: Platforms = 0;
            let mut kept: Platforms = 0;
            let mut hosts = Vec::new();
            for rule in self.rules.iter().filter(|rule| rule.matcher.is_match(path)) {
                match (&rule.rule.except, rule.negated) {
                    (_, true) => kept |= rule.rule.platforms,
                    (Some(except), false) => hosts.push(except),
                    (None, false) => ignored |= rule.rule.platforms,
                }
            }
            scope.platforms &= !(ignored & !kept);
            if kept != ALL {
                for except in hosts {
                    scope = scope.and(&Scope {
                        platforms: ALL,
                        hosts: Some(except.clone()),
                    });
                }
            }
        }
        scope
    }

    fn report(
        &mut self,
        source: &str,
        target: Option<&str>,
        outcome: Outcome,
        reason: Reason,
        detail: Option<String>,
    ) {
        self.plan
            .report
            .push(source, target, outcome, reason, detail);
    }

    /// Not imported: keep the source in the repo for the user to convert,
    /// unless it looks like a secret.
    fn aside(&mut self, source: &str, target: &str, reason: Reason, detail: Option<String>) {
        let secret = self
            .read(source)
            .ok()
            .and_then(|bytes| crate::adopt::secret_rule(&String::from_utf8_lossy(&bytes)));
        if let Some(rule) = secret {
            let rule = Some(rule.to_string());
            self.report(source, Some(target), Outcome::Manual, Reason::Secret, rule);
            return;
        }
        self.report(source, Some(target), Outcome::Manual, reason, detail);
        self.plan.aside.push(source.to_string());
    }

    fn dir(&mut self, source: &str, target: &str, attrs: &DirAttrs, only_keep: bool) {
        if attrs.remove {
            self.report(source, Some(target), Outcome::Manual, Reason::Remove, None);
        }
        if attrs.exact {
            self.report(
                source,
                Some(target),
                Outcome::Manual,
                Reason::ExactDir,
                None,
            );
        }
        if attrs.private || attrs.readonly {
            self.report(
                source,
                Some(target),
                Outcome::Manual,
                Reason::DirPermissions,
                None,
            );
        }
        if only_keep && !self.scope(target).is_empty() {
            self.report(
                source,
                Some(target),
                Outcome::Manual,
                Reason::EmptyDir,
                None,
            );
        }
    }

    fn read(&self, source: &str) -> Result<Vec<u8>> {
        let path = self.inputs.root.join(source);
        std::fs::read(&path).map_err(Error::read(&path))
    }

    fn file(&mut self, source: &str, target: &str, attrs: &FileAttrs) -> Result<()> {
        let scope = self.scope(target);
        if scope.platforms == 0 {
            self.report(
                source,
                Some(target),
                Outcome::Skipped,
                Reason::Ignored,
                None,
            );
            return Ok(());
        }
        let reason = match attrs.kind {
            FileKind::Modify => Some(Reason::Modify),
            FileKind::Remove => Some(Reason::Remove),
            FileKind::Symlink => Some(Reason::Symlink),
            FileKind::Create if attrs.template => Some(Reason::CreateTemplate),
            _ => None,
        };
        if let Some(reason) = reason {
            self.aside(source, target, reason, None);
            return Ok(());
        }
        if crate::dotfiles::startup_file(Path::new(target)).is_some() {
            self.report(
                source,
                Some(target),
                Outcome::Skipped,
                Reason::StartupFile,
                None,
            );
            return Ok(());
        }
        let repo = self.inputs.ctx.paths.repo();
        if self.declared.contains(target) || repo.join("home").join(target).exists() {
            self.report(source, Some(target), Outcome::Skipped, Reason::Exists, None);
            return Ok(());
        }
        let mut bytes = self.read(source)?;
        if attrs.encrypted {
            // Without the repo key it could not be sealed again: chezmoi is
            // not asked for a passphrase for nothing.
            if !self.inputs.can_encrypt {
                self.plan.needs_key = true;
                self.report(
                    source,
                    Some(target),
                    Outcome::Manual,
                    Reason::Encrypted,
                    None,
                );
                return Ok(());
            }
            let decrypted = match self.inputs.chezmoi {
                Some(chezmoi) => chezmoi.decrypt(&self.inputs.root.join(source)),
                None => Err(Error::Usage("chezmoi is not installed".into())),
            };
            match decrypted {
                Ok(plain) => bytes = plain,
                Err(err) => {
                    self.report(
                        source,
                        Some(target),
                        Outcome::Manual,
                        Reason::Encrypted,
                        Some(err.to_string()),
                    );
                    return Ok(());
                }
            }
        }
        let mut scope = scope;
        let mut template = None;
        if attrs.template {
            let Ok(text) = String::from_utf8(bytes.clone()) else {
                self.aside(
                    source,
                    target,
                    Reason::Template,
                    Some("not UTF-8 text".into()),
                );
                return Ok(());
            };
            match convert::file(&text, &self.machine) {
                // chezmoi writes an `empty_` file where its condition does not hold.
                Ok(converted) if attrs.empty && converted.scope != Scope::default() => {
                    self.aside(source, target, Reason::Scope, None);
                    return Ok(());
                }
                Ok(converted) => {
                    scope = scope.and(&converted.scope);
                    bytes = converted.text.clone().into_bytes();
                    template = Some((text, converted));
                }
                Err(err) => {
                    let detail = format!("line {}: {}", err.line, err.what);
                    self.aside(source, target, Reason::Template, Some(detail));
                    return Ok(());
                }
            }
        }
        let still_template = template
            .as_ref()
            .is_some_and(|(_, converted)| converted.template);
        if scope.is_empty() || (!attrs.empty && !still_template && bytes.trim_ascii().is_empty()) {
            self.report(source, Some(target), Outcome::Skipped, Reason::Empty, None);
            return Ok(());
        }
        if !attrs.encrypted
            && let Some(rule) = crate::adopt::secret_rule(&String::from_utf8_lossy(&bytes))
        {
            self.report(
                source,
                Some(target),
                Outcome::Manual,
                Reason::Secret,
                Some(rule.into()),
            );
            return Ok(());
        }
        let Some(when) = scope.when() else {
            self.aside(source, target, Reason::Scope, None);
            return Ok(());
        };
        let when = when.unwrap_or_default();
        let entry = self.entry(target, attrs, still_template, when);
        if let Some((original, converted)) = &template {
            if let Some(reason) = self.compare(original, &entry, attrs.empty, converted)? {
                self.aside(source, target, reason, None);
                return Ok(());
            }
            self.used(source, target, converted);
        }
        self.imported
            .insert(source.to_string(), format!("home/{target}"));
        self.report(
            source,
            Some(target),
            Outcome::Imported,
            Reason::Dotfile,
            None,
        );
        self.plan.dotfiles.push(NewDotfile {
            entry,
            content: bytes,
            executable: attrs.executable,
        });
        Ok(())
    }

    /// The declaration, deployed and with the permissions chezmoi gives it.
    fn entry(&self, target: &str, attrs: &FileAttrs, template: bool, when: When) -> Dotfile {
        let mut mode = if attrs.executable { 0o777 } else { 0o666 } & !self.inputs.config.umask;
        if attrs.private {
            mode &= !0o077;
        }
        if attrs.readonly {
            mode &= !0o222;
        }
        let repo_mode = if attrs.executable { 0o755 } else { 0o644 };
        let deploy = if template {
            Deploy::Template
        } else if attrs.kind == FileKind::Create {
            Deploy::Create
        } else if self.inputs.config.symlink
            && !attrs.encrypted
            && !attrs.private
            && !attrs.executable
        {
            Deploy::Link
        } else {
            Deploy::Copy
        };
        // A rendered or decrypted file keeps the mode of one already there
        // unless its own is declared.
        let declared = template || attrs.encrypted || mode != repo_mode;
        Dotfile {
            path: target.to_string(),
            source: None,
            deploy,
            permissions: (deploy != Deploy::Link && declared).then(|| format!("{mode:04o}")),
            encrypted: attrs.encrypted,
            when,
        }
    }

    /// What converting used: vars, hostnames, secrets.
    fn used(&mut self, source: &str, target: &str, converted: &Converted) {
        for name in &converted.uses.vars {
            let value = match self.inputs.config.data.get(name) {
                Some(Value::String(text)) => Some(text.clone()),
                Some(Value::Bool(value)) => Some(value.to_string()),
                _ => None,
            };
            if let Some(value) = value {
                self.plan.vars.insert(name.clone(), value);
            }
        }
        self.plan.op |= converted.uses.op;
        if !converted.uses.hostnames.is_empty() {
            let detail = converted
                .uses
                .hostnames
                .iter()
                .map(|(name, id)| format!("{name} → {id}"))
                .collect::<Vec<_>>()
                .join(", ");
            self.report(
                source,
                Some(target),
                Outcome::Note,
                Reason::Hostname,
                Some(detail),
            );
        }
        if converted.may_be_empty {
            self.report(
                source,
                Some(target),
                Outcome::Note,
                Reason::MayBeEmpty,
                None,
            );
        }
    }

    /// The user's git ignores some names, such as `.env`: those would be
    /// declared but never committed.
    fn git_ignored(&mut self) -> Result<()> {
        let home = |path: &str| format!("home/{path}");
        let paths: Vec<String> = self
            .plan
            .dotfiles
            .iter()
            .map(|new| home(&new.entry.path))
            .chain(self.plan.scripts.iter().map(|new| new.source.clone()))
            .collect();
        let ignored = crate::git::ignored(&self.inputs.ctx.paths.repo(), &paths)?;
        if ignored.is_empty() {
            return Ok(());
        }
        self.plan
            .dotfiles
            .retain(|new| !ignored.contains(&home(&new.entry.path)));
        self.plan
            .scripts
            .retain(|new| !ignored.contains(&new.source));
        for item in &mut self.plan.report.items {
            let path = match (item.outcome, item.reason, &item.target) {
                (Outcome::Imported, Reason::Dotfile, Some(target)) => home(target),
                (Outcome::Imported, Reason::Script, Some(target)) => target.clone(),
                _ => continue,
            };
            if ignored.contains(&path) {
                item.outcome = Outcome::Manual;
                item.reason = Reason::GitIgnored;
            }
        }
        Ok(())
    }

    /// Where the converted template does not give what chezmoi gives here:
    /// `Differs`, or `MayBeEmpty` where chezmoi writes no file. Templates
    /// that read secrets are not rendered, nor ones that do not apply here.
    fn compare(
        &self,
        original: &str,
        entry: &Dotfile,
        empty: bool,
        converted: &Converted,
    ) -> Result<Option<Reason>> {
        let (Some(chezmoi), false) = (self.inputs.chezmoi, converted.uses.secrets) else {
            return Ok(None);
        };
        let mut host = self.inputs.host.clone();
        for name in &converted.uses.vars {
            if let Some(value) = self.inputs.config.data.get(name) {
                let value = value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_string);
                host.vars.entry(name.clone()).or_insert(value);
            }
        }
        if !entry.when.matches(&HostFacts::current(&host)) {
            return Ok(None);
        }
        // Only this text: chezmoi reads no other template or external.
        let Ok(theirs) = chezmoi.render(original.as_bytes()) else {
            return Ok(Some(Reason::Differs));
        };
        if !empty && theirs.trim_ascii().is_empty() {
            return Ok(Some(Reason::MayBeEmpty));
        }
        let ours = if converted.template {
            let ctx = self.inputs.ctx;
            let secrets = ctx.secrets(&host)?;
            match crate::template::render(&entry.path, &converted.text, &host, &ctx.home, &secrets)
            {
                Ok(text) => text,
                Err(_) => return Ok(Some(Reason::Differs)),
            }
        } else {
            converted.text.clone()
        };
        Ok((ours.as_bytes() != theirs.as_slice()).then_some(Reason::Differs))
    }
}
