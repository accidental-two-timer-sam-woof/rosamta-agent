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
        /// Holds `.keep` and nothing else chezmoi manages.
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

fn names(dir: &Path) -> Result<Vec<(String, bool)>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(Error::read(dir))? {
        let entry = entry.map_err(Error::read(dir))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().map_err(Error::read(dir))?.is_dir();
        names.push((name, is_dir));
    }
    names.sort();
    Ok(names)
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
    for (name, is_dir) in names(&root.join(source))? {
        let source_path = join(source, &name);
        if name.starts_with('.') && !external {
            match (name.as_str(), is_dir) {
                (".chezmoiscripts", true) => {
                    managed |= collect(root, &source_path, target, false, suffix, out)?;
                }
                (".chezmoiignore", false) => out.push(Entry::Ignore {
                    source: source_path,
                    target_dir: target.to_string(),
                }),
                (".chezmoiexternals", true) => {
                    for (file, is_dir) in names(&root.join(&source_path))? {
                        if !is_dir {
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
            let dir_target = join(target, &name);
            let has_keep = root.join(&source_path).join(".keep").exists();
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
                only_keep: has_keep && !inner,
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
    let mut files: Vec<String> = names(root)?
        .into_iter()
        .filter(|(name, is_dir)| !is_dir && name.starts_with(".chezmoidata."))
        .map(|(name, _)| name)
        .collect();
    if root.join(".chezmoidata").is_dir() {
        files.extend(
            names(&root.join(".chezmoidata"))?
                .into_iter()
                .filter(|(_, is_dir)| !is_dir)
                .map(|(name, _)| format!(".chezmoidata/{name}")),
        );
    }
    let mut data = Map::new();
    for file in files {
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

    /// Not imported: keep the source in the repo for the user to convert.
    fn aside(&mut self, source: &str, target: &str, reason: Reason, detail: Option<String>) {
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
            let decrypted = match self.inputs.chezmoi {
                Some(chezmoi) => chezmoi.decrypt(&self.inputs.root.join(source)),
                None => Err(Error::Usage("chezmoi is not installed".into())),
            };
            match decrypted {
                Ok(plain) if self.inputs.can_encrypt => bytes = plain,
                Ok(_) => {
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
                Ok(converted) => {
                    scope = scope.and(&converted.scope);
                    bytes = converted.text.clone().into_bytes();
                    template = Some(converted);
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
            .is_some_and(|converted| converted.template);
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
        if let Some(converted) = &template {
            if still_template && !self.same_as_chezmoi(source, target, &entry, converted)? {
                self.aside(source, target, Reason::Differs, None);
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
            && !attrs.readonly
        {
            Deploy::Link
        } else {
            Deploy::Copy
        };
        // Rendered and decrypted files are written 0600 unless declared.
        let default = if template || attrs.encrypted {
            0o600
        } else {
            repo_mode
        };
        Dotfile {
            path: target.to_string(),
            source: None,
            deploy,
            permissions: (deploy != Deploy::Link && mode != default).then(|| format!("{mode:04o}")),
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
        let text = without_tags(&converted.text);
        if converted.template && text.trim().is_empty() {
            self.report(
                source,
                Some(target),
                Outcome::Note,
                Reason::MayBeEmpty,
                None,
            );
        }
    }

    /// Whether the converted template renders here as `chezmoi cat` does.
    /// Templates that read secrets are not rendered, nor ones that do not
    /// apply here.
    fn same_as_chezmoi(
        &self,
        source: &str,
        target: &str,
        entry: &Dotfile,
        converted: &Converted,
    ) -> Result<bool> {
        let (Some(chezmoi), false) = (self.inputs.chezmoi, converted.uses.secrets) else {
            return Ok(true);
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
            return Ok(true);
        }
        let ctx = self.inputs.ctx;
        let secrets = ctx.secrets(&host)?;
        let Ok(ours) = crate::template::render(source, &converted.text, &host, &ctx.home, &secrets)
        else {
            return Ok(false);
        };
        let theirs = chezmoi.cat(&ctx.home.join(target))?;
        Ok(ours.as_bytes() == theirs.as_slice())
    }
}

/// The text outside tags, to tell a template that may render nothing.
fn without_tags(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(['{']) {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        if let Some(raw) = tail.strip_prefix("{% raw %}") {
            let end = raw.find("{% endraw %}").unwrap_or(raw.len());
            out.push_str(&raw[..end]);
            rest = raw.get(end + "{% endraw %}".len()..).unwrap_or_default();
            continue;
        }
        let close = if tail.starts_with("{%") {
            "%}"
        } else if tail.starts_with("{{") {
            "}}"
        } else {
            out.push('{');
            rest = &tail[1..];
            continue;
        };
        match tail.find(close) {
            Some(end) => rest = &tail[end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}
