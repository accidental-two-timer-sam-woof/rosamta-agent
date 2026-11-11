//! Types for every file rosamta reads. Reading goes through serde; writing
//! goes through `toml_edit` (see `edit`) so user comments survive.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Repo format this build reads and writes.
pub const CURRENT_FORMAT: u32 = 1;

/// `rosamta.toml` at the repo root.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepoManifest {
    /// Repo format version.
    #[schemars(range(min = 1))]
    pub format: u32,
    /// Host IDs of every host that has joined this repo.
    #[serde(default)]
    pub hosts: Vec<String>,
}

/// One file under `modules/`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Module {
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Phase for aliases (default `aliases`) and raw content (default
    /// `late`). Environment variables and PATH always load in `env`.
    pub phase: Option<Phase>,
    /// Order within the phase; lower loads first.
    #[serde(default = "default_order")]
    pub order: u32,
    #[serde(default)]
    pub when: When,
    /// Shells to render for; empty means every supported shell.
    #[serde(default)]
    pub shells: Vec<Shell>,
    /// Exported variables. `$NAME`, `${NAME}` and a leading `~` expand.
    #[serde(default)]
    pub env: IndexMap<String, String>,
    #[serde(default)]
    pub path: PathEntries,
    /// Defined only in interactive shells.
    #[serde(default)]
    pub aliases: IndexMap<String, String>,
    /// Shell code copied as is, only into the shells it names.
    #[serde(default)]
    pub raw: Raw,
}

