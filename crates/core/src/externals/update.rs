//! `rsmt externals update`: move each lock to the newest version, for
//! every host at once.

use std::path::PathBuf;

use serde::Serialize;

use super::{Kind, MANIFEST, archive, git, load};
use crate::apply::Context;
use crate::dotfiles::clean;
use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lock {
    /// Relative to the home directory.
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,
    pub new: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Updated {
    pub locks: Vec<Lock>,
    /// Declarations whose newest version could not be found.
    pub failed: Vec<Failure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub path: PathBuf,
    pub reason: String,
}

/// Lock every declaration, or those at `only`, to its newest version,
/// whatever its condition, and commit. The caller holds the adopt lock.
pub fn update(ctx: &Context, only: &[PathBuf]) -> Result<Updated> {
    let repo = ctx.paths.repo();
    let manifest = load(&repo)?;
    let declared: Vec<Option<PathBuf>> = manifest
        .external
        .iter()
        .map(|entry| clean(&entry.path))
        .collect();
    if let Some(stray) = only
        .iter()
        .find(|path| !declared.contains(&Some(path.to_path_buf())))
    {
        return Err(Error::Usage(format!(
            "~/{} is not an external",
            stray.display()
        )));
    }
    let mut out = Updated::default();
    let mut changed = Vec::new();
    for (index, entry) in manifest.external.iter().enumerate() {
        let Some(path) = declared[index].clone() else {
            continue;
        };
        if !only.is_empty() && !only.contains(&path) {
            continue;
        }
        let newest = match entry.kind() {
            None => Err(Error::Usage("set `git` or `archive`".into())),
            Some(Kind::Git) => git::remote_head(&ctx.home, entry.url(), entry.branch.as_deref())
                .map(|rev| ("rev", rev)),
            Some(Kind::Archive) => newest_archive(ctx, entry.url()).map(|sum| ("sha256", sum)),
        };
        match newest {
            Ok((field, new)) if entry.lock() != Some(new.as_str()) => {
                changed.push((index, field));
                out.locks.push(Lock {
                    path,
                    old: entry.lock().map(str::to_string),
                    new,
                });
            }
            Ok(_) => {}
            Err(err) => out.failed.push(Failure {
                path,
                reason: err.to_string(),
            }),
        }
    }
    if changed.is_empty() {
        // Locks an earlier run wrote but could not commit, or a hand edit.
        crate::sync::commit_scanned(ctx, &[MANIFEST], None)?;
        return Ok(out);
    }
    crate::edit::edit_toml(&repo.join(MANIFEST), |doc| {
        let Some(list) = doc
            .get_mut("external")
            .and_then(|item| item.as_array_of_tables_mut())
        else {
            return;
        };
        for ((index, field), lock) in changed.iter().zip(&out.locks) {
            if let Some(table) = list.get_mut(*index) {
                table[*field] = toml_edit::value(lock.new.as_str());
            }
        }
    })?;
    let names: Vec<String> = out
        .locks
        .iter()
        .map(|lock| lock.path.display().to_string())
        .collect();
    crate::sync::commit_scanned(
        ctx,
        &[MANIFEST],
        Some(&format!("update(externals): {}", names.join(", "))),
    )?;
    Ok(out)
}

/// The SHA-256 of what `url` serves now.
fn newest_archive(ctx: &Context, url: &str) -> Result<String> {
    let dir = ctx.paths.state();
    std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
    let file = dir.join("externals-download");
    let sum = archive::download(url, &file);
    let _ = std::fs::remove_file(&file);
    sum
}
