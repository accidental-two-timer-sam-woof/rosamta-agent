//! Go templates in rosamta's terms: minijinja for dotfiles, plain text and
//! `watch` for scripts, and the platforms and hosts a whole-file condition
//! or a `.chezmoiignore` block leaves.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use super::gotmpl::{Expr, Node, Unsupported, parse};
use crate::config::{Arch, Os, When};

/// A set of {macos, linux} × {x86_64, aarch64}, one bit each.
pub type Platforms = u8;
pub const ALL: Platforms = 0b1111;
const MACOS: Platforms = 0b0011;
const LINUX: Platforms = 0b1100;
const X86_64: Platforms = 0b0101;
const AARCH64: Platforms = 0b1010;

/// Where an item applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub platforms: Platforms,
    /// Host IDs; `None` for every host.
    pub hosts: Option<BTreeSet<String>>,
}

impl Default for Scope {
    fn default() -> Self {
        Self {
            platforms: ALL,
            hosts: None,
        }
    }
}

impl Scope {
    pub fn and(&self, other: &Scope) -> Scope {
        let hosts = match (&self.hosts, &other.hosts) {
            (Some(a), Some(b)) => Some(a.intersection(b).cloned().collect()),
            (a, b) => a.clone().or_else(|| b.clone()),
        };
        Scope {
            platforms: self.platforms & other.platforms,
            hosts,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.platforms == 0 || self.hosts.as_ref().is_some_and(BTreeSet::is_empty)
    }

    /// `when` for this scope: `Some(None)` everywhere, `None` when the
    /// platforms are not one list of systems times one list of architectures.
    pub fn when(&self) -> Option<Option<When>> {
        let platforms = self.platforms;
        let os: Vec<(Os, Platforms)> = [(Os::Macos, MACOS), (Os::Linux, LINUX)]
            .into_iter()
            .filter(|(_, bits)| platforms & bits != 0)
            .collect();
        let arch: Vec<(Arch, Platforms)> = [(Arch::X86_64, X86_64), (Arch::Aarch64, AARCH64)]
            .into_iter()
            .filter(|(_, bits)| platforms & bits != 0)
            .collect();
        let product = os
            .iter()
            .flat_map(|(_, o)| arch.iter().map(move |(_, a)| o & a))
            .fold(0, |all, bit| all | bit);
        if product != platforms {
            return None;
        }
        let when = When {
            os: if os.len() == 2 {
                Vec::new()
            } else {
                os.iter().map(|(os, _)| *os).collect()
            },
            arch: if arch.len() == 2 {
                Vec::new()
            } else {
                arch.iter().map(|(arch, _)| *arch).collect()
            },
            hosts: self.hosts.iter().flatten().cloned().collect(),
            ..Default::default()
        };
        Some((when != When::default()).then_some(when))
    }
}

/// This machine as chezmoi and rosamta name it, and the data templates read.
pub struct Machine<'a> {
    /// `.chezmoidata` values the config does not override: the same on
    /// every host.
    pub shared: &'a Map<String, Value>,
    /// `chezmoi data` without `chezmoi`: this machine's values.
    pub data: &'a Map<String, Value>,
    /// chezmoi's `.chezmoi.hostname` here.
    pub hostname: Option<&'a str>,
    /// rosamta's host ID here.
    pub host_id: &'a str,
}

impl Machine<'_> {
    /// The host ID a chezmoi hostname stands for.
    fn host(&self, hostname: &str) -> String {
        if Some(hostname) == self.hostname {
            self.host_id.to_string()
        } else {
            crate::host::sanitize(hostname)
        }
    }
}

fn is(expr: &Expr, path: &[&str]) -> bool {
    matches!(expr, Expr::Field(fields) if fields.iter().map(String::as_str).eq(path.iter().copied()))
}

