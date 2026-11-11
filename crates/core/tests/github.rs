//! rosamta's GitHub credential helper goes into the repo, never into the
//! user's global git config.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// A global git config with content, shared by this test binary.
fn global_config() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        let path = dir.join("gitconfig");
        std::fs::write(
            &path,
            "[user]\n\tname = me\n[credential]\n\thelper = osxkeychain\n",
        )
        .unwrap();
        // SAFETY: set once, before this binary spawns git.
        unsafe {
            std::env::set_var("GIT_CONFIG_GLOBAL", &path);
            std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        }
        path
    })
}

#[test]
fn https_helper_leaves_global_git_config_alone() {
    let global = global_config();
    let before = std::fs::read(global).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let status = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["init", "--quiet"])
        .status()
        .unwrap();
    assert!(status.success());

    rosamta_core::github::configure_https(repo.path(), "/usr/bin/rosamta").unwrap();
    assert_eq!(std::fs::read(global).unwrap(), before);
    let local = std::fs::read_to_string(repo.path().join(".git/config")).unwrap();
    assert!(local.contains("git-credential"), "{local}");

    assert!(rosamta_core::github::remove_helper(repo.path()).unwrap());
    assert_eq!(std::fs::read(global).unwrap(), before);
}
