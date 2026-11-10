//! Copies of rc files taken before rosamta changes them (ADR 0002).
//!
//! `$XDG_STATE_HOME/rosamta/snapshots` is a local git repo that never has a
//! remote. `files/` holds the content before each change, with the reason in
//! the commit message. `original/` holds what a file looked like before
//! rosamta first touched it, and `original-absent` lists files that did not
//! exist then; neither is ever overwritten.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::git;

const ABSENT: &str = "original-absent";

pub struct Snapshots {
    dir: PathBuf,
    home: PathBuf,
}

/// A file as rosamta first found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Original {
    pub path: PathBuf,
    /// `None` when the file did not exist.
    pub content: Option<Vec<u8>>,
}

impl Snapshots {
    pub fn new(state: &Path, home: &Path) -> Self {
        Self {
            dir: state.join("snapshots"),
            home: home.to_path_buf(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn ensure_repo(&self) -> Result<()> {
        if self.dir.join(".git").exists() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir).map_err(Error::write(&self.dir))?;
        git::run(&self.dir, &["init", "--quiet"])?;
        // Local history only: no identity prompts, no signing passphrases.
        for (key, value) in [
            ("user.name", "rosamta"),
            ("user.email", "rosamta@localhost"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
        ] {
            git::run(&self.dir, &["config", key, value])?;
        }
        Ok(())
    }

    /// Where a host path is stored inside the snapshot repo.
    fn key(&self, path: &Path) -> PathBuf {
        match path.strip_prefix(&self.home) {
            Ok(rel) => PathBuf::from("home").join(rel),
            Err(_) => PathBuf::from("root").join(path.strip_prefix("/").unwrap_or(path)),
        }
    }

    fn path_of(&self, key: &Path) -> PathBuf {
        match key.strip_prefix("home") {
            Ok(rel) => self.home.join(rel),
            Err(_) => Path::new("/").join(key.strip_prefix("root").unwrap_or(key)),
        }
    }

    /// Record the current content of `paths` before changing them.
    pub fn save(&self, paths: &[PathBuf], reason: &str) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        self.ensure_repo()?;
        let mut absent = self.absent_list()?;
        for path in paths {
            let key = self.key(path);
            let content = match std::fs::read(path) {
                Ok(content) => Some(content),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
                Err(err) => {
                    return Err(Error::Read {
                        path: path.clone(),
                        source: err,
                    });
                }
            };
            let original = self.dir.join("original").join(&key);
            if !original.exists() && !absent.contains(&key) {
                match &content {
                    Some(content) => write(&original, content)?,
                    None => absent.push(key.clone()),
                }
            }
            let latest = self.dir.join("files").join(&key);
            match &content {
                Some(content) => write(&latest, content)?,
                None => {
                    let _ = std::fs::remove_file(&latest);
                }
            }
        }
        absent.sort();
        absent.dedup();
        let list: String = absent
            .iter()
            .map(|key| format!("{}\n", key.display()))
            .collect();
        write(&self.dir.join(ABSENT), list.as_bytes())?;
        let names: Vec<String> = paths
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        let first = git::run(&self.dir, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_err();
        // The user's global hooks are for their own repos, not this one;
        // set on every save so repos made before this setting get it too.
        git::run(&self.dir, &["config", "core.hooksPath", "/dev/null"])?;
        git::commit_all(&self.dir, &format!("{reason}\n\n{}", names.join("\n")))?;
        if first {
            git::run(&self.dir, &["tag", "original"])?;
        }
        Ok(())
    }

    fn absent_list(&self) -> Result<Vec<PathBuf>> {
        match std::fs::read_to_string(self.dir.join(ABSENT)) {
            Ok(text) => Ok(text
                .lines()
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .collect()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(Error::Read {
                path: self.dir.join(ABSENT),
                source: err,
            }),
        }
    }

    /// Every file rosamta has changed, as it was before the first change.
    pub fn originals(&self) -> Result<Vec<Original>> {
        let mut out = Vec::new();
        let root = self.dir.join("original");
        if root.is_dir() {
            let mut stack = vec![root.clone()];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).map_err(Error::read(&dir))? {
                    let path = entry.map_err(Error::read(&dir))?.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else {
                        let key = path.strip_prefix(&root).unwrap_or(&path);
                        let content = std::fs::read(&path).map_err(Error::read(&path))?;
                        out.push(Original {
                            path: self.path_of(key),
                            content: Some(content),
                        });
                    }
                }
            }
        }
        for key in self.absent_list()? {
            out.push(Original {
                path: self.path_of(&key),
                content: None,
            });
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }
}

fn write(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
    }
    std::fs::write(path, content).map_err(Error::write(path))
}
