//! Files and directories as whole values: hash, copy, swap into place.
//! Symlinks inside a tree are kept as symlinks, never followed.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::fnv::Fnv;

/// What a path holds, compared by name, content and symlink target.
/// Permissions are left out: they are set after every deploy. `None` when
/// nothing is there.
pub fn hash(path: &Path) -> Result<Option<String>> {
    let mut hasher = Fnv::new();
    if !feed(path, &mut hasher)? {
        return Ok(None);
    }
    Ok(Some(hasher.finish()))
}

/// The hash a regular file with `bytes` would have.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Fnv::new();
    hasher.write(b"f");
    hasher.write(bytes);
    hasher.finish()
}

fn feed(path: &Path, hasher: &mut Fnv) -> Result<bool> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(Error::read(path)(err)),
    };
    if meta.is_symlink() {
        let target = std::fs::read_link(path).map_err(Error::read(path))?;
        hasher.write(b"l");
        hasher.write(target.as_os_str().as_encoded_bytes());
    } else if meta.is_dir() {
        hasher.write(b"d");
        for name in names(path)? {
            hasher.write(name.as_encoded_bytes());
            hasher.write(&[0]);
            feed(&path.join(&name), hasher)?;
        }
        hasher.write(b"e");
    } else {
        hasher.write(b"f");
        hasher.write(&std::fs::read(path).map_err(Error::read(path))?);
    }
    Ok(true)
}

fn names(dir: &Path) -> Result<Vec<std::ffi::OsString>> {
    let mut names = std::fs::read_dir(dir)
        .map_err(Error::read(dir))?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(Error::read(dir))?;
    names.sort();
    Ok(names)
}

/// Where `path` points when it is a symlink.
pub fn link_of(path: &Path) -> Option<PathBuf> {
    std::fs::read_link(path).ok()
}

/// Every file and symlink in `path` (itself when not a directory), for
/// snapshots; a missing path stands for itself, so its absence is recorded.
pub fn files(path: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect(path, &mut out)?;
    Ok(out)
}

fn collect(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            for name in names(path)? {
                collect(&path.join(name), out)?;
            }
            Ok(())
        }
        // A link deployed by rosamta has nothing of the host's own to keep.
        Ok(meta)
            if meta.is_symlink() && std::fs::metadata(path).is_ok_and(|meta| meta.is_dir()) =>
        {
            Ok(())
        }
        _ => {
            out.push(path.to_path_buf());
            Ok(())
        }
    }
}

/// Copy `from` to a new path `to`, keeping symlinks and modes.
fn copy_new(from: &Path, to: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(from).map_err(Error::read(from))?;
    if meta.is_symlink() {
        let target = std::fs::read_link(from).map_err(Error::read(from))?;
        std::os::unix::fs::symlink(target, to).map_err(Error::write(to))
    } else if meta.is_dir() {
        std::fs::create_dir(to).map_err(Error::write(to))?;
        for name in names(from)? {
            copy_new(&from.join(&name), &to.join(&name))?;
        }
        std::fs::set_permissions(to, meta.permissions()).map_err(Error::write(to))
    } else {
        std::fs::copy(from, to).map_err(Error::write(to)).map(drop)
    }
}

