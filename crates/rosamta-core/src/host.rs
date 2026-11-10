//! `host.toml`: the host ID, tags and per-host preferences.

use std::path::Path;

use crate::config::HostConfig;
use crate::edit::{self, array_mut, edit_toml, rename_in_array};
use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::{git, repo, validate};

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

/// Load `host.toml`, creating it with a fresh host ID the first time.
/// Returns the config and whether it was created.
pub fn load_or_create(paths: &Paths) -> Result<(HostConfig, bool)> {
    let path = paths.host_config();
    if path.exists() {
        return Ok((load(&path)?, false));
    }
    let host = HostConfig::new(generate_id());
    let dir = path.parent().expect("host.toml has a parent directory");
    std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
    let text = toml::to_string(&host).expect("HostConfig serializes");
    edit::write_atomic(&path, &text)?;
    Ok((host, true))
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

/// Change the host ID everywhere: the repo's host list, every `when.hosts`,
/// and `host.toml`. The repo change is one commit.
pub fn rename(paths: &Paths, new_id: &str) -> Result<()> {
    check_id(new_id)?;
    let host = load(&paths.host_config())?;
    let old_id = host.id;
    if old_id == new_id {
        return Ok(());
    }
    let repo_dir = paths.repo();
    if repo_dir.join(repo::MANIFEST).is_file() {
        repo::ensure_supported(repo::read_manifest(&repo_dir)?.format)?;
        edit_toml(&repo_dir.join(repo::MANIFEST), |doc| {
            if let Some(hosts) = doc["hosts"].as_array_mut() {
                rename_in_array(hosts, &old_id, new_id);
            }
        })?;
        for module in repo::module_files(&repo_dir)? {
            edit_toml(&module, |doc| {
                if let Some(hosts) = array_mut(&mut doc["when"], "hosts") {
                    rename_in_array(hosts, &old_id, new_id);
                }
            })?;
        }
        git::commit_all(
            &repo_dir,
            &format!("chore(repo): rename host {old_id} to {new_id}"),
        )?;
    }
    edit_toml(&paths.host_config(), |doc| {
        edit::set_value(&mut doc["id"], new_id)
    })?;
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
    fn ids_are_checked() {
        assert!(check_id("work-mbp").is_ok());
        for bad in ["", "-x", "Work", "a b", "a.b"] {
            assert!(check_id(bad).unwrap_err().is_usage(), "{bad}");
        }
    }
}