impl Module {
    /// The shells it loads in: those listed (every shell when none are),
    /// and for raw code alone only the shells it is written for.
    pub fn reaches(&self) -> Vec<Shell> {
        let structured = !self.env.is_empty()
            || !self.path.prepend.is_empty()
            || !self.path.append.is_empty()
            || !self.aliases.is_empty();
        Shell::ALL
            .into_iter()
            .filter(|&shell| self.shells.is_empty() || self.shells.contains(&shell))
            .filter(|&shell| structured || self.raw.get(shell).is_some())
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathEntries {
    /// Put before the existing PATH, in this order.
    #[serde(default)]
    pub prepend: Vec<String>,
    /// Put after the existing PATH, in this order.
    #[serde(default)]
    pub append: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Raw {
    pub sh: Option<String>,
    pub bash: Option<String>,
    pub zsh: Option<String>,
    pub fish: Option<String>,
}

impl Raw {
    pub fn get(&self, shell: Shell) -> Option<&str> {
        match shell {
            Shell::Sh => self.sh.as_deref(),
            Shell::Bash => self.bash.as_deref(),
            Shell::Zsh => self.zsh.as_deref(),
            Shell::Fish => self.fish.as_deref(),
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Shell {
    Sh,
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    pub const ALL: [Shell; 4] = [Shell::Sh, Shell::Bash, Shell::Zsh, Shell::Fish];

    pub fn name(self) -> &'static str {
        match self {
            Shell::Sh => "sh",
            Shell::Bash => "bash",
            Shell::Zsh => "zsh",
            Shell::Fish => "fish",
        }
    }

    pub fn ext(self) -> &'static str {
        self.name()
    }
}

impl std::fmt::Display for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::str::FromStr for Shell {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Shell::ALL
            .into_iter()
            .find(|shell| shell.name() == name)
            .ok_or_else(|| format!("unknown shell `{name}`, expected sh, bash, zsh or fish"))
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    // Declaration order is load order.
    Env,
    Early,
    Framework,
    Tools,
    Prompt,
    Aliases,
    Late,
    /// zsh snippets only: loaded from `.zprofile` in login shells, after the
    /// system zprofile. Last, so the other phases keep their numbers.
    Login,
}

/// Where a module or dotfile applies. Values within a field are OR-ed,
/// fields are AND-ed, and an empty field places no constraint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct When {
    #[serde(default)]
    pub os: Vec<Os>,
    #[serde(default)]
    pub arch: Vec<Arch>,
    /// Host IDs, not system hostnames (ADR 0011).
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Macos,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub enum Arch {
    #[serde(rename = "x86_64")]
    X86_64,
    #[serde(rename = "aarch64")]
    Aarch64,
}

/// `dotfiles.toml`: every dotfile the repo deploys (ADR 0007).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DotfileManifest {
    #[serde(default)]
    pub dotfile: Vec<Dotfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dotfile {
    /// Where it goes, relative to the home directory.
    pub path: String,
    /// Its content in the repo, relative to the repo root; `home/<path>` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub deploy: Deploy,
    /// Octal mode set on the file or directory itself, such as "0600".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^0?[0-7]{3}$"))]
    pub permissions: Option<String>,
    /// Stored encrypted with age; deploys as copy.
    #[serde(default, skip_serializing_if = "is_default")]
    pub encrypted: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub when: When,
}

impl Dotfile {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.into(),
            source: None,
            deploy: Deploy::Link,
            permissions: None,
            encrypted: false,
            when: When::default(),
        }
    }

    pub fn source(&self) -> String {
        self.source
            .clone()
            .unwrap_or_else(|| format!("home/{}", self.path))
    }

    /// `permissions` as a mode; `None` when absent or not octal.
    pub fn mode(&self) -> Option<u32> {
        let text = self.permissions.as_deref()?;
        let digits = text
            .strip_prefix('0')
            .filter(|_| text.len() == 4)
            .unwrap_or(text);
        (digits.len() == 3 && digits.bytes().all(|b| (b'0'..=b'7').contains(&b)))
            .then(|| u32::from_str_radix(digits, 8).ok())
            .flatten()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Deploy {
    /// A symlink to the repo.
    #[default]
    Link,
    /// A copy, written again when the repo changes.
    Copy,
    /// Rendered from a template.
    Template,
    /// Copied only when nothing is there, then left alone.
    Create,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// What a condition is checked against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFacts {
    pub os: Os,
    pub arch: Arch,
    pub id: String,
    pub tags: Vec<String>,
}

impl HostFacts {
    pub fn current(host: &HostConfig) -> Self {
        Self {
            os: if cfg!(target_os = "macos") {
                Os::Macos
            } else {
                Os::Linux
            },
            arch: if cfg!(target_arch = "aarch64") {
                Arch::Aarch64
            } else {
                Arch::X86_64
            },
            id: host.id.clone(),
            tags: host.tags.clone(),
        }
    }
}

impl When {
    pub fn matches(&self, host: &HostFacts) -> bool {
        fn any<T: PartialEq>(wanted: &[T], actual: &T) -> bool {
            wanted.is_empty() || wanted.contains(actual)
        }
        any(&self.os, &host.os)
            && any(&self.arch, &host.arch)
            && any(&self.hosts, &host.id)
            && (self.tags.is_empty() || self.tags.iter().any(|tag| host.tags.contains(tag)))
    }
}

/// `host.toml`: this host's own settings, never committed to the repo.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    /// Host ID. Generated once from the hostname, then never changed by rosamta.
    pub id: String,
    /// Kept sorted and without duplicates.
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub adopt: AdoptPolicy,
    /// What happens to the adopted lines in the original startup file.
    #[serde(default)]
    pub adopt_original: OriginalLines,
    #[serde(default)]
    pub publish: SyncPolicy,
    #[serde(default)]
    pub pull: SyncPolicy,
    /// Minutes between automatic pulls.
    #[serde(
        default = "default_pull_interval",
        deserialize_with = "at_least_one_minute"
    )]
    #[schemars(range(min = 1))]
    pub pull_interval: u32,
    /// Print a one-line reminder in interactive shells when items await review.
    #[serde(default = "yes")]
    pub shell_notice: bool,
    /// What applying outside a terminal does with scripts that need to run.
    #[serde(default)]
    pub scripts: ScriptPolicy,
    /// Values for `vars.*` in templates, only on this host.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "SecretSettings::is_default")]
    pub secrets: SecretSettings,
}

/// `[secrets]` in `host.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretSettings {
    /// Bitwarden tools to try, in order; empty means rbw, bw, bws.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bitwarden: Vec<BitwardenTool>,
}

