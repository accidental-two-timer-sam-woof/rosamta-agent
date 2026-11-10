//! Shared logic for the rosamta CLI, GUI and background service.

pub mod adopt;
pub mod apply;
pub mod config;
pub mod edit;
pub mod error;
pub mod generated;
mod git;
pub mod hooks;
pub mod host;
pub mod paths;
mod process;
pub mod render;
pub mod repo;
pub mod schema;
pub mod shells;
pub mod snapshot;
pub mod time;
pub mod validate;

pub use error::{Error, Result};
