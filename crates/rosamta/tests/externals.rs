//! `externals.toml`: checks, and downloads from local git repos and a
//! local HTTP server.

mod common;

use common::Home;
use predicates::prelude::*;

#[test]
fn schema_describes_externals_toml() {
    let home = Home::new();
    let out = home
        .cmd("rsmt")
        .args(["schema", "externals"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let schema: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(schema["properties"]["external"].is_object());
}

#[test]
fn validate_reports_unlocked_and_nested_externals() {
    let home = Home::new();
    home.write("rosamta.toml", "format = 1\n");
    home.write("dotfiles.toml", "[[dotfile]]\npath = \".config/zsh\"\n");
    home.write("home/.config/zsh/aliases.zsh", "alias l=ls\n");
    home.write(
        "externals.toml",
        "[[external]]\npath = \".config/zsh/plugins/a\"\ngit = \"https://example.com/a.git\"\n",
    );
    home.cmd("rsmt").arg("validate").assert().code(2).stderr(
        predicate::str::contains("external `.config/zsh/plugins/a` is not locked yet")
            .and(predicate::str::contains("is inside dotfile `.config/zsh`")),
    );
}
