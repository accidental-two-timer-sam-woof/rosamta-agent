//! `rsmt add` and `rsmt forget`: start and stop managing a dotfile.

use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use super::tree::{self, Content};
use super::{MANIFEST, crypt, deploy, review, state};
use crate::apply::Context;
use crate::config::{Deploy, Dotfile};
use crate::error::{Error, Result};
use crate::sync;

/// `path` relative to the home directory, without resolving symlinks.
fn home_relative(ctx: &Context, path: &Path) -> Result<PathBuf> {
    let mut abs = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                abs.pop();
            }
            Component::CurDir => {}
            other => abs.push(other),
        }
    }
    // A home behind a symlink: compare resolved directories, but never
    // resolve the path itself, which may be a link.
    let real = || {
        let parent = std::fs::canonicalize(abs.parent()?).ok()?;
        Some((
            std::fs::canonicalize(&ctx.home).ok()?,
            parent.join(abs.file_name()?),
        ))
    };
    abs.strip_prefix(&ctx.home)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            let (home, abs) = real()?;
            abs.strip_prefix(home).ok().map(Path::to_path_buf)
        })
        .filter(|rel| !rel.as_os_str().is_empty())
        .ok_or_else(|| {
            Error::Usage(format!(
                "{} is not inside the home directory",
                path.display()
            ))
        })
}

/// rosamta's own directories never become dotfiles, nor anything holding one.
fn refuse_own(ctx: &Context, target: &Path) -> Result<()> {
    let config = ctx.paths.host_config();
    let own = [
        ctx.paths.repo(),
        ctx.paths.generated(),
        ctx.paths.state().to_path_buf(),
        config
            .parent()
            .expect("host.toml has a parent")
            .to_path_buf(),
    ];
    if own
        .iter()
        .any(|dir| target.starts_with(dir) || dir.starts_with(target))
    {
        return Err(Error::Usage(format!(
            "{} holds rosamta's own files",
            target.display()
        )));
    }
    Ok(())
}

fn refuse_managed(repo: &Path, rel: &Path) -> Result<()> {
    for entry in super::load(repo)?.dotfile {
        let Some(path) = super::clean(&entry.path) else {
            continue;
        };
        if rel.starts_with(&path) || path.starts_with(rel) {
            return Err(Error::Usage(format!(
                "~/{} is already managed as dotfile `{}`",
                rel.display(),
                entry.path
            )));
        }
    }
    Ok(())
}

/// The mode to keep when the group and others have no access, which git
/// would not carry to another host.
fn private_mode(path: &Path) -> Result<Option<String>> {
    let mode = std::fs::metadata(path)
        .map_err(Error::read(path))?
        .permissions()
        .mode()
        & 0o777;
    Ok((mode & 0o077 == 0).then(|| format!("{mode:04o}")))
}

