//! What adoption keeps in the state directory: past adoptions, the review
//! queue, ignored segments and the one-line shell notice.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// One adoption, enough to find its place again and undo it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adoption {
    pub id: u32,
    /// The shell whose file the lines came from.
    pub shell: crate::config::Shell,
    pub file: PathBuf,
    /// First removed line, 1-based, at the time of adoption.
    pub line: usize,
    /// Up to two lines above and below, as the file was before adoption.
    pub before: Vec<String>,
    pub after: Vec<String>,
    /// The same, counting only lines that stayed in the file.
    pub kept_before: Vec<String>,
    pub kept_after: Vec<String>,
    /// Exactly what left the file.
    pub removed: String,
    /// What took its place in comment mode.
    pub replacement: Option<String>,
    pub hash: String,
    pub name: Option<String>,
    /// The module file: relative to the repo, or absolute when local.
    pub module: Option<PathBuf>,
    pub local: bool,
    pub commit: Option<String>,
    pub order: u32,
    pub at: u64,
    /// This host was added to another host's module rather than a new one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include: bool,
    /// The module's host and OS limits before joining it for every host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
    /// The lines were already in a module that loads here; restoring them
    /// leaves that module alone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub duplicate: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub hosts: Vec<String>,
    pub os: Vec<crate::config::Os>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Adoptions {
    pub next_id: u32,
    pub adoptions: Vec<Adoption>,
}

/// A segment waiting for the user to accept or ignore it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: u32,
    pub file: PathBuf,
    pub start: usize,
    pub end: usize,
    pub detected_at: u64,
    pub hash: String,
    pub text: String,
    /// The module accepting it would create; `None` for comments only.
    pub module: Option<PathBuf>,
    /// Secrets stay on this host.
    pub local: bool,
    /// Accepting adds this host to another host's module.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include: bool,
    /// Set for a dotfile that differs from the repo; the fields above that
    /// describe rc file lines are then empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dotfile: Option<crate::dotfiles::review::DotfileItem>,
    /// Set for a script waiting to run; `file` is then the script.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<crate::scripts::review::ScriptItem>,
}

impl QueueItem {
    /// Content from an rc file waiting to be adopted.
    pub fn is_rc(&self) -> bool {
        self.dotfile.is_none() && self.script.is_none()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Queue {
    pub next_id: u32,
    pub items: Vec<QueueItem>,
}

/// Segments the user chose to keep, by normalized hash.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Ignored {
    pub hashes: Vec<String>,
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(state: &Path) -> Self {
        Self {
            dir: state.to_path_buf(),
        }
    }

    pub(crate) fn load<T: DeserializeOwned + Default>(&self, name: &str) -> Result<T> {
        let path = self.dir.join(name);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|err| Error::Usage(format!("{}: {err}", path.display()))),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
            Err(err) => Err(Error::Read { path, source: err }),
        }
    }

    pub(crate) fn save<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(&self.dir).map_err(Error::write(&self.dir))?;
        // Adoption records and queue items can hold lines that look secret.
        std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))
            .map_err(Error::write(&self.dir))?;
        let path = self.dir.join(name);
        let text = serde_json::to_string_pretty(value).expect("state serializes");
        crate::edit::write_atomic(&path, &format!("{text}\n"))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(Error::write(&path))
    }

    pub fn adoptions(&self) -> Result<Adoptions> {
        self.load("adoptions.json")
    }

    pub fn save_adoptions(&self, value: &Adoptions) -> Result<()> {
        self.save("adoptions.json", value)
    }

    pub fn queue(&self) -> Result<Queue> {
        self.load("queue.json")
    }

    pub fn save_queue(&self, value: &Queue) -> Result<()> {
        self.save("queue.json", value)
    }

    pub fn ignored(&self) -> Result<Ignored> {
        self.load("ignored.json")
    }

    pub fn save_ignored(&self, value: &Ignored) -> Result<()> {
        self.save("ignored.json", value)
    }

    pub fn notice_path(&self) -> PathBuf {
        self.dir.join("notice")
    }

    /// The line interactive shells print; removed when there is nothing to say.
    pub fn set_notice(&self, line: Option<&str>) -> Result<()> {
        let path = self.notice_path();
        match line {
            Some(line) => {
                std::fs::create_dir_all(&self.dir).map_err(Error::write(&self.dir))?;
                crate::edit::write_atomic(&path, &format!("{line}\n"))
            }
            None => match std::fs::remove_file(&path) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                    Err(Error::Write { path, source: err })
                }
                _ => Ok(()),
            },
        }
    }
}

pub use crate::time::now;
