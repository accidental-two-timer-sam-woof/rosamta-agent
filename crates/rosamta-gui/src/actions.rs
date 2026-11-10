//! Writes, each through core under the same lock the CLI and the background
//! service take. Run off the main thread.

use rosamta_core::adopt::{self, Scope};
use rosamta_core::apply::{self, Context};
use rosamta_core::daemon::service;
use rosamta_core::dotfiles;
use rosamta_core::edit;
use rosamta_core::generated;
use rosamta_core::{Result, host, repo, sync};
use std::path::PathBuf;

pub fn sync_now(ctx: &Context) -> Result<()> {
    sync::sync(ctx).map(drop)
}

/// Adopt an rc file item, or keep this host's version of a dotfile.
pub fn accept(ctx: &Context, id: u32, scope: Scope) -> Result<()> {
    let lock = adopt::lock(ctx)?;
    adopt::accept(ctx, id, scope)?;
    sync::publish_if_auto(ctx, &lock).map(drop)
}

pub fn ignore(ctx: &Context, id: u32) -> Result<()> {
    let _lock = adopt::lock(ctx)?;
    adopt::ignore(ctx, id).map(drop)
}

/// Put the repo's version of a dotfile back.
pub fn discard(ctx: &Context, id: u32) -> Result<()> {
    let _lock = adopt::lock(ctx)?;
    dotfiles::review::discard(ctx, id).map(drop)
}

pub fn set_module_enabled(ctx: &Context, file: PathBuf, enabled: bool) -> Result<()> {
    let _lock = adopt::lock(ctx)?;
    edit::set_module_enabled(&file, enabled)?;
    apply_if_applied(ctx)
}

pub fn set_host(ctx: &Context, key: &str, value: impl Into<toml_edit::Value>) -> Result<()> {
    let _lock = adopt::lock(ctx)?;
    host::set(&ctx.paths, key, value)
}

/// Tags choose modules and dotfiles, so shells are applied again.
pub fn add_tag(ctx: &Context, tag: String) -> Result<()> {
    let _lock = adopt::lock(ctx)?;
    host::add_tag(&ctx.paths, &tag)?;
    apply_if_applied(ctx)
}

pub fn remove_tag(ctx: &Context, tag: String) -> Result<()> {
    let _lock = adopt::lock(ctx)?;
    host::remove_tag(&ctx.paths, &tag)?;
    apply_if_applied(ctx)
}

pub fn rename(ctx: &Context, id: String) -> Result<()> {
    let lock = adopt::lock(ctx)?;
    host::rename(&ctx.paths, id.trim())?;
    sync::publish_if_auto(ctx, &lock).map(drop)
}

pub fn set_service(ctx: &Context, enabled: bool) -> Result<()> {
    if enabled {
        service::enable(&ctx.paths).map(drop)
    } else {
        service::disable(&ctx.paths).map(drop)
    }
}

/// Create a repo, or clone one when `url` is set.
pub fn init(url: Option<String>) -> Result<()> {
    let paths = rosamta_core::paths::Paths::from_env()?;
    let outcome = match url.as_deref().map(str::trim) {
        None => repo::init_new(&paths)?,
        Some(url) => repo::join(&paths, url)?,
    };
    if outcome.cloned || outcome.existing {
        // Adding this host to an existing repo committed it to the host list.
        let ctx = Context::from_env()?;
        let lock = adopt::lock(&ctx)?;
        sync::publish_if_auto(&ctx, &lock)?;
    }
    Ok(())
}

/// A host that never applied keeps its shells as they are, as everywhere else.
fn apply_if_applied(ctx: &Context) -> Result<()> {
    if generated::current_id(&ctx.paths.generated()).is_none() {
        return Ok(());
    }
    apply::apply(ctx).map(drop)
}