/// Move each path into the repo's `home/`, declare it and deploy it back.
/// Encrypted dotfiles need the key unlocked on this host. Returns the paths
/// relative to the home directory. The caller holds the adopt lock.
pub fn add(ctx: &Context, paths: &[PathBuf], copy: bool, encrypt: bool) -> Result<Vec<PathBuf>> {
    let repo = ctx.paths.repo();
    // Check every path before writing anything.
    let mut entries: Vec<(PathBuf, Dotfile)> = Vec::new();
    for path in paths {
        let rel = home_relative(ctx, path)?;
        if entries.iter().any(|(seen, _)| *seen == rel) {
            continue;
        }
        let target = ctx.home.join(&rel);
        refuse_own(ctx, &target)?;
        refuse_managed(&repo, &rel)?;
        let meta = std::fs::symlink_metadata(&target).map_err(Error::read(&target))?;
        if meta.is_symlink() {
            return Err(Error::Usage(format!(
                "{} is a symlink; add the file it points to",
                target.display()
            )));
        }
        if encrypt && meta.is_dir() {
            return Err(Error::Usage(format!(
                "{} is a directory; only files can be encrypted",
                target.display()
            )));
        }
        if !encrypt && let Some((file, rule)) = review::secret_in(&target)? {
            return Err(Error::Usage(format!(
                "{} looks like it contains a secret ({rule}); add it with --encrypt",
                file.display()
            )));
        }
        let mut entry = Dotfile::new(&rel.to_string_lossy());
        entry.deploy = if copy || encrypt {
            Deploy::Copy
        } else {
            Deploy::Link
        };
        entry.encrypted = encrypt;
        entry.permissions = private_mode(&target)?;
        if std::fs::symlink_metadata(repo.join(entry.source())).is_ok() {
            return Err(Error::Usage(format!(
                "{} already exists in the repo",
                entry.source()
            )));
        }
        entries.push((rel, entry));
    }
    let mut changed = vec![MANIFEST.to_string()];
    for (rel, entry) in &entries {
        let target = ctx.home.join(rel);
        let source = repo.join(entry.source());
        if entry.encrypted {
            let plain = std::fs::read(&target).map_err(Error::read(&target))?;
            tree::put(Content::Bytes(&crypt::encrypt(ctx, &plain)?), &source)?;
        } else {
            tree::put(Content::Tree(&target), &source)?;
        }
        review::append(&repo, entry)?;
        changed.push(entry.source());
    }
    if encrypt {
        changed.push(crypt::REPO_KEY.to_string());
    }
    let names: Vec<String> = entries
        .iter()
        .map(|(rel, _)| rel.display().to_string())
        .collect();
    let changed: Vec<&str> = changed.iter().map(String::as_str).collect();
    sync::commit_scanned(
        ctx,
        &changed,
        Some(&format!("add(home): {}", names.join(", "))),
    )?;
    deploy::deploy(ctx)?;
    Ok(entries.into_iter().map(|(rel, _)| rel).collect())
}

/// Stop managing `path` on every host: its declarations and repo content
/// go; a link here becomes a regular copy. Other hosts do the same when
/// they next deploy. The caller holds the adopt lock.
pub fn forget(ctx: &Context, path: &Path) -> Result<PathBuf> {
    let repo = ctx.paths.repo();
    let rel = home_relative(ctx, path)?;
    let entries: Vec<Dotfile> = super::load(&repo)?
        .dotfile
        .into_iter()
        .filter(|entry| super::clean(&entry.path).as_deref() == Some(rel.as_path()))
        .collect();
    if entries.is_empty() {
        return Err(Error::Usage(format!(
            "~/{} is not a dotfile",
            rel.display()
        )));
    }
    let target = ctx.home.join(&rel);
    let shown = tree::link_of(&target).filter(|link| {
        entries
            .iter()
            .any(|entry| *link == repo.join(entry.source()))
    });
    if let Some(source) = shown {
        tree::put(Content::Tree(&source), &target)?;
    }
    let name = rel.to_string_lossy().into_owned();
    crate::edit::edit_toml(&repo.join(MANIFEST), |doc| {
        if let Some(list) = doc
            .get_mut("dotfile")
            .and_then(|item| item.as_array_of_tables_mut())
        {
            list.retain(|table| {
                table
                    .get("path")
                    .and_then(|path| path.as_str())
                    .and_then(super::clean)
                    .is_none_or(|path| path.to_string_lossy() != name)
            });
        }
    })?;
    let mut changed = vec![MANIFEST.to_string()];
    for entry in &entries {
        tree::remove(&repo.join(entry.source()))?;
        changed.push(entry.source());
    }
    let mut deployed = state::load(ctx.paths.state())?;
    deployed.entries.remove(&rel);
    deployed.ignored.remove(&rel);
    state::save(ctx.paths.state(), &deployed)?;
    let changed: Vec<&str> = changed.iter().map(String::as_str).collect();
    sync::commit_scanned(ctx, &changed, Some(&format!("forget(home): {name}")))?;
    deploy::deploy(ctx)?;
    Ok(rel)
}
