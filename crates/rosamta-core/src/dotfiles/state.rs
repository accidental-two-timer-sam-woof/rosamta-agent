//! `deployed.json`: what rosamta put at each path, to tell a repo change
//! from a change made on this host.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::adopt::store::Store;
use crate::config::Deploy;
use crate::error::Result;

const FILE: &str = "deployed.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deployed {
    /// By path relative to the home directory.
    #[serde(default)]
    pub entries: BTreeMap<PathBuf, Record>,
    /// Paths the user chose to keep as they are, until either side changes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ignored: BTreeMap<PathBuf, Sides>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub deploy: Deploy,
    /// Relative to the repo root.
    pub source: String,
    /// The repo content when it was last deployed (the ciphertext when encrypted).
    pub source_hash: String,
    /// What was written at the path; for a link, the same as `source_hash`.
    pub target_hash: String,
    /// The declared `permissions`, kept for a link turned back into a copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
}

/// Content hashes of the two sides of a dotfile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sides {
    pub local: String,
    pub repo: String,
}

pub fn load(state: &Path) -> Result<Deployed> {
    Store::new(state).load(FILE)
}

pub fn save(state: &Path, value: &Deployed) -> Result<()> {
    Store::new(state).save(FILE, value)
}
