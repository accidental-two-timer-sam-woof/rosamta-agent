//! Which shells are installed, and whether they are new enough.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::Shell;
use crate::process::output_with_timeout;

pub fn floor(shell: Shell) -> Option<(u32, u32)> {
    match shell {
        Shell::Sh => None,
        Shell::Bash => Some((3, 2)),
        Shell::Zsh => Some((5, 8)),
        Shell::Fish => Some((3, 6)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Detected {
    pub shell: Shell,
    pub path: PathBuf,
    /// `None` for sh, or when the shell would not report a version.
    pub version: Option<(u32, u32)>,
}

impl Detected {
    /// Why scripts are not generated for this shell, if they are not.
    pub fn unsupported(&self) -> Option<String> {
        let floor = floor(self.shell)?;
        match self.version {
            Some(version) if version >= floor => None,
            Some((major, minor)) => Some(format!(
                "{} {major}.{minor} is older than {}.{}; upgrade it to load rosamta",
                self.shell, floor.0, floor.1
            )),
            None => Some(format!(
                "could not read the version of {}",
                self.path.display()
            )),
        }
    }
}

/// First executable named `name` on `path_var`.
pub fn which(name: &str, path_var: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    path_var
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(name))
        .find(|candidate| {
            candidate
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Cache {
    #[serde(default)]
    shells: BTreeMap<String, CacheEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
struct CacheEntry {
    path: PathBuf,
    modified: u64,
    version: String,
}

/// Installed shells and their versions. Versions are cached in `cache_file`
/// by executable path and modification time.
pub fn detect(path_var: &str, cache_file: &Path) -> Vec<Detected> {
    let mut cache: Cache = std::fs::read_to_string(cache_file)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default();
    let before = toml::to_string(&cache).unwrap_or_default();
    let mut found = Vec::new();
    for shell in Shell::ALL {
        let Some(path) = which(shell.name(), path_var) else {
            continue;
        };
        let version = if shell == Shell::Sh {
            None
        } else {
            let modified = modified_secs(&path);
            let cached = cache
                .shells
                .get(shell.name())
                .filter(|entry| entry.path == path && entry.modified == modified);
            let text = match cached {
                Some(entry) => Some(entry.version.clone()),
                None => query_version(shell, &path),
            };
            if let Some(text) = &text {
                cache.shells.insert(
                    shell.name().into(),
                    CacheEntry {
                        path: path.clone(),
                        modified,
                        version: text.clone(),
                    },
                );
            }
            text.as_deref().and_then(parse_version)
        };
        found.push(Detected {
            shell,
            path,
            version,
        });
    }
    let after = toml::to_string(&cache).unwrap_or_default();
    if after != before {
        if let Some(dir) = cache_file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(cache_file, after);
    }
    found
}

fn modified_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs())
}

/// Ask the shell for its own version variable instead of parsing `--version`.
fn query_version(shell: Shell, path: &Path) -> Option<String> {
    let mut cmd = Command::new(path);
    match shell {
        Shell::Bash => cmd.args([
            "--norc",
            "--noprofile",
            "-c",
            "echo \"${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}\"",
        ]),
        Shell::Zsh => cmd.args(["-f", "-c", "echo $ZSH_VERSION"]),
        // Point the hook at nothing so the user's config stays inert.
        Shell::Fish => cmd
            .args(["-c", "echo $version"])
            .env("ROSAMTA_CURRENT", "/nonexistent"),
        Shell::Sh => return None,
    };
    let output = output_with_timeout(&mut cmd, Duration::from_secs(5)).ok()??;
    let text = String::from_utf8_lossy(&output.stdout)
        .lines()
        .last()?
        .trim()
        .to_string();
    (output.status.success() && parse_version(&text).is_some()).then_some(text)
}

fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut numbers = text.split(['.', '-', ' ']).map(|part| part.parse::<u32>());
    Some((numbers.next()?.ok()?, numbers.next()?.ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse() {
        assert_eq!(parse_version("3.2"), Some((3, 2)));
        assert_eq!(parse_version("5.9"), Some((5, 9)));
        assert_eq!(parse_version("3.7.1"), Some((3, 7)));
        assert_eq!(parse_version("4.0b1"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn floors() {
        let fish = |version| Detected {
            shell: Shell::Fish,
            path: "/usr/bin/fish".into(),
            version,
        };
        let message = fish(Some((3, 3))).unsupported().unwrap();
        assert!(
            message.contains("3.3") && message.contains("3.6"),
            "{message}"
        );
        assert_eq!(fish(Some((3, 6))).unsupported(), None);
        assert_eq!(fish(Some((4, 0))).unsupported(), None);
        let sh = Detected {
            shell: Shell::Sh,
            path: "/bin/sh".into(),
            version: None,
        };
        assert_eq!(sh.unsupported(), None);
    }

    #[test]
    fn detects_local_shells_and_caches() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("shells.toml");
        let path = std::env::var("PATH").unwrap();
        let first = detect(&path, &cache);
        assert!(first.iter().any(|found| found.shell == Shell::Sh));
        for found in first.iter().filter(|found| found.shell != Shell::Sh) {
            assert!(found.version.is_some(), "{found:?}");
        }
        assert_eq!(detect(&path, &cache), first);
        if first.iter().any(|found| found.shell != Shell::Sh) {
            assert!(std::fs::read_to_string(&cache).unwrap().contains("version"));
        }
    }
}
