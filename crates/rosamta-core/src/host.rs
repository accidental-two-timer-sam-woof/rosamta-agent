//! `host.toml`: the host ID, tags and per-host preferences.

use std::path::Path;

use toml_edit::{Array, Item};

use crate::config::{HostConfig, RepoManifest};
use crate::edit::{self, TomlFile, array_mut, edit_toml, rename_in_array};
use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::{repo, validate};

pub fn load(path: &Path) -> Result<HostConfig> {
    let text = std::fs::read_to_string(path).map_err(Error::read(path))?;
    validate::parse(&text).map_err(|issue| {
        let line = issue
            .line
            .map(|line| format!(":{line}"))
            .unwrap_or_default();
        Error::Usage(format!("{}{line}: {}", path.display(), issue.message))
    })
}

/// Load `host.toml`, creating it the first time with a fresh host ID that no
/// host in `taken` has. Returns the config and, when it was created, the ID
/// generated from the hostname (the config's own unless that one was taken).
pub fn load_or_create(paths: &Paths, taken: &[String]) -> Result<(HostConfig, Option<String>)> {
    let path = paths.host_config();
    if path.exists() {
        return Ok((load(&path)?, None));
    }
    let generated = generate_id();
    let host = HostConfig::new(unique_id(&generated, taken));
    let dir = path.parent().expect("host.toml has a parent directory");
    std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
    let text = toml::to_string(&host).expect("HostConfig serializes");
    edit::write_atomic(&path, &text)?;
    Ok((host, Some(generated)))
}

/// `id`, or the first of `id-2`, `id-3`, ... that no host in `taken` has.
fn unique_id(id: &str, taken: &[String]) -> String {
    std::iter::once(id.to_string())
        .chain((2..).map(|n| format!("{id}-{n}")))
        .find(|candidate| !taken.contains(candidate))
        .expect("taken is finite")
}

/// A host ID derived from this machine's name. Called only when `host.toml`
/// is first created, so later hostname changes do not affect it.
pub fn generate_id() -> String {
    let name = if cfg!(target_os = "macos") {
        scutil_local_host_name()
    } else {
        None
    };
    sanitize(&name.or_else(system_hostname).unwrap_or_default())
}

fn scutil_local_host_name() -> Option<String> {
    let output = std::process::Command::new("scutil")
        .args(["--get", "LocalHostName"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn system_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for `buf.len()` bytes, and gethostname writes at most that many.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|&byte| byte == 0).unwrap_or(buf.len());
    Some(String::from_utf8_lossy(&buf[..end]).into_owned())
}

/// Keep the part before the first dot, lowercase it, and turn every run of
/// other characters into a single `-`.
pub fn sanitize(name: &str) -> String {
    let mut id = String::new();
    for c in name.split('.').next().unwrap_or_default().chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c.to_ascii_lowercase());
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_end_matches('-');
    if id.is_empty() {
        "host".into()
    } else {
        id.into()
    }
}

fn check_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(Error::Usage(format!(
            "invalid host ID `{id}`: use lowercase letters, digits and '-', not starting with '-'"
        )))
    }
}

pub fn add_tag(paths: &Paths, tag: &str) -> Result<bool> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Err(Error::Usage("a tag cannot be empty".into()));
    }
    let host = load(&paths.host_config())?;
    edit_toml(&paths.host_config(), |doc| {
        edit::set_sorted_strings(
            &mut doc["tags"],
            host.tags.into_iter().chain([tag.to_string()]),
        );
    })
}

pub fn remove_tag(paths: &Paths, tag: &str) -> Result<bool> {
    let host = load(&paths.host_config())?;
    edit_toml(&paths.host_config(), |doc| {
        edit::set_sorted_strings(
            &mut doc["tags"],
            host.tags.into_iter().filter(|kept| kept != tag),
        );
    })
}

/// Change the host ID everywhere: the repo's host list, every `when.hosts`
/// in repo and local modules and in dotfiles, and `host.toml`. The repo change is one commit.
pub fn rename(paths: &Paths, new_id: &str) -> Result<()> {
    check_id(new_id)?;
    let old_id = load(&paths.host_config())?.id;
    if old_id == new_id {
        return Ok(());
    }
    let repo_dir = paths.repo();
    let manifest = repo_dir.join(repo::MANIFEST);
    let dotfiles = repo_dir.join(crate::dotfiles::MANIFEST);
    let mut files = Vec::new();
    if manifest.is_file() {
        let RepoManifest { format, hosts } = repo::read_manifest(&repo_dir)?;
        repo::ensure_supported(format)?;
        if hosts.iter().any(|id| id == new_id) {
            return Err(Error::Usage(format!(
                "host ID `{new_id}` is already used by another host"
            )));
        }
        files.push(manifest.clone());
        if dotfiles.is_file() {
            files.push(dotfiles.clone());
        }
        files.extend(repo::module_files(&repo_dir)?);
    }
    files.extend(repo::module_files(&paths.local())?);
    // Parse every file before writing any, so a broken one leaves nothing half renamed.
    let mut files: Vec<TomlFile> = files
        .iter()
        .map(|path| TomlFile::read(path))
        .collect::<Result<_>>()?;
    let mut host_file = TomlFile::read(&paths.host_config())?;

    let mut changed = Vec::new();
    for file in &mut files {
        let lists: Vec<&mut Array> = if file.path == manifest {
            file.doc["hosts"].as_array_mut().into_iter().collect()
        } else if file.path == dotfiles {
            file.doc
                .get_mut("dotfile")
                .and_then(Item::as_array_of_tables_mut)
                .into_iter()
                .flat_map(|entries| entries.iter_mut())
                .filter_map(|entry| array_mut(entry.get_mut("when")?, "hosts"))
                .collect()
        } else {
            array_mut(&mut file.doc["when"], "hosts")
                .into_iter()
                .collect()
        };
        for hosts in lists {
            rename_in_array(hosts, &old_id, new_id);
        }
        if file.save()?
            && let Ok(rel) = file.path.strip_prefix(&repo_dir)
        {
            changed.push(rel.to_string_lossy().into_owned());
        }
    }
    edit::set_value(&mut host_file.doc["id"], new_id);
    host_file.save()?;
    // Last, so a failed commit leaves the rename done, only not committed yet.
    let changed: Vec<&str> = changed.iter().map(String::as_str).collect();
    repo::commit_files(
        &repo_dir,
        &format!("chore(repo): rename host {old_id} to {new_id}"),
        &changed,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_names() {
        assert_eq!(sanitize("xinshoutw-MacBook-Pro"), "xinshoutw-macbook-pro");
        assert_eq!(sanitize("lab-server.example.com"), "lab-server");
        assert_eq!(sanitize("Sam's  Mac_mini"), "sam-s-mac-mini");
        assert_eq!(sanitize("--box--"), "box");
        assert_eq!(sanitize(""), "host");
    }

    #[test]
    fn generated_id_is_usable() {
        assert!(check_id(&generate_id()).is_ok());
    }

    #[test]
    fn unique_ids_skip_taken_ones() {
        let taken = ["mbp", "mbp-2", "lab"].map(String::from);
        assert_eq!(unique_id("mbp", &taken), "mbp-3");
        assert_eq!(unique_id("work", &taken), "work");
    }

    #[test]
    fn ids_are_checked() {
        assert!(check_id("work-mbp").is_ok());
        for bad in ["", "-x", "Work", "a b", "a.b"] {
            assert!(check_id(bad).unwrap_err().is_usage(), "{bad}");
        }
    }
}
