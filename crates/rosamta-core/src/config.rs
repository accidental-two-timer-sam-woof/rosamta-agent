//! Types for every file rosamta reads. Reading goes through serde; writing
//! goes through `toml_edit` (see `edit`) so user comments survive.

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
    pub phase: Option<Phase>,
    /// Order within the phase; lower loads first.
    #[serde(default = "default_order")]
    pub order: u32,
    #[serde(default)]
    pub when: When,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Env,
    Early,
    Framework,
    Tools,
    Prompt,
    Aliases,
    Late,
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
    #[serde(default = "default_pull_interval")]
    #[schemars(range(min = 1))]
    pub pull_interval: u32,
    /// Print a one-line reminder in interactive shells when items await review.
    #[serde(default = "yes")]
    pub shell_notice: bool,
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
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AdoptPolicy {
    Auto,
    #[default]
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
        assert_eq!(host.adopt, AdoptPolicy::Ask);
        assert_eq!(host.adopt_original, OriginalLines::Delete);
        assert_eq!(
            (host.publish, host.pull, host.pull_interval),
            (SyncPolicy::Auto, SyncPolicy::Auto, 15)
        );
        assert!(host.shell_notice);
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
