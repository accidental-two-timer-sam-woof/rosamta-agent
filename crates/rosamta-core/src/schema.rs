//! JSON Schema for each file kind, generated from the same types that parse them.

use crate::config::{DotfileManifest, HostConfig, Module, RepoManifest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Repo,
    Module,
    Host,
    Dotfiles,
}

pub fn schema(kind: Kind) -> serde_json::Value {
    let schema = match kind {
        Kind::Repo => schemars::schema_for!(RepoManifest),
        Kind::Module => schemars::schema_for!(Module),
        Kind::Host => schemars::schema_for!(HostConfig),
        Kind::Dotfiles => schemars::schema_for!(DotfileManifest),
    };
    schema.to_value()
}
