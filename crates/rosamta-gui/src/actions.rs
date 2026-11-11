//! Writes, each through core under the same lock the CLI and the background
//! service take. Run off the main thread.

use std::path::PathBuf;

use rosamta_core::adopt::{self, Lock, Scope};
use rosamta_core::apply::{self, Context};
use rosamta_core::daemon::service;
use rosamta_core::dotfiles;
use rosamta_core::edit;
use rosamta_core::generated;
use rosamta_core::scripts::review as scripts;
use rosamta_core::{Error, Result, host, repo, sync};

use crate::text::tf;

/// Done, with a warning to show when the change was made but a follow-up,
/// such as pushing it, was not.
pub type Done = Result<Option<String>>;

pub fn sync_now(ctx: &Context) -> Done {
    sync::sync(ctx).map(|_| None)
}

/// What `rsmt review` does: queue new rc file content, or adopt it under
/// the `auto` policy. Nothing before this host is set up.
pub fn refresh(ctx: &Context) -> Done {
    if !ctx.paths.host_config().is_file() || !ctx.paths.repo().join(".git").exists() {
        return Ok(None);
    }
    let lock = adopt::lock(ctx)?;
    if adopt::refresh(ctx)?.adopted.is_empty() {
        return Ok(None);
    }
    Ok(publish(ctx, &lock))
}

/// Adopt an rc file item, or keep this host's version of a dotfile.
pub fn accept(ctx: &Context, id: u32, scope: Scope) -> Done {
    let lock = adopt::lock(ctx)?;
    adopt::accept(ctx, id, scope)?;
    Ok(publish(ctx, &lock))
}

pub fn ignore(ctx: &Context, id: u32) -> Done {
    let _lock = adopt::lock(ctx)?;
    adopt::ignore(ctx, id).map(|_| None)
}

/// Run a script item. One that fails stays listed, with its log to read.
pub fn run_script(ctx: &Context, id: u32) -> Done {
    let _lock = adopt::lock(ctx)?;
    let listed =
        scripts::item_of(ctx, id)?.ok_or_else(|| Error::Usage(format!("no review item {id}")))?;
    let outcome = scripts::accept(ctx, id, &listed)?;
    Ok((!outcome.ok).then(|| tf("item.script_failed", &[("script", &listed.source)]).to_string()))
}

/// Put the repo's version of a dotfile back.
pub fn discard(ctx: &Context, id: u32) -> Done {
    let _lock = adopt::lock(ctx)?;
    dotfiles::review::discard(ctx, id).map(|_| None)
}

pub fn set_module_enabled(ctx: &Context, file: PathBuf, enabled: bool) -> Done {
    let _lock = adopt::lock(ctx)?;
    edit::set_module_enabled(&file, enabled)?;
    apply_if_applied(ctx)
}

pub fn set_host(ctx: &Context, key: &str, value: impl Into<toml_edit::Value>) -> Done {
    let _lock = adopt::lock(ctx)?;
    host::set(&ctx.paths, key, value)?;
    // The reminder shells print follows `shell_notice` at once.
    adopt::sync_notice(ctx).map(|_| None)
}

/// Tags choose modules and dotfiles, so shells are applied again.
pub fn add_tag(ctx: &Context, tag: String) -> Done {
    let _lock = adopt::lock(ctx)?;
    host::add_tag(&ctx.paths, &tag)?;
    apply_if_applied(ctx)
}

pub fn remove_tag(ctx: &Context, tag: String) -> Done {
    let _lock = adopt::lock(ctx)?;
    host::remove_tag(&ctx.paths, &tag)?;
    apply_if_applied(ctx)
}

pub fn rename(ctx: &Context, id: String) -> Done {
    let lock = adopt::lock(ctx)?;
    host::rename(&ctx.paths, id.trim())?;
    Ok(publish(ctx, &lock))
}

pub fn set_service(ctx: &Context, enabled: bool) -> Done {
    if enabled {
        service::enable(&ctx.paths).map(|_| None)
    } else {
        service::disable(&ctx.paths).map(|_| None)
    }
}

/// Create a repo, or clone one when `url` is set.
pub fn init(url: Option<String>) -> Done {
    let paths = rosamta_core::paths::Paths::from_env()?;
    let outcome = match url.as_deref().map(str::trim) {
        None => repo::init_new(&paths)?,
        Some(url) => repo::join(&paths, url)?,
    };
    if !(outcome.cloned || outcome.existing) {
        return Ok(None);
    }
    // Adding this host to an existing repo committed it to the host list.
    let ctx = Context::from_env()?;
    let lock = adopt::lock(&ctx)?;
    Ok(publish(&ctx, &lock))
}

/// Push what was just committed, as the CLI does: a failure is a warning,
/// since the change itself is made and the next sync retries.
fn publish(ctx: &Context, lock: &Lock) -> Option<String> {
    sync::publish_if_auto(ctx, lock)
        .err()
        .map(|err| err.to_string())
}

/// A host that never applied keeps its shells as they are, as everywhere else.
fn apply_if_applied(ctx: &Context) -> Done {
    if generated::current_id(&ctx.paths.generated()).is_none() {
        return Ok(None);
    }
    apply::apply(ctx).map(|_| None)
}
