//! Turn modules into per-shell scripts that shells source at startup (ADR 0003).
//!
//! Each shell directory holds the module fragments plus small loaders:
//! `env.<ext>` (every shell mode), `rc.<ext>` (interactive phases) and, for
//! zsh, `profile.zsh` (puts PATH back in order after macOS `path_helper`).
//! Loaders list fragments explicitly, so load order never depends on globbing.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::config::{HostFacts, Module, Phase, Shell};

/// Modules that apply to this host, sorted by order and then name.
pub fn select(mut modules: Vec<(String, Module)>, host: &HostFacts) -> Vec<(String, Module)> {
    modules.retain(|(_, module)| module.enabled && module.when.matches(host));
    modules.sort_by(|(a_name, a), (b_name, b)| (a.order, a_name).cmp(&(b.order, b_name)));
    modules
}

/// Problems the parser cannot catch: names that no shell accepts.
pub fn check(module: &Module) -> Vec<String> {
    let mut problems = Vec::new();
    for name in module.env.keys() {
        if !is_identifier(name) {
            problems.push(format!("`{name}` is not a valid environment variable name"));
        }
    }
    for name in module.aliases.keys() {
        let bad = name.is_empty()
            || name
                .chars()
                .any(|c| c.is_whitespace() || "=/'\"$`\\".contains(c));
        if bad {
            problems.push(format!("`{name}` is not a valid alias name"));
        }
    }
    let entries = module.path.prepend.iter().chain(&module.path.append);
    if entries.into_iter().any(|entry| entry.trim().is_empty()) {
        problems.push("PATH entries cannot be empty".into());
    }
    problems
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A piece of a value: text kept as is, or a variable the shell expands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Var(String),
}

/// Split a value into text and `$NAME` / `${NAME}` references. A leading `~`
/// (alone or before `/`) becomes `$HOME`; every other character is literal.
fn parse_value(value: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut rest = value;
    if rest == "~" || rest.starts_with("~/") {
        parts.push(Part::Var("HOME".into()));
        rest = &rest[1..];
    }
    let mut text = String::new();
    while let Some(c) = rest.chars().next() {
        if let Some((name, len)) = (c == '$').then(|| var_at(rest)).flatten() {
            if !text.is_empty() {
                parts.push(Part::Text(std::mem::take(&mut text)));
            }
            parts.push(Part::Var(name.into()));
            rest = &rest[len..];
            continue;
        }
        text.push(c);
        rest = &rest[c.len_utf8()..];
    }
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    parts
}

/// `$NAME` or `${NAME}` at the start of `s`: the name and the bytes consumed.
fn var_at(s: &str) -> Option<(&str, usize)> {
    let ident_len = |t: &str| {
        let len = t
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(t.len());
        (len > 0 && !t.starts_with(|c: char| c.is_ascii_digit())).then_some(len)
    };
    if let Some(inner) = s.strip_prefix("${") {
        let len = ident_len(inner)?;
        inner[len..]
            .starts_with('}')
            .then(|| (&inner[..len], len + 3))
    } else {
        let len = ident_len(&s[1..])?;
        Some((&s[1..1 + len], len + 1))
    }
}

/// A POSIX double-quoted string where only the parsed variables expand.
fn posix_value(value: &str) -> String {
    let mut out = String::from('"');
    for part in parse_value(value) {
        match part {
            Part::Var(name) => write!(out, "${{{name}}}").unwrap(),
            Part::Text(text) => {
                for c in text.chars() {
                    if matches!(c, '"' | '\\' | '`' | '$') {
                        out.push('\\');
                    }
                    out.push(c);
                }
            }
        }
    }
    out.push('"');
    out
}

/// Single-quote a word for POSIX shells.
fn posix_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// A fish word: literal text in single quotes, variables in double quotes.
fn fish_value(value: &str) -> String {
    let parts = parse_value(value);
    if parts.is_empty() {
        return "''".into();
    }
    parts
        .into_iter()
        .map(|part| match part {
            Part::Var(name) => format!("\"${name}\""),
            Part::Text(text) => fish_quote(&text),
        })
        .collect()
}

fn fish_quote(text: &str) -> String {
    format!("'{}'", text.replace('\\', r"\\").replace('\'', r"\'"))
}