/// What to put in place.
pub enum Content<'a> {
    Tree(&'a Path),
    Bytes(&'a [u8]),
}

/// Put `content` at `to`, replacing whatever is there. A file replaces in
/// one rename; a directory is built beside it and swapped in.
pub fn put(content: Content, to: &Path) -> Result<()> {
    let parent = to.parent().expect("a target has a parent");
    std::fs::create_dir_all(parent).map_err(Error::write(parent))?;
    let name = to
        .file_name()
        .expect("a target has a name")
        .to_string_lossy();
    let fresh = parent.join(format!(".{name}.rosamta-new"));
    remove(&fresh)?;
    match content {
        Content::Tree(from) => copy_new(from, &fresh)?,
        Content::Bytes(bytes) => write_private(&fresh, bytes)?,
    }
    let is_dir = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
    if is_dir(to) || is_dir(&fresh) {
        let old = parent.join(format!(".{name}.rosamta-old"));
        remove(&old)?;
        if std::fs::symlink_metadata(to).is_ok() {
            std::fs::rename(to, &old).map_err(Error::write(to))?;
        }
        std::fs::rename(&fresh, to).map_err(Error::write(to))?;
        return remove(&old);
    }
    std::fs::rename(&fresh, to).map_err(Error::write(to))
}

/// Replace `at` with a symlink to `target`.
pub fn link(target: &Path, at: &Path) -> Result<()> {
    let parent = at.parent().expect("a target has a parent");
    std::fs::create_dir_all(parent).map_err(Error::write(parent))?;
    let name = at
        .file_name()
        .expect("a target has a name")
        .to_string_lossy();
    let fresh = parent.join(format!(".{name}.rosamta-new"));
    remove(&fresh)?;
    std::os::unix::fs::symlink(target, &fresh).map_err(Error::write(&fresh))?;
    if std::fs::symlink_metadata(at).is_ok_and(|meta| meta.is_dir()) {
        // rename cannot replace a directory with a file.
        remove(at)?;
    }
    std::fs::rename(&fresh, at).map_err(Error::write(at))
}

pub fn remove(path: &Path) -> Result<()> {
    let result = match std::fs::symlink_metadata(path) {
        // A copied read-only folder (0555) keeps its children until it is writable.
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path).or_else(|err| {
            if err.kind() != std::io::ErrorKind::PermissionDenied {
                return Err(err);
            }
            writable(path);
            std::fs::remove_dir_all(path)
        }),
        Ok(_) => std::fs::remove_file(path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => Err(err),
    };
    result.map_err(Error::write(path))
}

/// Give the owner write access to `dir` and every folder in it.
fn writable(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::symlink_metadata(dir)
        && meta.is_dir()
    {
        let mode = meta.permissions().mode() | 0o700;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode));
        for name in names(dir).unwrap_or_default() {
            writable(&dir.join(name));
        }
    }
}

/// Bytes are decrypted or rendered content that may hold a secret, so the
/// file is never readable by others, even before its mode is set.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .map_err(Error::write(path))
}

/// The permission bits of a regular file, if one is there.
pub fn mode_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|meta| meta.is_file())
        .map(|meta| meta.permissions().mode() & 0o7777)
}

pub fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let current = std::fs::metadata(path).map_err(Error::read(path))?;
    if current.permissions().mode() & 0o7777 == mode {
        return Ok(());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(Error::write(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_only_folder_can_be_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        std::fs::create_dir_all(from.join("locked")).unwrap();
        std::fs::write(from.join("locked/file"), "a\n").unwrap();
        let locked = std::fs::Permissions::from_mode(0o555);
        std::fs::set_permissions(from.join("locked"), locked).unwrap();
        let to = dir.path().join("to");
        put(Content::Tree(&from), &to).unwrap();
        put(Content::Tree(&from), &to).unwrap();
        assert_eq!(
            std::fs::read_to_string(to.join("locked/file")).unwrap(),
            "a\n"
        );
        assert!(!dir.path().join(".to.rosamta-old").exists());
        remove(&to).unwrap();
        remove(&from).unwrap();
    }

    #[test]
    fn trees_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("lua")).unwrap();
        std::fs::write(src.join("init.lua"), "x").unwrap();
        std::fs::write(src.join("lua/a.lua"), "y").unwrap();
        std::os::unix::fs::symlink("init.lua", src.join("alias.lua")).unwrap();
        let dst = dir.path().join("deep/dst");
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("stale"), "old").unwrap();

        put(Content::Tree(&src), &dst).unwrap();
        assert_eq!(hash(&src).unwrap(), hash(&dst).unwrap());
        assert!(!dst.join("stale").exists());
        assert_eq!(
            link_of(&dst.join("alias.lua")),
            Some(PathBuf::from("init.lua"))
        );

        std::fs::write(dst.join("lua/a.lua"), "z").unwrap();
        assert_ne!(hash(&src).unwrap(), hash(&dst).unwrap());
        assert_eq!(hash(&dir.path().join("none")).unwrap(), None);
        assert_eq!(files(&dst).unwrap().len(), 3);
    }

    #[test]
    fn files_and_links_swap() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a/.gitconfig");
        put(Content::Bytes(b"[user]\n"), &file).unwrap();
        assert_eq!(hash(&file).unwrap(), Some(hash_bytes(b"[user]\n")));
        let source = dir.path().join("repo/.gitconfig");
        put(Content::Bytes(b"[user]\n"), &source).unwrap();
        link(&source, &file).unwrap();
        assert_eq!(link_of(&file), Some(source.clone()));
        let tree = dir.path().join("tree");
        std::fs::create_dir(&tree).unwrap();
        link(&source, &tree).unwrap();
        assert_eq!(link_of(&tree), Some(source));
        set_mode(&file, 0o600).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
