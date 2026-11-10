//! publish, pull and sync between two hosts that share a bare remote.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Home, git_env};
use predicates::prelude::*;

/// A bare remote, host `a` that created the repo and host `b` that joined it.
struct Hosts {
    _remote: tempfile::TempDir,
    bare: PathBuf,
    a: Home,
    b: Home,
}

fn hosts() -> Hosts {
    let remote = tempfile::tempdir().unwrap();
    let bare = remote.path().join("dotfiles.git");
    git(
        remote.path(),
        &["init", "--quiet", "--bare", "dotfiles.git"],
    );
    let a = Home::new();
    a.write_host("a");
    run(&a, &["init"]);
    git(
        &a.repo(),
        &["remote", "add", "origin", bare.to_str().unwrap()],
    );
    run(&a, &["publish"]);
    let b = Home::new();
    b.write_host("b");
    run(&b, &["init", bare.to_str().unwrap()]);
    Hosts {
        _remote: remote,
        bare,
        a,
        b,
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let mut cmd = assert_cmd::Command::new("git");
    git_env(&mut cmd);
    let out = cmd.arg("-C").arg(dir).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn run(home: &Home, args: &[&str]) -> String {
    let out = home.cmd("rsmt").args(args).output().unwrap();
    assert!(
        out.status.success(),
        "rsmt {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// Commit subjects on the remote, newest first.
fn remote_log(hosts: &Hosts) -> Vec<String> {
    git(&hosts.bare, &["log", "--format=%s", "HEAD"])
        .lines()
        .map(str::to_string)
        .collect()
}

fn state(home: &Home) -> serde_json::Value {
    let text = fs::read_to_string(home.root.join(".local/state/rosamta/sync.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn set_host(home: &Home, line: &str) {
    let path = home.host_config();
    let text = fs::read_to_string(&path).unwrap();
    fs::write(path, format!("{text}{line}\n")).unwrap();
}

/// Like an installer adding a line to a hooked startup file.
fn append(home: &Home, rel: &str, text: &str) {
    let path = home.root.join(rel);
    let old = fs::read_to_string(&path).unwrap_or_default();
    fs::write(path, old + text).unwrap();
}

fn current(home: &Home) -> PathBuf {
    fs::read_link(home.root.join(".local/share/rosamta/generated/current")).unwrap()
}

#[test]
fn joining_publishes_the_new_host() {
    let hosts = hosts();
    assert_eq!(remote_log(&hosts)[0], "chore(repo): add host b");
    assert_eq!(state(&hosts.b)["publish"]["outcome"], "ok");
    assert_eq!(state(&hosts.b)["unpushed"], 0);
}

#[test]
fn publish_commits_hand_edits_and_pushes() {
    let hosts = hosts();
    hosts
        .a
        .write("modules/zoxide.toml", "[env]\n_ZO_ECHO = \"1\"\n");
    let out = run(&hosts.a, &["publish"]);
    assert!(out.contains("Published 1 commit"), "{out}");
    assert_eq!(remote_log(&hosts)[0], "edit(modules): add zoxide");
    assert_eq!(run(&hosts.a, &["publish"]), "Nothing to publish\n");
}

#[test]
fn a_secret_stops_publish_before_anything_leaves() {
    let hosts = hosts();
    let before = remote_log(&hosts);
    hosts.a.write(
        "modules/env.toml",
        "[env]\nGH = \"ghp_0123456789abcdefghijABCDEFGHIJ012345\"\n",
    );
    hosts
        .a
        .cmd("rsmt")
        .arg("publish")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("modules/env.toml:2 (github-pat)"));
    assert_eq!(remote_log(&hosts), before);
    // Not committed either: removing the line is all it takes.
    assert_eq!(
        git(&hosts.a.repo(), &["status", "--porcelain"]),
        "?? modules/\n"
    );
    let blocked = state(&hosts.a);
    assert_eq!(blocked["publish"]["outcome"], "blocked");
    assert_eq!(blocked["publish"]["files"][0], "modules/env.toml");
    assert_eq!(blocked["unpushed"], 0);

    // One already committed is caught too, before the push.
    git(&hosts.a.repo(), &["add", "--all"]);
    git(&hosts.a.repo(), &["commit", "--quiet", "-m", "env"]);
    hosts
        .a
        .cmd("rsmt")
        .arg("publish")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("modules/env.toml:2 (github-pat)"));
    assert_eq!(remote_log(&hosts), before);
    assert_eq!(state(&hosts.a)["unpushed"], 1);
}

#[test]
fn pull_brings_in_and_applies_another_hosts_module() {
    let hosts = hosts();
    run(&hosts.a, &["apply"]);
    hosts
        .b
        .write("modules/greeting.toml", "[env]\nGREETING = \"hi\"\n");
    run(&hosts.b, &["publish"]);

    let out = run(&hosts.a, &["pull"]);
    assert!(out.contains("Pulled 2 commits"), "{out}");
    assert!(hosts.a.repo().join("modules/greeting.toml").is_file());
    let shell = hosts.a.shell(&[
        "sh".into(),
        "-l".into(),
        "-c".into(),
        "printf %s \"$GREETING\"".into(),
    ]);
    assert_eq!(String::from_utf8_lossy(&shell.stdout), "hi");
    assert_eq!(run(&hosts.a, &["pull"]), "Already up to date\n");
}

#[test]
fn a_conflict_leaves_the_repo_as_it_was() {
    let hosts = hosts();
    run(&hosts.a, &["pull"]);
    hosts
        .a
        .write("modules/editor.toml", "[env]\nEDITOR = \"vim\"\n");
    run(&hosts.a, &["publish"]);
    run(&hosts.b, &["pull"]);
    hosts
        .a
        .write("modules/editor.toml", "[env]\nEDITOR = \"nvim\"\n");
    run(&hosts.a, &["publish"]);
    hosts
        .b
        .write("modules/editor.toml", "[env]\nEDITOR = \"hx\"\n");
    git(&hosts.b.repo(), &["commit", "--quiet", "--all", "-m", "hx"]);
    let head = git(&hosts.b.repo(), &["rev-parse", "HEAD"]);

    hosts
        .b
        .cmd("rsmt")
        .arg("pull")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("modules/editor.toml"));
    assert_eq!(git(&hosts.b.repo(), &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&hosts.b.repo(), &["status", "--porcelain"]), "");
    assert!(!hosts.b.repo().join(".git/rebase-merge").exists());
    assert_eq!(state(&hosts.b)["pull"]["outcome"], "conflict");
}

#[test]
fn a_pulled_module_that_breaks_startup_is_not_applied() {
    let hosts = hosts();
    run(&hosts.a, &["apply"]);
    let before = current(&hosts.a);
    let noisy = "echo noisy-startup >&2";
    hosts.b.write(
        "modules/noisy.toml",
        &format!("[raw]\nsh = '{noisy}'\nbash = '{noisy}'\nzsh = '{noisy}'\nfish = '{noisy}'\n"),
    );
    run(&hosts.b, &["publish"]);

    hosts
        .a
        .cmd("rsmt")
        .arg("pull")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("module noisy"));
    assert!(hosts.a.repo().join("modules/noisy.toml").is_file());
    assert_eq!(current(&hosts.a), before);
    assert_eq!(state(&hosts.a)["pull"]["outcome"], "failed");
}

#[test]
fn sync_goes_both_ways_and_retries_a_rejected_push() {
    let hosts = hosts();
    hosts.a.write("modules/a.toml", "[env]\nA = \"1\"\n");
    hosts.b.write("modules/b.toml", "[env]\nB = \"1\"\n");
    run(&hosts.b, &["publish"]);

    // Rejected first: b pushed meanwhile.
    let out = run(&hosts.a, &["publish"]);
    assert!(out.contains("Published 1 commit"), "{out}");
    run(&hosts.b, &["sync"]);
    for home in [&hosts.a, &hosts.b] {
        assert!(home.repo().join("modules/a.toml").is_file());
        assert!(home.repo().join("modules/b.toml").is_file());
    }
    assert_eq!(
        git(&hosts.a.repo(), &["rev-parse", "HEAD"]),
        git(&hosts.bare, &["rev-parse", "HEAD"])
    );
}

/// Adoption commits, then pushes only when the publish policy is `auto`.
#[test]
fn publish_policy_decides_whether_adoption_pushes() {
    let hosts = hosts();
    run(&hosts.a, &["apply"]);
    set_host(&hosts.a, "publish = \"manual\"");
    append(&hosts.a, ".profile", "export ONE=1\n");
    let before = remote_log(&hosts);
    run(&hosts.a, &["adopt", "--yes"]);
    assert_eq!(remote_log(&hosts), before);
    assert_eq!(state(&hosts.a)["unpushed"], 1);

    let host = fs::read_to_string(hosts.a.host_config()).unwrap();
    fs::write(hosts.a.host_config(), host.replace("manual", "auto")).unwrap();
    append(&hosts.a, ".profile", "export TWO=2\n");
    run(&hosts.a, &["adopt", "--yes"]);
    assert!(remote_log(&hosts)[0].starts_with("adopt(sh)"));
    assert_eq!(state(&hosts.a)["unpushed"], 0);
}

#[test]
fn offline_commits_wait_for_the_next_sync() {
    let hosts = hosts();
    run(&hosts.a, &["apply"]);
    let moved = hosts.bare.with_extension("moved");
    fs::rename(&hosts.bare, &moved).unwrap();
    append(&hosts.a, ".profile", "export ONE=1\n");
    hosts
        .a
        .cmd("rsmt")
        .args(["adopt", "--yes"])
        .assert()
        .success()
        .stderr(predicate::str::contains("will be pushed on the next sync"));
    let state_now = state(&hosts.a);
    assert_eq!(state_now["publish"]["outcome"], "offline");
    assert_eq!(state_now["unpushed"], 1);

    fs::rename(&moved, &hosts.bare).unwrap();
    run(&hosts.a, &["sync"]);
    assert!(remote_log(&hosts)[0].starts_with("adopt(sh)"));
    assert_eq!(state(&hosts.a)["unpushed"], 0);
}

#[test]
fn a_commit_that_cannot_be_signed_is_not_made() {
    let hosts = hosts();
    for (key, value) in [
        ("commit.gpgsign", "true"),
        ("user.signingkey", "0000000000000000"),
        ("gpg.program", "false"),
    ] {
        git(&hosts.a.repo(), &["config", key, value]);
    }
    let head = git(&hosts.a.repo(), &["rev-parse", "HEAD"]);
    hosts
        .a
        .write("modules/zoxide.toml", "[env]\n_ZO_ECHO = \"1\"\n");
    hosts
        .a
        .cmd("rsmt")
        .arg("publish")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("commit signing failed"));
    assert_eq!(git(&hosts.a.repo(), &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(&hosts.a.repo(), &["status", "--porcelain"]),
        "?? modules/\n"
    );
    let state = state(&hosts.a);
    assert_eq!(state["publish"]["outcome"], "blocked");
    assert!(
        state["publish"]["reason"]
            .as_str()
            .unwrap()
            .contains("commit signing failed")
    );
}

#[test]
fn a_commit_a_hook_stops_is_not_made_and_says_why() {
    let hosts = hosts();
    let hook = hosts.a.repo().join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\necho stopped by hook\nexit 1\n").unwrap();
    let mut permissions = fs::metadata(&hook).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    fs::set_permissions(&hook, permissions).unwrap();
    hosts
        .a
        .write("modules/zoxide.toml", "[env]\n_ZO_ECHO = \"1\"\n");
    hosts
        .a
        .cmd("rsmt")
        .arg("publish")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("stopped by hook"));
    assert_eq!(
        git(&hosts.a.repo(), &["status", "--porcelain"]),
        "?? modules/\n"
    );
    let state = state(&hosts.a);
    assert_eq!(state["publish"]["outcome"], "failed");
    assert!(
        state["publish"]["reason"]
            .as_str()
            .unwrap()
            .contains("stopped by hook")
    );
}

/// What was scanned is what gets committed, even when the file changes
/// right after the scan.
#[test]
fn an_edit_after_the_scan_is_not_committed() {
    let hosts = hosts();
    run(&hosts.a, &["pull"]);
    let repo = hosts.a.repo();
    // The file as it is until it is staged; then as if a secret was written
    // to it, with a new mtime so git reads it again.
    let filter = hosts.a.root.join("filter.sh");
    fs::write(
        &filter,
        "if grep -qa modules/zoxide.toml .git/index; then\n\
         cat > /dev/null\n\
         printf 'GH = \"ghp_0123456789abcdefghijABCDEFGHIJ012345\"\\n'\n\
         else\n\
         touch -t 203001010000 \"$1\"\n\
         cat\n\
         fi\n",
    )
    .unwrap();
    git(
        &repo,
        &[
            "config",
            "filter.late.clean",
            &format!("sh '{}' %f", filter.display()),
        ],
    );
    fs::write(
        repo.join(".git/info/attributes"),
        "modules/zoxide.toml filter=late\n",
    )
    .unwrap();
    hosts.a.write("modules/zoxide.toml", "[env]\n");
    run(&hosts.a, &["publish"]);
    let committed = git(&repo, &["show", "HEAD:modules/zoxide.toml"]);
    assert_eq!(committed, "[env]\n");
    assert_eq!(
        git(&repo, &["status", "--porcelain"]),
        " M modules/zoxide.toml\n"
    );
}

#[test]
fn without_a_remote_publish_is_a_usage_error() {
    let home = Home::new();
    run(&home, &["init"]);
    home.cmd("rsmt")
        .arg("publish")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("the repo has no remote"));
}

