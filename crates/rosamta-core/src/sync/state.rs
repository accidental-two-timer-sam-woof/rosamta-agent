//! `sync.json`: the last Publish and Pull, for the GUI and the background service.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::adopt::store::Store;
use crate::error::Result;

const FILE: &str = "sync.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    /// The remote could not be reached; commits wait for the next try.
    Offline,
    /// Retrying will not help until the user acts: signing, a secret, access.
    Blocked,
    /// Pull stopped on a rebase conflict.
    Conflict,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub at: u64,
    pub outcome: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Files to look at, relative to the repo: conflicts or possible secrets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

impl Attempt {
    pub fn ok() -> Self {
        Self::new(Outcome::Ok, None)
    }

    pub fn new(outcome: Outcome, reason: Option<String>) -> Self {
        Self {
            at: crate::time::now(),
            outcome,
            reason,
            files: Vec::new(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    pub publish: Option<Attempt>,
    pub pull: Option<Attempt>,
    /// Commits the remote does not have yet.
    pub unpushed: u32,
}

pub fn load(state: &Path) -> Result<SyncState> {
    Store::new(state).load(FILE)
}

pub(crate) fn update(state: &Path, change: impl FnOnce(&mut SyncState)) -> Result<()> {
    let store = Store::new(state);
    let mut value: SyncState = store.load(FILE)?;
    change(&mut value);
    store.save(FILE, &value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), SyncState::default());
        update(dir.path(), |state| {
            state.publish = Some(Attempt::new(Outcome::Offline, Some("down".into())));
            state.unpushed = 2;
        })
        .unwrap();
        update(dir.path(), |state| state.pull = Some(Attempt::ok())).unwrap();
        let state = load(dir.path()).unwrap();
        assert_eq!(state.publish.unwrap().outcome, Outcome::Offline);
        assert_eq!(state.pull.unwrap().outcome, Outcome::Ok);
        assert_eq!(state.unpushed, 2);
        let text = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(text.contains("\"outcome\": \"offline\""), "{text}");
    }
}
