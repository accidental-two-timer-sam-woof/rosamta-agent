//! JSON Schema for each file kind, generated from the same types that parse them.

use crate::config::{HostConfig, Module, RepoManifest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Repo,
    Module,
    Host,
}

pub fn schema(kind: Kind) -> serde_json::Value {
    let schema = match kind {
        Kind::Repo => schemars::schema_for!(RepoManifest),
        Kind::Module => schemars::schema_for!(Module),
        Kind::Host => schemars::schema_for!(HostConfig),
    };
    schema.to_value()
}