fn env_body(shell: Shell, module: &Module) -> String {
    let mut out = String::new();
    if shell == Shell::Fish {
        for (name, value) in &module.env {
            writeln!(out, "set -gx {name} {}", fish_value(value)).unwrap();
        }
        for dir in module.path.prepend.iter().rev() {
            let dir = fish_value(dir);
            writeln!(out, "contains -- {dir} $PATH; or set -gx PATH {dir} $PATH").unwrap();
        }
        for dir in &module.path.append {
            let dir = fish_value(dir);
            writeln!(out, "contains -- {dir} $PATH; or set -gx PATH $PATH {dir}").unwrap();
        }
        return out;
    }
    for (name, value) in &module.env {
        writeln!(out, "export {name}={}", posix_value(value)).unwrap();
    }
    // Prepending in reverse keeps the declared order; existing entries stay put.
    for dir in module.path.prepend.iter().rev() {
        let dir = posix_value(dir);
        writeln!(
            out,
            "case \":${{PATH}}:\" in *:{dir}:*) ;; *) PATH={dir}\":${{PATH}}\" ;; esac"
        )
        .unwrap();
    }
    for dir in &module.path.append {
        let dir = posix_value(dir);
        writeln!(
            out,
            "case \":${{PATH}}:\" in *:{dir}:*) ;; *) PATH=\"${{PATH}}:\"{dir} ;; esac"
        )
        .unwrap();
    }
    if !module.path.prepend.is_empty() || !module.path.append.is_empty() {
        out.push_str("export PATH\n");
    }
    out
}

fn alias_body(shell: Shell, module: &Module) -> String {
    let mut out = String::new();
    for (name, command) in &module.aliases {
        match shell {
            Shell::Fish => writeln!(out, "alias {name} {}", fish_quote(command)).unwrap(),
            _ => writeln!(out, "alias {name}={}", posix_quote(command)).unwrap(),
        }
    }
    out
}

/// One module's contribution to one phase of one shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    pub phase: Phase,
    pub order: u32,
    pub module: String,
    pub kind: &'static str,
    pub body: String,
}

impl Fragment {
    pub fn file_name(&self, shell: Shell) -> String {
        format!(
            "{}-{:04}-{}-{}.{}",
            self.phase as u8,
            self.order,
            self.module,
            self.kind,
            shell.ext()
        )
    }

    pub fn is_raw(&self) -> bool {
        self.kind == "raw"
    }
}

/// Every fragment of `modules` for `shell`, in load order.
pub fn fragments(modules: &[(String, Module)], shell: Shell) -> Vec<Fragment> {
    let mut out = Vec::new();
    for (name, module) in modules {
        if !module.shells.is_empty() && !module.shells.contains(&shell) {
            continue;
        }
        let order = module.order;
        let mut push = |phase, kind, body: String| {
            if !body.is_empty() {
                out.push(Fragment {
                    phase,
                    order,
                    module: name.clone(),
                    kind,
                    body,
                });
            }
        };
        push(Phase::Env, "env", env_body(shell, module));
        // Aliases never load in non-interactive shells, even if phase is env.
        let alias_phase = module
            .phase
            .filter(|phase| *phase != Phase::Env)
            .unwrap_or(Phase::Aliases);
        push(alias_phase, "alias", alias_body(shell, module));
        let raw_phase = module.phase.unwrap_or(Phase::Late);
        push(
            raw_phase,
            "raw",
            module.raw.get(shell).map(with_newline).unwrap_or_default(),
        );
    }
    out.sort_by(|a, b| {
        (a.phase, a.order, &a.module, a.kind).cmp(&(b.phase, b.order, &b.module, b.kind))
    });
    out
}

fn with_newline(text: &str) -> String {
    if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    }
}

/// Where loaders find things at run time.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Absolute directory of this version, e.g. `.../generated/v-1794300000-3f9a2c1e`.
    pub version_dir: PathBuf,
    pub version_id: String,
    /// Where `ROSAMTA_PROFILE=1` appends timings.
    pub profile_log: PathBuf,
}

/// Render every file for `shells`, keyed by path relative to the version dir.
pub fn render(
    modules: &[(String, Module)],
    shells: &[Shell],
    layout: &Layout,
) -> BTreeMap<PathBuf, String> {
    let mut files = BTreeMap::new();
    for &shell in shells {
        let frags = fragments(modules, shell);
        let dir = layout.version_dir.join(shell.name());
        for frag in &frags {
            files.insert(
                Path::new(shell.name()).join(frag.file_name(shell)),
                frag.body.clone(),
            );
        }
        let (env, rc): (Vec<_>, Vec<_>) = frags.iter().partition(|frag| frag.phase == Phase::Env);
        files.insert(
            Path::new(shell.name()).join(format!("env.{}", shell.ext())),
            loader(shell, Stage::Env, &env, &dir, layout),
        );
        files.insert(
            Path::new(shell.name()).join(format!("rc.{}", shell.ext())),
            loader(shell, Stage::Rc, &rc, &dir, layout),
        );
        if shell == Shell::Zsh {
            files.insert(PathBuf::from("zsh/profile.zsh"), zsh_path_order(modules));
        }
    }
    files
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Env,
    Rc,
}

