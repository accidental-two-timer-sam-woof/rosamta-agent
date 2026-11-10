//! Publish, Pull and Sync between the repo and its remote.

mod state;

use crate::apply::Context;
use crate::error::Result;

pub use state::{Attempt, Outcome, SyncState, load as load_state};

/// Record that a commit could not be signed, from any command that commits.
pub(crate) fn signing_blocked(ctx: &Context, message: &str) -> Result<()> {
    let reason = format!("commit signing failed: {message}");
    state::update(ctx.paths.state(), |state| {
        state.publish = Some(Attempt::new(Outcome::Blocked, Some(reason)))
    })
}
