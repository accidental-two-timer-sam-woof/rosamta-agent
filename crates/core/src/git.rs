//! Calls to the system `git`, so the user's own config (signing, credentials) applies.

use std::path::{Path, PathBuf};
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

/// Long enough to clone a large repo, short enough that the background
/// service is not held for good.
const FOREIGN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Set when rosamta runs from a git hook; each would point git at that
/// repository instead of the one `-C` names.
const REPO_VARS: [&str; 7] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

fn git() -> Command {
    let mut cmd = Command::new("git");
    cmd.args(FOREGROUND)
        // The background service has no terminal to answer a prompt on.
        .env("GIT_TERMINAL_PROMPT", "0")
        // English messages, so failures can be told apart.
        .env("LC_ALL", "C");
    for var in REPO_VARS {
        cmd.env_remove(var);
    }
    cmd
}

pub(crate) fn run(dir: &Path, args: &[&str]) -> Result<String> {
    let output = git().arg("-C").arg(dir).args(args).output();
    check(args, output)
}

/// `run` for a repository the repo only names, such as an external: plain
/// transports only, nothing that clone's own config could run, no prompt
/// even in a terminal, and a stalled or endless transfer gives up.
pub(crate) fn run_foreign(dir: &Path, args: &[&str]) -> Result<String> {
    let mut cmd = git();
    cmd.env("GIT_ALLOW_PROTOCOL", "https:http:ssh:git:file")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "http.lowSpeedLimit=1000",
            "-c",
            "http.lowSpeedTime=60",
        ])
        .arg("-C")
        .arg(dir)
        .args(args);
    let output =
        crate::process::output_with_timeout(&mut cmd, FOREIGN_TIMEOUT).and_then(|output| {
            output.ok_or_else(|| {
                std::io::Error::other(format!(
                    "gave up after {} minutes",
                    FOREIGN_TIMEOUT.as_secs() / 60
                ))
            })
        });
    check(args, output)
}

