//! Writes, each through core under the same lock the CLI and the background
//! service take. Run off the main thread.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use rosamta_core::adopt::{self, Lock, Scope};
use rosamta_core::apply::{self, Context};
use rosamta_core::chezmoi::{self, Report};
use rosamta_core::daemon::service;
use rosamta_core::dotfiles;
use rosamta_core::edit;
use rosamta_core::generated;
use rosamta_core::scripts::review::{self as scripts, ScriptItem};
use rosamta_core::{Result, host, repo, sync};

use crate::text::tf;

/// Done, with a warning to show when the change was made but a follow-up,
/// such as pushing it, was not.
pub type Done = Result<Option<String>>;

pub fn sync_now(ctx: &Context) -> Done {
    sync::sync(ctx).map(|_| None)
}

/// Try again after the problem `kind`. What a pull brought in but could not
/// apply is applied first: a sync alone finds nothing new and calls it done.
pub fn retry(ctx: &Context, kind: &str) -> Done {
    let warning = match kind {
        "apply_failed" => {
            let _lock = adopt::lock(ctx)?;
            apply_if_applied(ctx)?
        }
        _ => None,
    };
    // The sync records that the pull is applied now, whatever the warning.
    sync_now(ctx).map(|_| warning)
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

/// Run the script item as shown. One that fails stays listed, with its log
/// to read.
pub fn run_script(ctx: &Context, id: u32, shown: ScriptItem) -> Done {
    let _lock = adopt::lock(ctx)?;
    let outcome = scripts::accept(ctx, id, &shown)?;
    Ok((!outcome.ok).then(|| tf("item.script_failed", &[("script", &shown.source)]).to_string()))
}

/// Count the script item as shown as run, without running it.
pub fn mark_run(ctx: &Context, id: u32, shown: ScriptItem) -> Done {
    let _lock = adopt::lock(ctx)?;
    scripts::ignore(ctx, id, &shown).map(|_| None)
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

/// Create a repo and import chezmoi's source into it. Encrypted files are
/// left to convert by hand: there is no terminal to ask for a passphrase.
/// When importing fails, what this created is removed, so setting up can
/// start over.
pub fn import_chezmoi(ctx: &Context) -> Result<(Report, Option<String>)> {
    let options = chezmoi::Options {
        source: None,
        dry_run: false,
        chezmoi: chezmoi::Chezmoi::find(ctx),
    };
    chezmoi::source_dir(ctx, &options)?;
    let repo_dir = ctx.paths.repo();
    let new_repo = !repo_dir.exists();
    let outcome = repo::init_new(&ctx.paths)?;
    let lock = adopt::lock(ctx)?;
    match chezmoi::import(ctx, &options) {
        Ok(report) => Ok((report, publish(ctx, &lock))),
        Err(err) => {
            if new_repo && !outcome.cloned && !outcome.existing {
                let _ = std::fs::remove_dir_all(&repo_dir);
                if outcome.host_created {
                    let _ = std::fs::remove_file(ctx.paths.host_config());
                }
            }
            Err(err)
        }
    }
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
    Ok(apply::apply(ctx)?.failures().into_iter().next())
}

/// What to hand to the system to open `path`: the file itself, or its
/// folder when the file is not there (made first when that is missing too)
/// or is a program, which the system would run instead of showing.
pub fn to_open(path: &Path) -> std::io::Result<PathBuf> {
    let program = std::fs::metadata(path)
        .is_ok_and(|file| file.is_file() && file.permissions().mode() & 0o111 != 0);
    if path.exists() && !program {
        return Ok(path.to_path_buf());
    }
    let folder = path.parent().unwrap_or(path);
    std::fs::create_dir_all(folder)?;
    Ok(folder.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::to_open;

    #[test]
    fn a_missing_file_opens_its_folder_made_when_missing_too() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a/b/file.toml");
        let folder = file.parent().unwrap();
        assert_eq!(to_open(&file).unwrap(), folder);
        assert!(folder.is_dir());
        std::fs::write(&file, "").unwrap();
        assert_eq!(to_open(&file).unwrap(), file);
    }

    #[test]
    fn a_program_opens_its_folder_so_it_is_not_run() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("setup");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        assert_eq!(to_open(&script).unwrap(), script);
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(to_open(&script).unwrap(), dir.path());
    }
}
