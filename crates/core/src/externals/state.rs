//! `externals.json`: what rosamta downloaded at each path.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::adopt::store::Store;
use crate::error::Result;

const FILE: &str = "externals.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Downloaded {
    /// By path relative to the home directory.
    #[serde(default)]
    pub entries: BTreeMap<PathBuf, Record>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub url: String,
    /// `rev` or `sha256` when it was downloaded.
    pub lock: String,
    /// An archive's extracted content, to tell whether it changed since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
}

pub fn load(state: &Path) -> Result<Downloaded> {
    Store::new(state).load(FILE)
}

pub fn save(state: &Path, value: &Downloaded) -> Result<()> {
    Store::new(state).save(FILE, value)
}
