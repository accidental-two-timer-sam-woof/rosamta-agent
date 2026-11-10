//! One daemon per user: an exclusive lock on `daemon.lock`, and
//! `daemon.json` describing the running instance for `rsmt daemon status`.

use std::fs::{File, TryLockError};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::adopt::store::Store;
use crate::error::{Error, Result};

const LOCK: &str = "daemon.lock";
const FILE: &str = "daemon.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonState {
    pub pid: u32,
    pub started_at: u64,
    pub version: String,
    /// `None` when the login shell's environment was read; otherwise why
    /// the daemon runs with the one it inherited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_error: Option<String>,
    /// Problems already notified, by kind and reason, until they clear.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notified: Vec<String>,
}

/// Held for the daemon's lifetime; the lock goes with the process.
pub struct Instance {
    _lock: File,
}

fn open_lock(state: &Path) -> Result<File> {
    std::fs::create_dir_all(state).map_err(Error::write(state))?;
    let path = state.join(LOCK);
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(Error::write(&path))
}

impl Instance {
    pub fn acquire(state: &Path) -> Result<Self> {
        let lock = open_lock(state)?;
        match lock.try_lock() {
            Ok(()) => Ok(Self { _lock: lock }),
            Err(TryLockError::WouldBlock) => {
                let pid = load(state).ok().flatten().map(|state| state.pid);
                Err(Error::Daemon(match pid {
                    Some(pid) => format!("the background service is already running (pid {pid})"),
                    None => "the background service is already running".into(),
                }))
            }
            Err(TryLockError::Error(err)) => Err(Error::Write {
                path: state.join(LOCK),
                source: err,
            }),
        }
    }
}

/// Whether some process holds the daemon lock.
pub fn running(state: &Path) -> Result<bool> {
    match open_lock(state)?.try_lock() {
        Ok(()) => Ok(false),
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(err)) => Err(Error::Write {
            path: state.join(LOCK),
            source: err,
        }),
    }
}

/// The running daemon's description; `None` when it never wrote one.
pub fn load(state: &Path) -> Result<Option<DaemonState>> {
    if !state.join(FILE).exists() {
        return Ok(None);
    }
    Store::new(state).load(FILE).map(Some)
}

pub fn save(state: &Path, value: &DaemonState) -> Result<()> {
    Store::new(state).save(FILE, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Another test forking a child holds a copy of every descriptor until
    /// the child execs, so a lock can outlive its `File` for a moment.
    fn eventually(mut check: impl FnMut() -> bool) -> bool {
        (0..100).any(|_| {
            check() || {
                std::thread::sleep(std::time::Duration::from_millis(20));
                false
            }
        })
    }

    #[test]
    fn second_instance_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(eventually(|| !running(dir.path()).unwrap()));
        let mut first = None;
        assert!(eventually(|| {
            first = Instance::acquire(dir.path()).ok();
            first.is_some()
        }));
        save(
            dir.path(),
            &DaemonState {
                pid: 42,
                ..DaemonState::default()
            },
        )
        .unwrap();
        assert!(running(dir.path()).unwrap());
        let err = Instance::acquire(dir.path()).err().unwrap();
        assert_eq!(
            err.to_string(),
            "the background service is already running (pid 42)"
        );
        drop(first);
        assert!(eventually(|| !running(dir.path()).unwrap()));
        assert!(eventually(|| Instance::acquire(dir.path()).is_ok()));
    }
}
