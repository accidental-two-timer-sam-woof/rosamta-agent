//! rsmt add and rsmt forget against a throwaway HOME.

mod common;

use std::fs;

use common::Home;
use predicates::prelude::*;

fn set_up() -> Home {
    let home = Home::new();
    home.cmd("rsmt").arg("init").assert().success();
    home
}

#[test]
fn add_reports_and_links() {
    let home = set_up();
    fs::create_dir_all(home.root.join(".config/ghostty")).unwrap();
    fs::write(home.root.join(".config/ghostty/config"), "theme = dark\n").unwrap();
    home.cmd("rsmt")
        .current_dir(home.root.join(".config"))
        .args(["add", "ghostty"])
        .assert()
        .success()
        .stdout("Added ~/.config/ghostty\n");
    assert!(
        fs::symlink_metadata(home.root.join(".config/ghostty"))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn encrypting_needs_a_terminal_for_the_passphrase() {
    let home = set_up();
    fs::write(home.root.join(".netrc"), "machine x\n").unwrap();
    home.cmd("rsmt")
        .args(["add", "--encrypt"])
        .arg(home.root.join(".netrc"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "set the passphrase for encrypted dotfiles in a terminal",
        ));
    assert!(!home.repo().join("home/.netrc").exists());
}