const HEADER: &str = "# Generated by rosamta. Edits are overwritten on the next apply.\n";

fn loader(shell: Shell, stage: Stage, frags: &[&Fragment], dir: &Path, layout: &Layout) -> String {
    // Fragments are sourced plainly: a failing last command is normal in rc
    // code, and shells name the failing file, which carries the module name.
    let file = |frag: &Fragment| dir.join(frag.file_name(shell)).display().to_string();
    let log = layout.profile_log.display().to_string();
    let name = shell.name();
    // Per shell: login bash sources ~/.profile (sh's env) before its own.
    let id = format!("{}:{name}", layout.version_id);
    let stage_label = stage_name(stage);
    let mut out = String::from(HEADER);
    if shell == Shell::Fish {
        if stage == Stage::Env {
            writeln!(
                out,
                "if test \"$ROSAMTA_ENV\" != {id}\nset -gx ROSAMTA_ENV {id}"
            )
            .unwrap();
        }
        let now = "perl -MTime::HiRes=time -e 'printf \"%d\", time*1000'";
        writeln!(
            out,
            "set -l __rosamta_start (test \"$ROSAMTA_PROFILE\" = 1; and command -q perl; and {now})"
        )
        .unwrap();
        for frag in frags {
            writeln!(out, "source {}", fish_quote(&file(frag))).unwrap();
        }
        writeln!(
            out,
            "test -n \"$__rosamta_start\"; and echo (date +%s) fish {stage_label} total (math ({now}) - $__rosamta_start) >> {}",
            fish_quote(&log)
        )
        .unwrap();
        if stage == Stage::Env {
            out.push_str("end\n");
        }
        return out;
    }

    if stage == Stage::Env {
        // An exported marker keeps env from running twice in one process tree.
        writeln!(
            out,
            "if [ \"${{ROSAMTA_ENV-}}\" != {id} ]; then\nROSAMTA_ENV={id}\nexport ROSAMTA_ENV"
        )
        .unwrap();
    }
    let log = posix_quote(&log);
    writeln!(out, "if [ \"${{ROSAMTA_PROFILE-}}\" = 1 ]; then").unwrap();
    if shell == Shell::Zsh {
        out.push_str("  zmodload zsh/datetime 2>/dev/null\n");
    }
    // Milliseconds: whole seconds, then the first three fraction digits. The
    // decimal mark follows the locale in bash, and zsh prints more digits.
    writeln!(out, "  _rosamta_now() {{ if [ -n \"${{EPOCHREALTIME-}}\" ]; then _rosamta_f=${{EPOCHREALTIME#*[.,]}}; _rosamta_t=${{EPOCHREALTIME%[.,]*}}${{_rosamta_f%\"${{_rosamta_f#???}}\"}}; elif command -v perl >/dev/null 2>&1; then _rosamta_t=$(perl -MTime::HiRes=time -e 'printf \"%d\", time*1000'); else _rosamta_t=; fi; }}").unwrap();
    writeln!(out, "  _rosamta_now; _rosamta_total=$_rosamta_t").unwrap();
    for frag in frags {
        let path = posix_quote(&file(frag));
        let m = &frag.module;
        // Per-module timing only where the shell has a millisecond clock.
        writeln!(out, "  if [ -n \"${{EPOCHREALTIME-}}\" ]; then _rosamta_now; _rosamta_s=$_rosamta_t; . {path}; _rosamta_now; echo \"$(date +%s) {name} {stage_label} {m} $((_rosamta_t - _rosamta_s))\" >> {log}; else . {path}; fi").unwrap();
    }
    writeln!(out, "  _rosamta_now; [ -n \"$_rosamta_total\" ] && [ -n \"$_rosamta_t\" ] && echo \"$(date +%s) {name} {stage_label} total $((_rosamta_t - _rosamta_total))\" >> {log}").unwrap();
    out.push_str(
        "  unset -f _rosamta_now; unset _rosamta_t _rosamta_s _rosamta_f _rosamta_total\nelse\n",
    );
    for frag in frags {
        writeln!(out, "  . {}", posix_quote(&file(frag))).unwrap();
    }
    out.push_str("  :\nfi\n");
    if stage == Stage::Env {
        out.push_str("fi\n");
    }
    out
}

fn stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::Env => "env",
        Stage::Rc => "rc",
    }
}