#[test]
fn automatic_push_leaves_hand_edits_uncommitted() {
    let hosts = hosts();
    run(&hosts.a, &["apply"]);
    run(&hosts.a, &["pull"]);
    hosts
        .a
        .write("modules/zoxide.toml", "[env]\n_ZO_ECHO = \"1\"\n");
    append(&hosts.a, ".profile", "export ONE=1\n");
    run(&hosts.a, &["adopt", "--yes"]);
    assert!(remote_log(&hosts)[0].starts_with("adopt(sh)"));
    assert_eq!(
        git(&hosts.a.repo(), &["status", "--porcelain"]),
        "?? modules/zoxide.toml\n"
    );
}

#[test]
fn a_host_that_never_applied_pulls_without_touching_rc_files() {
    let hosts = hosts();
    fs::write(hosts.b.root.join(".profile"), "export MINE=1\n").unwrap();
    hosts
        .a
        .write("modules/greeting.toml", "[env]\nGREETING = \"hi\"\n");
    run(&hosts.a, &["publish"]);

    assert!(run(&hosts.b, &["pull"]).contains("Pulled"));
    assert!(hosts.b.repo().join("modules/greeting.toml").is_file());
    assert_eq!(
        fs::read_to_string(hosts.b.root.join(".profile")).unwrap(),
        "export MINE=1\n"
    );
    assert!(!hosts.b.root.join(".local/share/rosamta/generated").exists());
}
