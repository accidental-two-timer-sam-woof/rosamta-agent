//! JSON Schema for each file kind, generated from the same types that parse them.

use crate::config::{DotfileManifest, HostConfig, Module, RepoManifest};
use crate::externals::ExternalManifest;
use crate::scripts::ScriptManifest;
use crate::secrets::SecretCommands;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Repo,
    Module,
    Host,
    Dotfiles,
    Secrets,
    Externals,
    Scripts,
}

pub fn schema(kind: Kind) -> serde_json::Value {
    let schema = match kind {
        Kind::Repo => schemars::schema_for!(RepoManifest),
        Kind::Module => schemars::schema_for!(Module),
        Kind::Host => schemars::schema_for!(HostConfig),
        Kind::Dotfiles => schemars::schema_for!(DotfileManifest),
        Kind::Secrets => schemars::schema_for!(SecretCommands),
        Kind::Externals => schemars::schema_for!(ExternalManifest),
        Kind::Scripts => schemars::schema_for!(ScriptManifest),
    };
    schema.to_value()
}