/// Which of `paths` the repo's or the user's ignore rules leave out.
pub(crate) fn ignored(dir: &Path, paths: &[String]) -> Result<Vec<String>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = vec!["check-ignore", "--"];
    args.extend(paths.iter().map(String::as_str));
    let output = git().arg("-C").arg(dir).args(&args).output();
    match output {
        // 1: none is ignored.
        Ok(output) if output.status.code() == Some(1) => Ok(Vec::new()),
        output => Ok(check(&args, output)?.lines().map(str::to_string).collect()),
    }
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
    const SIGNING: [&str; 2] = ["gpg failed to sign", "error: gpg"];
    // A failed SSH signature shows only as a commit git could not write, as
    // does an object store git cannot write to.
    const STORE: [&str; 4] = [
        "insufficient permission",
        "No space left",
        "Disk quota",
        "Read-only file system",
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
    let unwritten = message.contains("failed to write commit object") && !has(&STORE);
    if has(&SIGNING) || unwritten {
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

/// A rebase stopped partway, by a conflict or a killed process.
pub(crate) fn rebasing(dir: &Path) -> bool {
    ["rebase-merge", "rebase-apply"].into_iter().any(|name| {
        run(dir, &["rev-parse", "--git-path", name])
            .is_ok_and(|path| dir.join(path.trim()).exists())
    })
}

/// `rebase --abort` drops a commit made mid-rebase, and one on a detached
/// HEAD is on no branch.
pub(crate) fn ensure_on_branch(dir: &Path) -> Result<()> {
    if rebasing(dir) {
        return Err(Error::Usage("a rebase is in progress in the repo".into()));
    }
    if !succeeds(dir, &["symbolic-ref", "--quiet", "HEAD"])? {
        return Err(Error::Usage("the repo is not on a branch".into()));
    }
    Ok(())
}

fn limit<'a>(args: &[&'a str], paths: &[&'a str]) -> Vec<&'a str> {
    let mut all = args.to_vec();
    if !paths.is_empty() {
        all.push("--");
        all.extend_from_slice(paths);
    }
    all
}

/// Commit the changes under `paths`, or every change when `paths` is empty.
/// Never adds `--no-gpg-sign`: signing follows the user's git config.
pub(crate) fn commit(dir: &Path, message: &str, paths: &[&str]) -> Result<Commit> {
    ensure_on_branch(dir)?;
    if run(dir, &limit(&["status", "--porcelain"], paths))?
        .trim()
        .is_empty()
    {
        return Ok(Commit::Clean);
    }
    run(dir, &limit(&["add", "--all"], paths))?;
    let args = limit(&["commit", "--quiet", "--message", message], paths);
    finish(dir, run(dir, &args), paths)
}

/// Commit what is staged under `paths` (everything staged when empty) and
/// add nothing; other staged changes stay staged and out of the commit. On
/// failure `paths` end up unstaged again.
pub(crate) fn commit_staged(dir: &Path, message: &str, paths: &[&str]) -> Result<Commit> {
    if succeeds(dir, &limit(&["diff", "--cached", "--quiet"], paths))? {
        return Ok(Commit::Clean);
    }
    let commit = ["commit", "--quiet", "--message", message];
    if paths.is_empty() {
        return finish(dir, run(dir, &commit), paths);
    }
    // `commit -- paths` would take the files as they are now, not as staged
    // and scanned: commit from an index of HEAD plus their staged entries.
    let index =
        PathBuf::from(run(dir, &["rev-parse", "--absolute-git-dir"])?.trim()).join("rosamta-index");
    let result = commit_from(dir, &index, &commit, paths);
    let _ = std::fs::remove_file(&index);
    finish(dir, result, paths)
}

/// Run `commit` with `index` holding HEAD and the staged entries of `paths`.
fn commit_from(dir: &Path, index: &Path, commit: &[&str], paths: &[&str]) -> Result<String> {
    let scoped = |args: &[&str]| {
        let output = git()
            .arg("-C")
            .arg(dir)
            .env("GIT_INDEX_FILE", index)
            .args(args)
            .output();
        check(args, output)
    };
    if succeeds(dir, &["rev-parse", "--verify", "--quiet", "HEAD"])? {
        scoped(&["read-tree", "HEAD"])?;
    } else {
        scoped(&["read-tree", "--empty"])?;
    }
    scoped(&limit(
        &["rm", "--cached", "-r", "--quiet", "--ignore-unmatch"],
        paths,
    ))?;
    let staged = run(dir, &limit(&["ls-files", "--stage", "-z"], paths))?;
    let entries: Vec<String> = staged
        .split('\0')
        .filter_map(|entry| {
            let (meta, path) = entry.split_once('\t')?;
            let mut meta = meta.split(' ');
            Some(format!("{},{},{path}", meta.next()?, meta.next()?))
        })
        .collect();
    if !entries.is_empty() {
        let mut args = vec!["update-index", "--add"];
        for entry in &entries {
            args.extend(["--cacheinfo", entry.as_str()]);
        }
        scoped(&args)?;
    }
    scoped(commit)
}

/// After `git commit`; whatever stopped it, `paths` end up unstaged again.
fn finish(dir: &Path, result: Result<String>, paths: &[&str]) -> Result<Commit> {
    let Err(err) = result else {
        return Ok(Commit::Created);
    };
    run(dir, &limit(&["reset", "--quiet"], paths))?;
    match err {
        Error::Git { message, .. } if classify(&message) == Failure::Signing => {
            Ok(Commit::SigningFailed(message))
        }
        err => Err(err),
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
        // A hook can fail silently, and `commit` explains on stdout.
        let text: Vec<String> = [&output.stderr, &output.stdout]
            .into_iter()
            .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string())
            .filter(|text| !text.is_empty())
            .collect();
        let message = if text.is_empty() {
            output.status.to_string()
        } else {
            text.join("\n")
        };
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

    /// What was staged and scanned goes in, not a later edit; new folders
    /// and deletions too, and other staged changes stay staged.
    #[test]
    fn a_scoped_commit_takes_what_was_staged() {
        isolate();
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        run(repo, &["init", "--quiet"]).unwrap();
        std::fs::write(repo.join("a"), "1\n").unwrap();
        std::fs::write(repo.join("gone"), "g\n").unwrap();
        commit_all(repo, "base").unwrap();
        std::fs::write(repo.join("a"), "scanned\n").unwrap();
        std::fs::remove_file(repo.join("gone")).unwrap();
        std::fs::create_dir(repo.join("new")).unwrap();
        std::fs::write(repo.join("new/b"), "b\n").unwrap();
        std::fs::write(repo.join("other"), "o\n").unwrap();
        run(repo, &["add", "--all"]).unwrap();
        std::fs::write(repo.join("a"), "edited\n").unwrap();
        let commit = commit_staged(repo, "scoped", &["a", "gone", "new"]).unwrap();
        assert!(matches!(commit, Commit::Created));
        assert_eq!(run(repo, &["show", "HEAD:a"]).unwrap(), "scanned\n");
        assert_eq!(run(repo, &["show", "HEAD:new/b"]).unwrap(), "b\n");
        assert_eq!(run(repo, &["ls-tree", "HEAD", "gone"]).unwrap(), "");
        assert_eq!(
            run(repo, &["diff", "--cached", "--name-only"]).unwrap(),
            "other\n"
        );
        assert_eq!(run(repo, &["diff", "--name-only"]).unwrap(), "a\n");
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

    /// A commit made mid-rebase is dropped by `rebase --abort`; one on a
    /// detached HEAD is on no branch.
    #[test]
    fn commits_refuse_a_rebase_or_a_detached_head() {
        isolate();
        let (_dir, [a, _]) = remote_and_clones();
        for text in ["one", "two"] {
            std::fs::write(a.join("file"), text).unwrap();
            assert!(commit_all(&a, text).unwrap());
        }
        let refused = |expected: &str| {
            std::fs::write(a.join("new"), "text").unwrap();
            let err = commit(&a, "edit", &[])
                .err()
                .expect("commit should be refused");
            assert!(err.to_string().contains(expected), "{err}");
            assert_eq!(run(&a, &["status", "--porcelain"]).unwrap(), "?? new\n");
        };
        assert!(run(&a, &["rebase", "--quiet", "--exec", "false", "HEAD~1"]).is_err());
        refused("a rebase is in progress");
        run(&a, &["rebase", "--abort"]).unwrap();
        run(&a, &["checkout", "--quiet", "--detach"]).unwrap();
        refused("the repo is not on a branch");
    }

    #[test]
    fn any_failed_commit_unstages_and_says_why() {
        isolate();
        let (_dir, [a, _]) = remote_and_clones();
        let hook = a.join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\necho stopped by hook\nexit 1\n").unwrap();
        let mut permissions = std::fs::metadata(&hook).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
        std::fs::set_permissions(&hook, permissions).unwrap();
        std::fs::write(a.join("file"), "text").unwrap();
        let err = commit(&a, "edit", &["file"])
            .err()
            .expect("the hook refuses");
        assert!(err.to_string().contains("stopped by hook"), "{err}");
        assert_eq!(run(&a, &["status", "--porcelain"]).unwrap(), "?? file\n");

        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        let err = commit(&a, "edit", &[]).err().expect("the hook refuses");
        assert!(err.to_string().contains("exit status: 1"), "{err}");
        assert_eq!(run(&a, &["status", "--porcelain"]).unwrap(), "?? file\n");
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
            // SSH signing names no program.
            (
                "error: Couldn't load public key /k: No such file or directory?\n\nfatal: failed to write commit object",
                Failure::Signing,
            ),
            (
                "error: unable to create temporary file: No space left on device\nfatal: failed to write commit object",
                Failure::Other,
            ),
            (
                "error: insufficient permission for adding an object to repository database .git/objects\nfatal: failed to write commit object",
                Failure::Other,
            ),
        ];
        for (message, expected) in cases {
            assert_eq!(classify(message), expected, "{message}");
        }
    }
}
