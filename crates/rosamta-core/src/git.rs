//! Calls to the system `git`, so the user's own config (signing, credentials) applies.

use std::path::Path;
use std::process::{Command, Output};

use crate::error::{Error, Result};

/// rosamta commits several times in a row; automatic maintenance that git
/// detaches after a commit could repack objects under the next one.
const FOREGROUND: [&str; 4] = [
    "-c",
    "gc.autoDetach=false",
    "-c",
    "maintenance.autoDetach=false",
];

fn git() -> Command {
    let mut cmd = Command::new("git");
    cmd.args(FOREGROUND)
        // The background service has no terminal to answer a prompt on.
        .env("GIT_TERMINAL_PROMPT", "0")
        // English messages, so failures can be told apart.
        .env("LC_ALL", "C");
    cmd
}

pub(crate) fn run(dir: &Path, args: &[&str]) -> Result<String> {
    let output = git().arg("-C").arg(dir).args(args).output();
    check(args, output)
}

/// Whether `git args` exits with 0; for questions such as `diff --quiet`.
pub(crate) fn succeeds(dir: &Path, args: &[&str]) -> Result<bool> {
    let output = git().arg("-C").arg(dir).args(args).output();
    output
        .map(|output| output.status.success())
        .map_err(|err| Error::Git {
            command: args.join(" "),
            message: err.to_string(),
        })
}

pub(crate) fn clone(url: &str, dest: &Path, config: &[&str]) -> Result<()> {
    let output = git()
        .args(config.iter().flat_map(|setting| ["-c", setting]))
        .args(["clone", "--quiet", url])
        .arg(dest)
        .output();
    check(&["clone", url], output).map(drop)
}

/// Why a git command failed, as far as rosamta acts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Signing a commit failed, for example with no way to ask for a passphrase.
    Signing,
    /// The remote has commits this repo lacks.
    Rejected,
    /// The remote refused the credentials or does not have the repo.
    Denied,
    /// The remote could not be reached.
    Offline,
    Other,
}

pub(crate) fn classify(message: &str) -> Failure {
    const SIGNING: [&str; 3] = [
        "gpg failed to sign",
        "error: gpg",
        "failed to write commit object",
    ];
    const REJECTED: [&str; 3] = ["[rejected]", "non-fast-forward", "fetch first"];
    const DENIED: [&str; 7] = [
        "Permission denied",
        "Authentication failed",
        "could not read Username",
        "terminal prompts disabled",
        "Repository not found",
        "error: 403",
        "Host key verification failed",
    ];
    const OFFLINE: [&str; 9] = [
        "Could not resolve host",
        "Could not read from remote repository",
        "unable to access",
        "Connection refused",
        "Connection timed out",
        "Operation timed out",
        "Network is unreachable",
        "No route to host",
        "Connection reset",
    ];
    let has = |needles: &[&str]| needles.iter().any(|needle| message.contains(needle));
    if has(&SIGNING) {
        Failure::Signing
    } else if has(&REJECTED) {
        Failure::Rejected
    } else if has(&DENIED) {
        Failure::Denied
    } else if has(&OFFLINE) {
        Failure::Offline
    } else {
        Failure::Other
    }
}

pub(crate) enum Commit {
    /// Nothing to commit.
    Clean,
    Created,
    /// Signing failed: nothing was committed and the changes stay unstaged.
    SigningFailed(String),
}

