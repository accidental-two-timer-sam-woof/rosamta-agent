//! `rsmt add` and `rsmt forget`: start and stop managing a dotfile.

use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use super::state::Deployed;
use super::tree::{self, Content};
use super::{MANIFEST, crypt, deploy, review, state};
use crate::apply::Context;
use crate::config::{Deploy, Dotfile};
use crate::error::{Error, Result};
use crate::sync;

/// `path` relative to the home directory, without resolving symlinks.
pub fn home_relative(ctx: &Context, path: &Path) -> Result<PathBuf> {
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

/// Shell startup files stay out, wherever this host's shells read them.
fn refuse_startup(ctx: &Context, rel: &Path, target: &Path) -> Result<()> {
    let watched = crate::adopt::watched_files(&ctx.hook_env(&ctx.installed()));
    let file = watched
        .iter()
        .map(|watched| watched.path.clone())
        .find(|file| file.starts_with(target))
        .or_else(|| super::startup_file(rel).map(|file| ctx.home.join(file)));
    match file {
        Some(file) => Err(Error::Usage(format!(
            "{} is a shell startup file; keep shell settings in modules",
            file.display()
        ))),
        None => Ok(()),
    }
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
pub fn add(
    ctx: &Context,
    paths: &[PathBuf],
    deploy: Deploy,
    encrypt: bool,
) -> Result<Vec<PathBuf>> {
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
        refuse_startup(ctx, &rel, &target)?;
        refuse_managed(&repo, &rel)?;
        let meta = std::fs::symlink_metadata(&target).map_err(Error::read(&target))?;
        if meta.is_symlink() {
            return Err(Error::Usage(format!(
                "{} is a symlink; add the file it points to",
                target.display()
            )));
        }
        if meta.is_dir() && tree::files(&target)?.is_empty() {
            return Err(Error::Usage(format!(
                "{} is an empty directory, which git cannot keep",
                target.display()
            )));
        }
        if encrypt && meta.is_dir() {
            return Err(Error::Usage(format!(
                "{} is a directory; only files can be encrypted",
                target.display()
            )));
        }
        if deploy == Deploy::Template {
            check_template(&target, &meta)?;
        }
        if !encrypt && let Some((file, rule)) = review::secret_in(&target)? {
            return Err(Error::Usage(format!(
                "{} looks like it contains a secret ({rule}); add it with --encrypt",
                file.display()
            )));
        }
        let mut entry = Dotfile::new(&rel.to_string_lossy());
        // An encrypted file cannot be a link.
        entry.deploy = match deploy {
            Deploy::Link if encrypt => Deploy::Copy,
            deploy => deploy,
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

/// A template is a text file minijinja can read; it is added as it is and
/// the parts that differ by host are edited in the repo afterwards.
fn check_template(target: &Path, meta: &std::fs::Metadata) -> Result<()> {
    if meta.is_dir() {
        return Err(Error::Usage(format!(
            "{} is a directory; only files can be templates",
            target.display()
        )));
    }
    let bytes = std::fs::read(target).map_err(Error::read(target))?;
    let text = String::from_utf8(bytes).map_err(|_| {
        Error::Usage(format!(
            "{} is not UTF-8 text; a template must be text",
            target.display()
        ))
    })?;
    crate::template::check(&target.display().to_string(), &text).map_err(|err| {
        Error::Usage(format!(
            "{} is not a valid template: {err}",
            target.display()
        ))
    })
}

/// Stop managing `path` on every host: its declarations and repo content
/// go; a link here becomes a regular copy. Other hosts do the same when
/// they next deploy. The caller holds the adopt lock.
pub fn forget(ctx: &Context, path: &Path) -> Result<PathBuf> {
    let repo = ctx.paths.repo();
    let rel = home_relative(ctx, path)?;
    let (entries, others): (Vec<Dotfile>, Vec<Dotfile>) = super::load(&repo)?
        .dotfile
        .into_iter()
        .partition(|entry| super::clean(&entry.path).as_deref() == Some(rel.as_path()));
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
        let source = entry.source();
        // Another declaration may show the same source, or a folder around or
        // inside it, however it spells the path; one that is unclear is kept.
        let shared = others.iter().any(|other| {
            match (super::clean(&source), super::clean(&other.source())) {
                (Some(mine), Some(theirs)) => {
                    theirs.starts_with(&mine) || mine.starts_with(&theirs)
                }
                _ => true,
            }
        });
        if !shared {
            tree::remove(&repo.join(&source))?;
            changed.push(source);
        }
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

/// Deployed targets that are rosamta's links: each target with the repo
/// source it shows.
fn our_links(ctx: &Context, deployed: &Deployed) -> Vec<(PathBuf, PathBuf)> {
    let repo = ctx.paths.repo();
    deployed
        .entries
        .iter()
        .filter(|(_, record)| record.deploy == Deploy::Link)
        .filter_map(|(rel, record)| {
            let target = ctx.home.join(rel);
            let source = repo.join(&record.source);
            (tree::link_of(&target)? == source).then_some((target, source))
        })
        .collect()
}

/// Stop tracking what was deployed, and drop its review items.
fn forget_deployed(ctx: &Context) -> Result<()> {
    state::save(ctx.paths.state(), &Deployed::default())?;
    review::sync_queue(ctx, Vec::new())
}

/// `rsmt eject`: each link becomes a regular copy of what it shows, so the
/// home works without the repo. Returns the targets that were links.
pub fn eject(ctx: &Context) -> Result<Vec<PathBuf>> {
    let links = our_links(ctx, &state::load(ctx.paths.state())?);
    for (target, source) in &links {
        tree::put(Content::Tree(source), target)?;
    }
    forget_deployed(ctx)?;
    Ok(links.into_iter().map(|(target, _)| target).collect())
}

/// Before `rsmt restore --original`: remove rosamta's links, so putting the
/// original files back never writes through them into the repo.
pub fn unlink(ctx: &Context) -> Result<()> {
    for (target, _) in our_links(ctx, &state::load(ctx.paths.state())?) {
        tree::remove(&target)?;
    }
    forget_deployed(ctx)
}
