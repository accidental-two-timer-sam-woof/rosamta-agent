//! JSON Schema for each file kind, generated from the same types that parse them.

use crate::config::{DotfileManifest, HostConfig, Module, RepoManifest};
use crate::secrets::SecretCommands;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Repo,
    Module,
    Host,
    Dotfiles,
    Secrets,
}

pub fn schema(kind: Kind) -> serde_json::Value {
    let schema = match kind {
        Kind::Repo => schemars::schema_for!(RepoManifest),
        Kind::Module => schemars::schema_for!(Module),
        Kind::Host => schemars::schema_for!(HostConfig),
        Kind::Dotfiles => schemars::schema_for!(DotfileManifest),
        Kind::Secrets => schemars::schema_for!(SecretCommands),
    };
    schema.to_value()
}
