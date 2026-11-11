//! Shared logic for the rosamta CLI, GUI and background service.

pub mod adopt;
pub mod apply;
pub mod chezmoi;
pub mod config;
pub mod daemon;
pub mod dotfiles;
pub mod edit;
pub mod error;
pub mod externals;
mod fnv;
pub mod generated;
mod git;
pub mod github;
pub mod hooks;
pub mod host;
pub mod i18n;
pub mod paths;
mod process;
pub mod render;
pub mod repo;
pub mod schema;
pub mod scope;
pub mod scripts;
pub mod secrets;
pub mod shells;
pub mod snapshot;
pub mod sync;
pub mod template;
pub mod time;
pub mod token;
pub mod validate;

pub use error::{Error, Result};
