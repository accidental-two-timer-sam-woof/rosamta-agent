//! A throwaway HOME and the CLI run inside it.
#![allow(dead_code)] // each test binary uses a different part

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use assert_cmd::Command;

/// Variables that would point rosamta or a shell outside this HOME.
const OUTSIDE: [&str; 13] = [
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "LANG",
    "LC_ALL",
    "ZDOTDIR",
    "ENV",
    "BASH_ENV",
    "TERM_PROGRAM",
    "ROSAMTA_ENV",
    "ROSAMTA_PROFILE",
    "ROSAMTA_CURRENT",
];

pub struct Home {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
}

impl Home {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        Self { _dir: dir, root }
    }

    pub fn repo(&self) -> PathBuf {
        self.root.join(".local/share/rosamta/repo")
    }

    pub fn host_config(&self) -> PathBuf {
        self.root.join(".config/rosamta/host.toml")
    }

    pub fn cmd(&self, bin: &str) -> Command {
        let mut cmd = Command::cargo_bin(bin).unwrap();
        cmd.env("HOME", &self.root);
        for var in OUTSIDE {
            cmd.env_remove(var);
        }
        // Never touch the developer's keychain.
        cmd.env("ROSAMTA_NO_KEYCHAIN", "1");
        git_env(&mut cmd);
        cmd
    }

    /// Start a shell the way a terminal would, inside this HOME, with a
    /// minimal environment: nothing from the developer's leaks in or out.
    pub fn shell(&self, argv: &[String]) -> std::process::Output {
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .env_clear()
            .env("HOME", &self.root)
            .env("TERM", "dumb")
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("USER", "tester")
            .env("LOGNAME", "tester")
            .timeout(Duration::from_secs(20));
        cmd.output().unwrap()
    }

    pub fn write(&self, rel: &str, text: &str) {
        let path = self.repo().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    pub fn write_host(&self, id: &str) {
        fs::create_dir_all(self.host_config().parent().unwrap()).unwrap();
        fs::write(self.host_config(), format!("id = \"{id}\"\n")).unwrap();
    }

    /// Every file under HOME with its content, for "nothing else changed" checks.
    pub fn snapshot(&self, skip: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        fn walk(dir: &Path, skip: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.starts_with(skip) {
                    continue;
                }
                if path.is_dir() {
                    walk(&path, skip, out);
                } else {
                    out.push((path.clone(), fs::read(&path).unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, skip, &mut out);
        out.sort();
        out
    }
}

/// Keep git away from the developer's config (signing, hooks, identity).
pub fn git_env(cmd: &mut Command) {
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com");
}

pub fn git(dir: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    git_env(&mut cmd);
    cmd.arg("-C").arg(dir).args(args).assert().success();
}

pub fn has(shell: &str) -> bool {
    rosamta_core::shells::which(shell, &std::env::var("PATH").unwrap_or_default()).is_some()
}

/// Each installed shell started as a terminal would, running `posix` (fish: `fish`).
pub fn sessions(posix: &str, fish: &str) -> Vec<Vec<String>> {
    let argv = |args: &[&str]| args.iter().map(|arg| arg.to_string()).collect();
    let mut out = Vec::new();
    if has("sh") {
        out.push(argv(&["sh", "-l", "-c", posix]));
    }
    if has("bash") {
        out.push(argv(&["bash", "-l", "-i", "-c", posix]));
        out.push(argv(&["bash", "-i", "-c", posix]));
    }
    if has("zsh") {
        out.push(argv(&["zsh", "-l", "-i", "-c", posix]));
    }
    if has("fish") {
        out.push(argv(&["fish", "-l", "-i", "-c", fish]));
    }
    out
}