/// Re-prepend every declared PATH entry, so macOS `path_helper` (run from
/// `/etc/zprofile` after `.zshenv`) cannot push them behind system paths.
fn zsh_path_order(modules: &[(String, Module)]) -> String {
    let mut out = String::from(HEADER);
    let dirs: Vec<&String> = modules
        .iter()
        .filter(|(_, module)| module.shells.is_empty() || module.shells.contains(&Shell::Zsh))
        .flat_map(|(_, module)| &module.path.prepend)
        .collect();
    for dir in dirs.into_iter().rev() {
        let dir = posix_value(dir);
        writeln!(out, "path=({dir} ${{path:#{dir}}})").unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Arch, Os};

    fn module(text: &str) -> Module {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn values_split_into_text_and_vars() {
        use Part::*;
        let t = |s: &str| Text(s.into());
        let v = |s: &str| Var(s.into());
        assert_eq!(parse_value("it's $USER"), [t("it's "), v("USER")]);
        assert_eq!(parse_value("~/bin"), [v("HOME"), t("/bin")]);
        assert_eq!(parse_value("a~b"), [t("a~b")]);
        assert_eq!(parse_value("${XDG}x$1$ $-"), [v("XDG"), t("x$1$ $-")]);
        assert_eq!(parse_value("${bad"), [t("${bad")]);
    }

    #[test]
    fn posix_and_fish_quoting() {
        assert_eq!(
            posix_value(r#"it's "$USER" \ `x` $5"#),
            r#""it's \"${USER}\" \\ \`x\` \$5""#
        );
        assert_eq!(fish_value(r"it's $USER \x"), r#"'it\'s '"$USER"' \\x'"#);
        assert_eq!(fish_value(""), "''");
    }

    #[test]
    fn selection_filters_and_orders() {
        let host = HostFacts {
            os: Os::Linux,
            arch: Arch::X86_64,
            id: "mbp".into(),
            tags: vec![],
        };
        let modules = vec![
            ("zoxide".into(), module("")),
            ("fzf".into(), module("order = 10")),
            ("off".into(), module("enabled = false")),
            ("lab".into(), module("[when]\nhosts = [\"lab-server\"]")),
        ];
        let names: Vec<_> = select(modules, &host)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(names, ["fzf", "zoxide"]);
    }

    #[test]
    fn fragments_follow_phase_then_order_then_name() {
        let modules = vec![
            (
                "b-prompt".into(),
                module("phase = \"prompt\"\n[raw]\nzsh = \"echo b\""),
            ),
            ("a-late".into(), module("[raw]\nzsh = \"echo a\"")),
            (
                "path".into(),
                module("order = 90\n[path]\nprepend = [\"~/.local/bin\"]"),
            ),
            ("aliases".into(), module("[aliases]\nll = \"ls -l\"")),
            (
                "early".into(),
                module("order = 10\n[env]\nEDITOR = \"nvim\""),
            ),
        ];
        let frags = fragments(&modules, Shell::Zsh);
        let order: Vec<_> = frags.iter().map(|f| (f.phase, f.module.as_str())).collect();
        assert_eq!(
            order,
            [
                (Phase::Env, "early"),
                (Phase::Env, "path"),
                (Phase::Prompt, "b-prompt"),
                (Phase::Aliases, "aliases"),
                (Phase::Late, "a-late")
            ]
        );
    }

    #[test]
    fn raw_only_reaches_its_shells() {
        let modules = vec![("ngrok".into(), module("[raw]\nzsh = \"eval x\""))];
        assert_eq!(fragments(&modules, Shell::Zsh).len(), 1);
        for shell in [Shell::Sh, Shell::Bash, Shell::Fish] {
            assert!(fragments(&modules, shell).is_empty(), "{shell}");
        }
    }

    #[test]
    fn shells_field_limits_structured_modules() {
        let modules = vec![("x".into(), module("shells = [\"fish\"]\n[env]\nA = \"1\""))];
        assert!(fragments(&modules, Shell::Bash).is_empty());
        assert_eq!(fragments(&modules, Shell::Fish)[0].body, "set -gx A '1'\n");
    }

    #[test]
    fn aliases_never_load_in_env_phase() {
        let modules = vec![(
            "x".into(),
            module("phase = \"env\"\n[aliases]\nll = \"ls -l\""),
        )];
        assert_eq!(fragments(&modules, Shell::Bash)[0].phase, Phase::Aliases);
    }

    #[test]
    fn checks_names() {
        let bad =
            module("[env]\n\"1X\" = \"a\"\n[aliases]\n\"a b\" = \"x\"\n[path]\nappend = [\" \"]");
        assert_eq!(check(&bad).len(), 3);
        assert!(check(&module("[env]\nGOOD_1 = \"a\"\n[aliases]\n\"ll\" = \"ls\"")).is_empty());
    }
}