impl SecretSettings {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BitwardenTool {
    Rbw,
    Bw,
    Bws,
}

impl HostConfig {
    pub fn new(id: String) -> Self {
        Self {
            id,
            tags: Vec::new(),
            adopt: AdoptPolicy::default(),
            adopt_original: OriginalLines::default(),
            publish: SyncPolicy::default(),
            pull: SyncPolicy::default(),
            pull_interval: default_pull_interval(),
            shell_notice: true,
            scripts: ScriptPolicy::default(),
            vars: BTreeMap::new(),
            secrets: SecretSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ScriptPolicy {
    /// Run them without a terminal.
    Auto,
    /// List them in the review queue.
    #[default]
    Ask,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AdoptPolicy {
    #[default]
    Auto,
    Ask,
    Off,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OriginalLines {
    #[default]
    Delete,
    Comment,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SyncPolicy {
    #[default]
    Auto,
    Manual,
}

fn yes() -> bool {
    true
}

fn default_order() -> u32 {
    50
}

fn default_pull_interval() -> u32 {
    15
}

fn at_least_one_minute<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    match u32::deserialize(deserializer)? {
        0 => Err(serde::de::Error::custom(
            "pull_interval must be at least 1 minute",
        )),
        minutes => Ok(minutes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(os: Os, id: &str, tags: &[&str]) -> HostFacts {
        HostFacts {
            os,
            arch: Arch::Aarch64,
            id: id.into(),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
        }
    }

    #[test]
    fn when_matching_rules() {
        let cases: &[(&str, HostFacts, bool)] = &[
            ("", host(Os::Linux, "lab", &[]), true),
            (
                r#"os = ["macos", "linux"]"#,
                host(Os::Linux, "lab", &[]),
                true,
            ),
            (r#"os = ["macos"]"#, host(Os::Linux, "lab", &[]), false),
            (
                r#"os = ["macos"]
tags = ["work"]"#,
                host(Os::Macos, "mbp", &["personal"]),
                false,
            ),
            (
                r#"os = ["macos"]
tags = ["work", "lab"]"#,
                host(Os::Macos, "mbp", &["lab"]),
                true,
            ),
            (r#"hosts = ["mbp"]"#, host(Os::Macos, "mbp", &[]), true),
            (r#"hosts = ["mbp"]"#, host(Os::Macos, "mbp-2", &[]), false),
            (r#"arch = ["x86_64"]"#, host(Os::Macos, "mbp", &[]), false),
        ];
        for (when, facts, expected) in cases {
            let when: When = toml::from_str(when).unwrap();
            assert_eq!(when.matches(facts), *expected, "{when:?} on {facts:?}");
        }
    }

    #[test]
    fn module_defaults() {
        let module: Module = toml::from_str("").unwrap();
        assert!(module.enabled);
        assert_eq!(module.order, 50);
        assert_eq!(module.phase, None);
        assert_eq!(module.when, When::default());
    }

    #[test]
    fn full_module() {
        let module: Module = toml::from_str(
            r#"
enabled = false
phase = "tools"
order = 10

[when]
os = ["macos"]
arch = ["aarch64"]
hosts = ["mbp"]
tags = ["work"]
"#,
        )
        .unwrap();
        assert!(!module.enabled);
        assert_eq!(module.phase, Some(Phase::Tools));
        assert_eq!(module.when.arch, vec![Arch::Aarch64]);
    }

    #[test]
    fn host_defaults() {
        let host: HostConfig = toml::from_str(r#"id = "mbp""#).unwrap();
        assert_eq!(host, HostConfig::new("mbp".into()));
        assert_eq!(host.adopt, AdoptPolicy::Auto);
        assert_eq!(host.adopt_original, OriginalLines::Delete);
        assert_eq!(
            (host.publish, host.pull, host.pull_interval),
            (SyncPolicy::Auto, SyncPolicy::Auto, 15)
        );
        assert!(host.shell_notice);
    }

    #[test]
    fn zero_pull_interval_is_rejected() {
        let err = toml::from_str::<HostConfig>("id = \"mbp\"\npull_interval = 0").unwrap_err();
        assert!(err.message().contains("at least 1"), "{err}");
        let host: HostConfig = toml::from_str("id = \"mbp\"\npull_interval = 1").unwrap();
        assert_eq!(host.pull_interval, 1);
    }

    #[test]
    fn manifest_needs_format() {
        assert!(toml::from_str::<RepoManifest>("hosts = []").is_err());
        let manifest: RepoManifest = toml::from_str("format = 1").unwrap();
        assert!(manifest.hosts.is_empty());
    }

    #[test]
    fn unknown_os_lists_allowed_values() {
        let err = toml::from_str::<When>(r#"os = ["darwin"]"#).unwrap_err();
        assert!(
            err.message().contains("macos") && err.message().contains("linux"),
            "{err}"
        );
    }
}