/// The platforms where `expr` holds, when it only compares the OS and arch.
pub fn platforms(expr: &Expr) -> Option<Platforms> {
    match expr {
        Expr::Bool(true) => Some(ALL),
        Expr::Bool(false) => Some(0),
        Expr::Call(name, args) if (name == "eq" || name == "ne") && args.len() >= 2 => {
            let bits = |field: &Expr, literals: &[Expr]| -> Option<Platforms> {
                let of: fn(&str) -> Platforms = if is(field, &["chezmoi", "os"]) {
                    |name| match name {
                        "darwin" => MACOS,
                        "linux" => LINUX,
                        _ => 0,
                    }
                } else if is(field, &["chezmoi", "arch"]) {
                    |name| match name {
                        "amd64" => X86_64,
                        "arm64" => AARCH64,
                        _ => 0,
                    }
                } else {
                    return None;
                };
                literals.iter().try_fold(0, |all, literal| match literal {
                    Expr::Str(text) => Some(all | of(text)),
                    _ => None,
                })
            };
            let equal = bits(&args[0], &args[1..]).or_else(|| {
                (args.len() == 2)
                    .then(|| bits(&args[1], &args[..1]))
                    .flatten()
            })?;
            match name.as_str() {
                "eq" => Some(equal),
                _ => (args.len() == 2).then_some(ALL & !equal),
            }
        }
        Expr::Call(name, args) if name == "not" && args.len() == 1 => {
            Some(ALL & !platforms(&args[0])?)
        }
        Expr::Call(name, args) if name == "and" && !args.is_empty() => args
            .iter()
            .try_fold(ALL, |all, arg| Some(all & platforms(arg)?)),
        Expr::Call(name, args) if name == "or" && !args.is_empty() => args
            .iter()
            .try_fold(0, |all, arg| Some(all | platforms(arg)?)),
        _ => None,
    }
}

