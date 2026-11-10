//! Shared logic for the rosamta CLI, GUI and background service.

pub mod config;
pub mod edit;
pub mod error;
mod git;
pub mod host;
pub mod paths;
pub mod repo;
pub mod schema;
pub mod validate;

pub use error::{Error, Result};
