//! Versioned output directories and the atomic `current` switch.
//!
//! ```text
//! generated/
//! ├── current -> v-1794300000-1a2b3c4d
//! ├── v-1794300000-1a2b3c4d/{sh,bash,zsh,fish}/…
//! └── v-1794290000-5e6f7a8b/                      # kept for rollback
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::Shell;
use crate::error::{Error, Result};
use crate::process::output_with_timeout;
use crate::render::Layout;

pub const CURRENT: &str = "current";
/// Versions kept besides the current one.
pub const KEEP: usize = 3;

pub fn new_version_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("v-{}-{:08x}", now.as_secs(), now.subsec_nanos())
}

pub fn layout(generated: &Path, state: &Path, id: &str) -> Layout {
    Layout {
        version_dir: generated.join(id),
        version_id: id.to_string(),
        profile_log: state.join("profile.log"),
        notice: state.join("notice"),
    }
}

/// The version `current` points at, if any.
pub fn current_id(generated: &Path) -> Option<String> {
    let target = std::fs::read_link(generated.join(CURRENT)).ok()?;
    target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// Files of a version, keyed by path relative to the version dir.
pub fn read_version(generated: &Path, id: &str) -> Result<BTreeMap<PathBuf, String>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) -> Result<()> {
        for entry in std::fs::read_dir(dir).map_err(Error::read(dir))? {
            let path = entry.map_err(Error::read(dir))?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else {
                let text = std::fs::read_to_string(&path).map_err(Error::read(&path))?;
                out.insert(path.strip_prefix(root).unwrap_or(&path).to_path_buf(), text);
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    let dir = generated.join(id);
    if dir.is_dir() {
        walk(&dir, &dir, &mut files)?;
    }
    Ok(files)
}

pub fn write_version(layout: &Layout, files: &BTreeMap<PathBuf, String>) -> Result<()> {
    for (rel, text) in files {
        let path = layout.version_dir.join(rel);
        let dir = path.parent().expect("files live in a shell directory");
        std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
        std::fs::write(&path, text).map_err(Error::write(&path))?;
    }
    Ok(())
}

/// Point `current` at `id` with one `rename`, so a starting shell sees
/// either the whole old version or the whole new one.
pub fn switch(generated: &Path, id: &str) -> Result<()> {
    let tmp = generated.join(format!(".{CURRENT}.{id}"));
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(id, &tmp).map_err(Error::write(&tmp))?;
    std::fs::rename(&tmp, generated.join(CURRENT)).map_err(Error::write(generated.join(CURRENT)))
}

/// Delete all but the newest `KEEP` versions, never the current one.
pub fn prune(generated: &Path) -> Result<()> {
    let current = current_id(generated);
    let mut versions: Vec<String> = std::fs::read_dir(generated)
        .map_err(Error::read(generated))?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("v-") && Some(name) != current.as_ref())
        .collect();
    // Seconds have a fixed width for centuries, so names sort by time.
    versions.sort();
    let excess = versions.len().saturating_sub(KEEP);
    for name in &versions[..excess] {
        let dir = generated.join(name);
        std::fs::remove_dir_all(&dir).map_err(Error::write(&dir))?;
    }
    Ok(())
}

pub fn remove_version(layout: &Layout) {
    let _ = std::fs::remove_dir_all(&layout.version_dir);
}

/// The shell's own parse error for `file`, if it does not parse.
pub fn syntax_error(shell_path: &Path, shell: Shell, file: &Path) -> Option<String> {
    let mut cmd = std::process::Command::new(shell_path);
    match shell {
        Shell::Fish => cmd.arg("--no-execute"),
        _ => cmd.arg("-n"),
    };
    let output = output_with_timeout(cmd.arg(file), Duration::from_secs(5)).ok()??;
    if output.status.success() {
        return None;
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Some(if message.is_empty() {
        "syntax error".into()
    } else {
        message
    })
}

/// Whether `code` parses as complete code for `shell`.
pub fn parses(shell_path: &Path, shell: Shell, code: &str) -> bool {
    let mut cmd = std::process::Command::new(shell_path);
    match shell {
        Shell::Fish => cmd.args(["--no-config", "--no-execute"]),
        Shell::Zsh => cmd.args(["-f", "-n"]),
        _ => cmd.arg("-n"),
    };
    cmd.arg("-c")
        .arg(code)
        .env_remove("BASH_ENV")
        .env_remove("ENV");
    output_with_timeout(&mut cmd, Duration::from_secs(5))
        .ok()
        .flatten()
        .is_some_and(|output| output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_is_a_symlink_swap_and_prune_keeps_recent() {
        let dir = tempfile::tempdir().unwrap();
        let generated = dir.path();
        for n in 1..=6 {
            let id = format!("v-179430000{n}-00000000");
            let files = BTreeMap::from([(PathBuf::from("sh/env.sh"), format!("# {n}\n"))]);
            write_version(&layout(generated, generated, &id), &files).unwrap();
            switch(generated, &id).unwrap();
        }
        assert_eq!(
            current_id(generated).as_deref(),
            Some("v-1794300006-00000000")
        );
        assert!(
            std::fs::symlink_metadata(generated.join(CURRENT))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        prune(generated).unwrap();
        let mut left: Vec<_> = std::fs::read_dir(generated)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "current",
                "v-1794300003-00000000",
                "v-1794300004-00000000",
                "v-1794300005-00000000",
                "v-1794300006-00000000"
            ]
        );
        let files = read_version(generated, "v-1794300006-00000000").unwrap();
        assert_eq!(files[Path::new("sh/env.sh")], "# 6\n");
    }

    #[test]
    fn syntax_errors_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.sh");
        let bad = dir.path().join("bad.sh");
        std::fs::write(&good, "echo ok\n").unwrap();
        std::fs::write(&bad, "if then fi\n").unwrap();
        let sh = Path::new("/bin/sh");
        assert_eq!(syntax_error(sh, Shell::Sh, &good), None);
        assert!(syntax_error(sh, Shell::Sh, &bad).is_some());
    }
}
