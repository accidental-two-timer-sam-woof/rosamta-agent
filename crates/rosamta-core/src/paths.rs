//! XDG locations, used on every platform (ADR 0008).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    config: PathBuf,
    data: PathBuf,
    state: PathBuf,
}

impl Paths {
    pub fn from_env() -> Result<Self> {
        Self::from_vars(|name| std::env::var_os(name))
    }

    pub fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self> {
        let home = var("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute())
            .ok_or_else(|| Error::Usage("HOME is not set to an absolute path".into()))?;
        // Per the XDG spec, unset or relative values fall back to the default.
        let dir = |name: &str, default: &str| {
            var(name)
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .unwrap_or_else(|| home.join(default))
                .join("rosamta")
        };
        Ok(Self {
            config: dir("XDG_CONFIG_HOME", ".config"),
            data: dir("XDG_DATA_HOME", ".local/share"),
            state: dir("XDG_STATE_HOME", ".local/state"),
        })
    }

    pub fn host_config(&self) -> PathBuf {
        self.config.join("host.toml")
    }

    /// The GitHub token, on hosts without a keychain.
    pub fn github_token(&self) -> PathBuf {
        self.config.join("github-token")
    }

    /// This host's copy of the key for encrypted dotfiles (ADR 0012).
    pub fn age_key(&self) -> PathBuf {
        self.config.join("age.key")
    }

    pub fn repo(&self) -> PathBuf {
        self.data.join("repo")
    }

    pub fn generated(&self) -> PathBuf {
        self.data.join("generated")
    }

    pub fn state(&self) -> &Path {
        &self.state
    }

    /// Host-only content (secrets) laid out like a repo: `local/modules/`.
    pub fn local(&self) -> PathBuf {
        self.config.join("local")
    }

    pub fn local_modules(&self) -> PathBuf {
        self.local().join(crate::repo::MODULES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(vars: &[(&str, &str)]) -> Paths {
        Paths::from_vars(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
        .unwrap()
    }

    #[test]
    fn defaults_when_unset() {
        let p = paths(&[("HOME", "/home/me")]);
        assert_eq!(
            p.host_config(),
            Path::new("/home/me/.config/rosamta/host.toml")
        );
        assert_eq!(p.repo(), Path::new("/home/me/.local/share/rosamta/repo"));
        assert_eq!(
            p.generated(),
            Path::new("/home/me/.local/share/rosamta/generated")
        );
        assert_eq!(p.state(), Path::new("/home/me/.local/state/rosamta"));
    }

    #[test]
    fn absolute_override() {
        let p = paths(&[("HOME", "/home/me"), ("XDG_CONFIG_HOME", "/srv/cfg")]);
        assert_eq!(p.host_config(), Path::new("/srv/cfg/rosamta/host.toml"));
    }

    #[test]
    fn relative_value_ignored() {
        let p = paths(&[("HOME", "/home/me"), ("XDG_STATE_HOME", "state")]);
        assert_eq!(p.state(), Path::new("/home/me/.local/state/rosamta"));
    }

    #[test]
    fn missing_home_is_an_error() {
        assert!(Paths::from_vars(|_| None).unwrap_err().is_usage());
    }
}