/// The hostnames `eq .chezmoi.hostname "a" "b"` names.
fn hostnames(expr: &Expr, op: &str) -> Option<Vec<String>> {
    let Expr::Call(name, args) = expr else {
        return None;
    };
    let [field, rest @ ..] = args.as_slice() else {
        return None;
    };
    if name != op || rest.is_empty() || !is(field, &["chezmoi", "hostname"]) {
        return None;
    }
    rest.iter()
        .map(|literal| match literal {
            Expr::Str(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Where `cond` holds, when it is the OS, the arch, one hostname list, or
/// all of them joined by `and`.
fn scope(cond: &Expr, machine: &Machine) -> Option<Scope> {
    if let Some(platforms) = platforms(cond) {
        return Some(Scope {
            platforms,
            hosts: None,
        });
    }
    if let Some(names) = hostnames(cond, "eq") {
        return Some(Scope {
            platforms: ALL,
            hosts: Some(names.iter().map(|name| machine.host(name)).collect()),
        });
    }
    match cond {
        Expr::Call(name, args) if name == "and" && !args.is_empty() => {
            args.iter().try_fold(Scope::default(), |all, arg| {
                Some(all.and(&scope(arg, machine)?))
            })
        }
        _ => None,
    }
}

/// What converting used, for `host.toml`, `secrets.toml` and the report.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Uses {
    /// Data names that became `vars`.
    pub vars: BTreeSet<String>,
    /// chezmoi hostnames and the host IDs they became.
    pub hostnames: BTreeMap<String, String>,
    /// `onepasswordRead`, now `secret("op", …)`.
    pub op: bool,
    /// Uses a secret, so rendering it here would unlock a password manager.
    pub secrets: bool,
    /// Source paths a script hashes with `include … | sha256sum`.
    pub watch: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    pub text: String,
    /// Still a template after the conversion.
    pub template: bool,
    pub scope: Scope,
    pub uses: Uses,
}

/// The template's body and scope once whole-file conditions are taken off:
/// those become `when`, and where they do not hold chezmoi writes nothing,
/// as `when` does.
fn unwrap(mut nodes: Vec<Node>, machine: &Machine) -> (Vec<Node>, Scope) {
    let mut found = Scope::default();
    loop {
        let trailing = match nodes.as_slice() {
            [Node::If { .. }] => None,
            [Node::If { .. }, Node::Text(text)] if text.trim().is_empty() => Some(text.clone()),
            _ => return (nodes, found),
        };
        let Node::If {
            branches,
            otherwise: None,
        } = &nodes[0]
        else {
            return (nodes, found);
        };
        let [(cond, body)] = branches.as_slice() else {
            return (nodes, found);
        };
        let Some(holds) = scope(cond, machine) else {
            return (nodes, found);
        };
        found = found.and(&holds);
        let mut body = body.clone();
        body.extend(trailing.map(Node::Text));
        nodes = body;
    }
}

/// A dotfile template.
pub fn file(source: &str, machine: &Machine) -> Result<Converted, Unsupported> {
    let (nodes, scope) = unwrap(parse(source)?, machine);
    let mut writer = Writer {
        machine,
        uses: Uses::default(),
        out: String::new(),
        template: false,
    };
    writer.nodes(&nodes)?;
    Ok(Converted {
        text: writer.out,
        template: writer.template,
        scope,
        uses: writer.uses,
    })
}

/// A script template: rosamta's scripts are not templates, so only text,
/// whole-file conditions and `include "<path>" | sha256sum` convert.
pub fn script(source: &str, machine: &Machine) -> Result<Converted, Unsupported> {
    let (nodes, scope) = unwrap(parse(source)?, machine);
    let mut text = String::new();
    let mut uses = Uses::default();
    for node in nodes {
        let line = match node {
            Node::Text(part) => {
                text.push_str(&part);
                continue;
            }
            Node::Output { line, .. } => line,
            Node::If { .. } => text.matches('\n').count() + 1,
        };
        let watched = match &node {
            Node::Output {
                expr: Expr::Call(name, args),
                ..
            } if name == "sha256sum" => match args.as_slice() {
                [Expr::Call(include, path)] if include == "include" => match path.as_slice() {
                    [Expr::Str(path)] => Some(path.clone()),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        };
        match watched {
            Some(path) => uses.watch.push(path),
            None => {
                return Err(Unsupported {
                    line,
                    what: "template output in a script".into(),
                });
            }
        }
    }
    check_script(&text)?;
    Ok(Converted {
        text,
        template: false,
        scope,
        uses,
    })
}

/// chezmoi's own environment does not exist under rosamta.
pub fn check_script(text: &str) -> Result<(), Unsupported> {
    match text.lines().position(|line| line.contains("CHEZMOI_")) {
        Some(index) => Err(Unsupported {
            line: index + 1,
            what: "`CHEZMOI_` environment variables".into(),
        }),
        None => Ok(()),
    }
}

/// A `.chezmoiignore` pattern and where it applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRule {
    pub pattern: String,
    /// The platforms it ignores on.
    pub platforms: Platforms,
    /// Inside `{{ if ne .chezmoi.hostname "a" }}`: ignored except on these
    /// hosts.
    pub except: Option<BTreeSet<String>>,
}

/// `.chezmoiignore`: each pattern and where it applies.
pub fn ignore(source: &str, machine: &Machine) -> Result<Vec<IgnoreRule>, Unsupported> {
    let mut out = Vec::new();
    ignore_nodes(&parse(source)?, ALL, &None, machine, &mut out)?;
    Ok(out)
}

fn ignore_nodes(
    nodes: &[Node],
    on: Platforms,
    except: &Option<BTreeSet<String>>,
    machine: &Machine,
    out: &mut Vec<IgnoreRule>,
) -> Result<(), Unsupported> {
    let mut line = 1;
    for node in nodes {
        match node {
            Node::Text(text) => {
                for pattern in text.lines() {
                    let pattern = pattern.trim();
                    if !pattern.is_empty() && !pattern.starts_with('#') {
                        out.push(IgnoreRule {
                            pattern: pattern.to_string(),
                            platforms: on,
                            except: except.clone(),
                        });
                    }
                }
                line += text.matches('\n').count();
            }
            Node::Output { line, .. } => {
                return Err(Unsupported {
                    line: *line,
                    what: "template output in .chezmoiignore".into(),
                });
            }
            Node::If {
                branches,
                otherwise,
            } => {
                if let ([(cond, body)], None) = (branches.as_slice(), otherwise) {
                    if let Some(names) = hostnames(cond, "ne") {
                        if except.is_some() {
                            return Err(Unsupported {
                                line,
                                what: "nested hostname conditions".into(),
                            });
                        }
                        let hosts = names.iter().map(|name| machine.host(name)).collect();
                        ignore_nodes(body, on, &Some(hosts), machine, out)?;
                        continue;
                    }
                }
                let mut left = on;
                for (cond, body) in branches {
                    let Some(holds) = platforms(cond) else {
                        return Err(Unsupported {
                            line,
                            what: "a condition other than the OS, architecture or `ne .chezmoi.hostname`".into(),
                        });
                    };
                    ignore_nodes(body, left & holds, except, machine, out)?;
                    left &= !holds;
                }
                if let Some(body) = otherwise {
                    ignore_nodes(body, left, except, machine, out)?;
                }
            }
        }
    }
    Ok(())
}

struct Writer<'a> {
    machine: &'a Machine<'a>,
    uses: Uses,
    out: String,
    template: bool,
}

impl Writer<'_> {
    fn nodes(&mut self, nodes: &[Node]) -> Result<(), Unsupported> {
        for node in nodes {
            match node {
                Node::Text(text) => self.text(text, 0)?,
                Node::Output { expr, line } => match self.constant(expr, *line)? {
                    Some(text) => self.text(&text, *line)?,
                    None => {
                        let expr = self.output(expr, *line)?;
                        self.tag(&format!("{{{{ {expr} }}}}"));
                    }
                },
                Node::If {
                    branches,
                    otherwise,
                } => {
                    for (index, (cond, body)) in branches.iter().enumerate() {
                        let cond = self.condition(cond, 0)?;
                        let keyword = if index == 0 { "if" } else { "elif" };
                        self.tag(&format!("{{% {keyword} {cond} %}}"));
                        self.nodes(body)?;
                    }
                    if let Some(body) = otherwise {
                        self.tag("{% else %}");
                        self.nodes(body)?;
                    }
                    self.tag("{% endif %}");
                }
            }
        }
        Ok(())
    }

    fn tag(&mut self, tag: &str) {
        self.template = true;
        self.out.push_str(tag);
    }

    /// Text as written; a part minijinja would read as a tag is kept raw.
    fn text(&mut self, text: &str, line: usize) -> Result<(), Unsupported> {
        if !["{{", "{%", "{#"].iter().any(|open| text.contains(open)) {
            self.out.push_str(text);
            return Ok(());
        }
        if text.contains("endraw") {
            return Err(Unsupported {
                line,
                what: "text containing `endraw`".into(),
            });
        }
        self.tag(&format!("{{% raw %}}{text}{{% endraw %}}"));
        Ok(())
    }

    /// The text an output is everywhere, when it is a constant.
    fn constant(&self, expr: &Expr, line: usize) -> Result<Option<String>, Unsupported> {
        let value = match expr {
            Expr::Str(text) => return Ok(Some(text.clone())),
            Expr::Num(number) => return Ok(Some(number.clone())),
            Expr::Bool(value) => return Ok(Some(value.to_string())),
            Expr::Field(path) if path.first().map(String::as_str) != Some("chezmoi") => {
                self.shared(path, line)?
            }
            _ => None,
        };
        Ok(value.map(|value| match value {
            Value::String(text) => text,
            other => other.to_string(),
        }))
    }

    /// A `.chezmoidata` value, which is the same on every host.
    fn shared(&self, path: &[String], line: usize) -> Result<Option<Value>, Unsupported> {
        let Some(mut value) = path
            .first()
            .and_then(|first| self.machine.shared.get(first))
        else {
            return Ok(None);
        };
        for name in &path[1..] {
            value = value.get(name).ok_or_else(|| Unsupported {
                line,
                what: format!("`.{}`", path.join(".")),
            })?;
        }
        match value {
            Value::String(_) | Value::Number(_) | Value::Bool(_) => Ok(Some(value.clone())),
            _ => Err(Unsupported {
                line,
                what: format!("`.{}` is not a single value", path.join(".")),
            }),
        }
    }

    /// An expression whose value is printed: the OS and arch print as
    /// chezmoi names them.
    fn output(&mut self, expr: &Expr, line: usize) -> Result<String, Unsupported> {
        if is(expr, &["chezmoi", "os"]) {
            return Ok("(\"darwin\" if host.os == \"macos\" else host.os)".into());
        }
        if is(expr, &["chezmoi", "arch"]) {
            return Ok("(\"amd64\" if host.arch == \"x86_64\" else \"arm64\" if host.arch == \"aarch64\" else host.arch)".into());
        }
        if is(expr, &["chezmoi", "hostname"]) {
            return Err(Unsupported {
                line,
                what: "printing `.chezmoi.hostname`".into(),
            });
        }
        self.expr(expr, line)
    }

    /// An expression tested for truth: a boolean `vars` value is a string.
    fn condition(&mut self, expr: &Expr, line: usize) -> Result<String, Unsupported> {
        match expr {
            Expr::Field(path) if self.is_bool_var(path) => {
                Ok(format!("({} == \"true\")", self.expr(expr, line)?))
            }
            Expr::Call(name, args) if matches!(name.as_str(), "and" | "or") && !args.is_empty() => {
                let parts = args
                    .iter()
                    .map(|arg| self.condition(arg, line))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(format!("({})", parts.join(&format!(" {name} "))))
            }
            Expr::Call(name, args) if name == "not" && args.len() == 1 => {
                Ok(format!("(not {})", self.condition(&args[0], line)?))
            }
            _ => self.expr(expr, line),
        }
    }

    /// A data name that becomes a `vars` value and is a boolean here.
    fn is_bool_var(&self, path: &[String]) -> bool {
        path.len() == 1
            && !self.machine.shared.contains_key(&path[0])
            && matches!(self.machine.data.get(&path[0]), Some(Value::Bool(_)))
    }

    fn expr(&mut self, expr: &Expr, line: usize) -> Result<String, Unsupported> {
        let unsupported = |what: String| Err(Unsupported { line, what });
        match expr {
            Expr::Str(text) => Ok(literal(text)),
            Expr::Num(number) => Ok(number.clone()),
            Expr::Bool(value) => Ok(value.to_string()),
            Expr::Field(path) => self.field(path, line),
            Expr::Call(name, args) => match (name.as_str(), args.as_slice()) {
                ("eq", [first, rest @ ..]) if !rest.is_empty() => {
                    let parts = rest
                        .iter()
                        .map(|other| self.compare(first, other, "==", line))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(format!("({})", parts.join(" or ")))
                }
                ("ne", [a, b]) => Ok(format!("({})", self.compare(a, b, "!=", line)?)),
                ("and" | "or", args) if !args.is_empty() => self.condition(expr, line),
                ("not", [_]) => self.condition(expr, line),
                ("onepasswordRead", [Expr::Str(reference)]) => {
                    self.uses.op = true;
                    self.uses.secrets = true;
                    Ok(format!("secret(\"op\", {})", literal(reference)))
                }
                _ => unsupported(format!("`{name}`")),
            },
            Expr::Chain(inner, fields) => {
                let secret = self.secret(inner, fields, line)?;
                self.uses.secrets = true;
                Ok(secret)
            }
        }
    }

    /// `(bitwarden "item" "x").login.password` and the like.
    fn secret(&self, inner: &Expr, fields: &[String], line: usize) -> Result<String, Unsupported> {
        let unsupported = |what: String| Err(Unsupported { line, what });
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        let Expr::Call(name, args) = inner else {
            return unsupported("fields of a value".into());
        };
        let [Expr::Str(kind), Expr::Str(item)] = args.as_slice() else {
            return unsupported(format!("`{name}` with these arguments"));
        };
        if kind != "item" {
            return unsupported(format!("`{name} \"{kind}\"`"));
        }
        match (name.as_str(), fields.as_slice()) {
            ("bitwarden", ["login", "password"]) => {
                Ok(format!("bitwarden({}, \"password\")", literal(item)))
            }
            ("bitwarden", ["login", "username"]) => {
                Ok(format!("bitwarden({}, \"username\")", literal(item)))
            }
            ("bitwarden", ["notes"]) => Ok(format!("bitwarden({}, \"notes\")", literal(item))),
            ("bitwardenFields", [field, "value"]) => {
                Ok(format!("bitwarden({}, {})", literal(item), literal(field)))
            }
            _ => unsupported(format!("`{name}` field `.{}`", fields.join("."))),
        }
    }

    /// `a <op> b`, with chezmoi's OS, arch and hostnames in rosamta's terms.
    fn compare(
        &mut self,
        a: &Expr,
        b: &Expr,
        op: &str,
        line: usize,
    ) -> Result<String, Unsupported> {
        for (field, other) in [(a, b), (b, a)] {
            let Expr::Str(value) = other else { continue };
            if is(field, &["chezmoi", "os"]) {
                let value = if value == "darwin" { "macos" } else { value };
                return Ok(format!("host.os {op} {}", literal(value)));
            }
            if is(field, &["chezmoi", "arch"]) {
                let value = match value.as_str() {
                    "amd64" => "x86_64",
                    "arm64" => "aarch64",
                    other => other,
                };
                return Ok(format!("host.arch {op} {}", literal(value)));
            }
            if is(field, &["chezmoi", "hostname"]) {
                let id = self.machine.host(value);
                self.uses.hostnames.insert(value.clone(), id.clone());
                return Ok(format!("host.id {op} {}", literal(&id)));
            }
        }
        for (field, other) in [(a, b), (b, a)] {
            if let (Expr::Field(path), Expr::Bool(value)) = (field, other) {
                if self.is_bool_var(path) {
                    return Ok(format!("{} {op} \"{value}\"", self.field(path, line)?));
                }
            }
        }
        let chezmoi = ["os", "arch", "hostname"].map(|name| ["chezmoi", name]);
        if [a, b]
            .iter()
            .any(|side| chezmoi.iter().any(|path| is(side, path)))
        {
            return Err(Unsupported {
                line,
                what: "comparing the OS, architecture or hostname with a computed value".into(),
            });
        }
        Ok(format!(
            "{} {op} {}",
            self.expr(a, line)?,
            self.expr(b, line)?
        ))
    }

    fn field(&mut self, path: &[String], line: usize) -> Result<String, Unsupported> {
        let names: Vec<&str> = path.iter().map(String::as_str).collect();
        match names.as_slice() {
            ["chezmoi", "homeDir"] => return Ok("host.home".into()),
            ["chezmoi", "os"] => return Ok("host.os".into()),
            ["chezmoi", "arch"] => return Ok("host.arch".into()),
            ["chezmoi", ..] => {
                return Err(Unsupported {
                    line,
                    what: format!("`.{}`", path.join(".")),
                });
            }
            _ => {}
        }
        if let Some(value) = self.shared(path, line)? {
            return Ok(match value {
                Value::String(text) => literal(&text),
                other => other.to_string(),
            });
        }
        match (names.as_slice(), self.machine.data.get(names[0])) {
            ([name], None | Some(Value::String(_) | Value::Bool(_))) if is_name(name) => {
                self.uses.vars.insert(name.to_string());
                Ok(format!("vars.{name}"))
            }
            _ => Err(Unsupported {
                line,
                what: format!("`.{}`", path.join(".")),
            }),
        }
    }
}

fn is_name(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A minijinja string literal.
fn literal(text: &str) -> String {
    serde_json::to_string(text).expect("a string serializes")
}

#[cfg(test)]
mod tests;
