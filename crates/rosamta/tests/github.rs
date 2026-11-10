//! GitHub sign-out and the credential helper, without reaching GitHub.

mod common;

use std::fs;

use common::{Home, git, git_env};
use predicates::prelude::*;

const HELPER_KEY: &str = "credential.https://github.com.helper";

fn token_file(home: &Home) -> std::path::PathBuf {
    home.root.join(".config/rosamta/github-token")
}

fn signed_in(home: &Home) {
    fs::create_dir_all(token_file(home).parent().unwrap()).unwrap();
    fs::write(token_file(home), "gho_test\n").unwrap();
}

#[test]
fn the_helper_hands_git_the_token_for_github_only() {
    let home = Home::new();
    signed_in(&home);
    home.cmd("rosamta")
        .args(["git-credential", "get"])
        .write_stdin("protocol=https\nhost=github.com\n\n")
        .assert()
        .success()
        .stdout("username=x-access-token\npassword=gho_test\n");
    home.cmd("rosamta")
        .args(["git-credential", "get"])
        .write_stdin("protocol=https\nhost=example.com\n\n")
        .assert()
        .success()
        .stdout("");
    home.cmd("rosamta")
        .args(["git-credential", "store"])
        .write_stdin("protocol=https\nhost=github.com\npassword=x\n\n")
        .assert()
        .success()
        .stdout("");
}

#[test]
fn logout_removes_the_token_and_the_repo_helper() {
    let home = Home::new();
    signed_in(&home);
    home.cmd("rsmt").arg("init").assert().success();
    git(&home.repo(), &["config", "--add", HELPER_KEY, ""]);
    git(
        &home.repo(),
        &[
            "config",
            "--add",
            HELPER_KEY,
            "!'/usr/bin/rosamta' git-credential",
        ],
    );

    home.cmd("rsmt")
        .args(["remote", "github", "logout"])
        .assert()
        .success()
        .stdout("Signed out of GitHub\n");
    assert!(!token_file(&home).exists());
    let mut query = assert_cmd::Command::new("git");
    git_env(&mut query);
    let left = query
        .arg("-C")
        .arg(home.repo())
        .args(["config", "--local", "--get-all", HELPER_KEY])
        .output()
        .unwrap();
    assert!(left.stdout.is_empty(), "{left:?}");
    home.cmd("rosamta")
        .args(["git-credential", "get"])
        .write_stdin("protocol=https\nhost=github.com\n\n")
        .assert()
        .success()
        .stdout("");
    home.cmd("rsmt")
        .args(["remote", "github", "logout"])
        .assert()
        .success()
        .stdout("Not signed in to GitHub\n");
}

#[test]
fn creating_a_github_repo_needs_sign_in() {
    let home = Home::new();
    home.cmd("rsmt")
        .args(["init", "--github", "dotfiles"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "not signed in to GitHub; run `rsmt remote github login` first",
        ));
    assert!(!home.repo().exists());
    home.cmd("rsmt")
        .args(["init", "--github", "a/b/c"])
        .assert()
        .code(2);
}