/// Commit the changes under `paths`, or every change when `paths` is empty.
/// Never adds `--no-gpg-sign`: signing follows the user's git config.
pub(crate) fn commit(dir: &Path, message: &str, paths: &[&str]) -> Result<Commit> {
    fn limit<'a>(args: &[&'a str], paths: &[&'a str]) -> Vec<&'a str> {
        let mut all = args.to_vec();
        if !paths.is_empty() {
            all.push("--");
            all.extend_from_slice(paths);
        }
        all
    }
    let with_paths = |args: &[&'static str]| limit(args, paths);
    if run(dir, &with_paths(&["status", "--porcelain"]))?
        .trim()
        .is_empty()
    {
        return Ok(Commit::Clean);
    }
    run(dir, &with_paths(&["add", "--all"]))?;
    let args = limit(&["commit", "--quiet", "--message", message], paths);
    match run(dir, &args) {
        Ok(_) => Ok(Commit::Created),
        Err(Error::Git { message, .. }) if classify(&message) == Failure::Signing => {
            run(dir, &with_paths(&["reset", "--quiet"]))?;
            Ok(Commit::SigningFailed(message))
        }
        Err(err) => Err(err),
    }
}

/// Stage everything and commit; does nothing when the tree is clean.
/// A signing failure is an error here.
pub(crate) fn commit_all(dir: &Path, message: &str) -> Result<bool> {
    match commit(dir, message, &[])? {
        Commit::Clean => Ok(false),
        Commit::Created => Ok(true),
        Commit::SigningFailed(message) => Err(Error::Git {
            command: "commit".into(),
            message,
        }),
    }
}

fn check(args: &[&str], output: std::io::Result<Output>) -> Result<String> {
    let command = args.join(" ");
    let output = output.map_err(|err| Error::Git {
        command: command.clone(),
        message: err.to_string(),
    })?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::Git { command, message });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Keep git away from the developer's config (signing, hooks, identity).
    pub(crate) fn isolate() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            for (key, value) in [
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_CONFIG_NOSYSTEM", "1"),
                ("GIT_AUTHOR_NAME", "test"),
                ("GIT_AUTHOR_EMAIL", "test@example.com"),
                ("GIT_COMMITTER_NAME", "test"),
                ("GIT_COMMITTER_EMAIL", "test@example.com"),
            ] {
                // SAFETY: set once, before any test in this binary spawns git.
                unsafe { std::env::set_var(key, value) };
            }
        });
    }

    fn failure(dir: &Path, args: &[&str]) -> Failure {
        match run(dir, args) {
            Err(Error::Git { message, .. }) => classify(&message),
            other => panic!("expected a git failure, got {other:?}"),
        }
    }

    /// A bare remote and two clones of it.
    fn remote_and_clones() -> (tempfile::TempDir, [std::path::PathBuf; 2]) {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("remote.git");
        run(dir.path(), &["init", "--quiet", "--bare", "remote.git"]).unwrap();
        let clones = ["a", "b"].map(|name| dir.path().join(name));
        for clone in &clones {
            let url = bare.to_str().unwrap();
            super::clone(url, clone, &[]).unwrap();
        }
        (dir, clones)
    }

    #[test]
    fn push_failures_are_classified() {
        isolate();
        let (dir, [a, b]) = remote_and_clones();
        for (clone, text) in [(&a, "a"), (&b, "b")] {
            std::fs::write(clone.join("file"), text).unwrap();
            assert!(commit_all(clone, "edit").unwrap());
        }
        run(&a, &["push", "--quiet", "origin", "HEAD"]).unwrap();
        assert_eq!(
            failure(&b, &["push", "--quiet", "origin", "HEAD"]),
            Failure::Rejected
        );
        std::fs::rename(dir.path().join("remote.git"), dir.path().join("moved")).unwrap();
        assert_eq!(
            failure(&b, &["push", "--quiet", "origin", "HEAD"]),
            Failure::Offline
        );
        assert_eq!(failure(&b, &["fetch", "--quiet"]), Failure::Offline);
    }

    #[test]
    fn signing_failure_leaves_no_commit() {
        isolate();
        let (_dir, [a, _]) = remote_and_clones();
        for (key, value) in [
            ("commit.gpgsign", "true"),
            ("user.signingkey", "0000000000000000"),
            ("gpg.program", "false"),
        ] {
            run(&a, &["config", key, value]).unwrap();
        }
        std::fs::write(a.join("file"), "text").unwrap();
        match commit(&a, "edit", &[]).unwrap() {
            Commit::SigningFailed(message) => assert!(message.contains("gpg"), "{message}"),
            _ => panic!("commit should fail to sign"),
        }
        assert!(run(&a, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_err());
        assert_eq!(run(&a, &["status", "--porcelain"]).unwrap(), "?? file\n");
        assert!(
            commit_all(&a, "edit")
                .unwrap_err()
                .to_string()
                .contains("gpg")
        );
    }

    #[test]
    fn classify_messages() {
        let cases = [
            (
                "error: gpg failed to sign the data\nfatal: failed to write commit object",
                Failure::Signing,
            ),
            (
                "! [rejected]        main -> main (fetch first)",
                Failure::Rejected,
            ),
            (
                "git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.",
                Failure::Denied,
            ),
            (
                "fatal: unable to access 'https://github.com/me/x.git/': Could not resolve host: github.com",
                Failure::Offline,
            ),
            (
                "ssh: connect to host github.com port 22: Network is unreachable",
                Failure::Offline,
            ),
            ("fatal: not a git repository", Failure::Other),
        ];
        for (message, expected) in cases {
            assert_eq!(classify(message), expected, "{message}");
        }
    }
}
