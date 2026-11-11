//! `rsmt import chezmoi`. With chezmoi installed it runs inside this HOME.

mod common;

use std::fs;
use std::path::Path;

use common::Home;
use predicates::prelude::*;

/// A source where chezmoi keeps it by default.
fn source(home: &Home) {
    let dir = home.root.join(".local/share/chezmoi");
    for (rel, text) in [
        ("dot_gitconfig", "[user]\n\tname = me\n"),
        ("private_dot_netrc", "machine example.com\n"),
        ("dot_bad.tmpl", "{{ range .list }}x{{ end }}\n"),
        ("README.md", "notes\n"),
        (".chezmoiignore", "README.md\n"),
    ] {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

fn head(repo: &Path) -> String {
    let mut cmd = assert_cmd::Command::new("git");
    common::git_env(&mut cmd);
    let out = cmd
        .arg("-C")
        .arg(repo)
        .args(["log", "-1", "--format=%s"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn importing_before_init_writes_nothing() {
    let home = Home::new();
    source(&home);
    home.cmd("rsmt")
        .args(["import", "chezmoi"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("run `rsmt init` first"));
    assert!(!home.repo().exists());
}

#[test]
fn a_missing_source_is_a_usage_error() {
    let home = Home::new();
    home.cmd("rsmt").arg("init").assert().success();
    let missing = home.root.join("nowhere");
    home.cmd("rsmt")
        .args(["import", "chezmoi"])
        .arg(&missing)
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "is not a chezmoi source directory",
        ));
}

#[test]
fn import_reports_each_item_and_commits() {
    let home = Home::new();
    home.cmd("rsmt").arg("init").assert().success();
    source(&home);
    let before = head(&home.repo());
    let host = fs::read_to_string(home.host_config()).unwrap();

    let lines = [
        "Imported ~/.gitconfig",
        "Imported ~/.netrc",
        "Not imported dot_bad.tmpl: unsupported template (line 1: `range`)",
        "Skipped README.md: excluded by .chezmoiignore",
        "Imported 2, skipped 1, not imported 1",
    ];
    let dry = home
        .cmd("rsmt")
        .args(["import", "chezmoi", "--dry-run"])
        .output()
        .unwrap();
    assert!(dry.status.success());
    let shown = String::from_utf8(dry.stdout).unwrap();
    for line in lines {
        assert!(shown.contains(line), "{line}\n{shown}");
    }
    assert_eq!(head(&home.repo()), before, "a dry run commits nothing");
    assert!(!home.repo().join("home").exists());
    assert_eq!(fs::read_to_string(home.host_config()).unwrap(), host);

    let out = home
        .cmd("rsmt")
        .args(["--json", "import", "chezmoi", "--dry-run"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        report["items"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!({
                "source": "dot_bad.tmpl",
                "target": ".bad",
                "outcome": "manual",
                "reason": "template",
                "detail": "line 1: `range`",
            })),
        "{report}"
    );

    let real = home
        .cmd("rsmt")
        .args(["import", "chezmoi"])
        .output()
        .unwrap();
    assert!(real.status.success());
    assert_eq!(String::from_utf8(real.stdout).unwrap(), shown);
    assert_eq!(
        head(&home.repo()),
        "import(chezmoi): 2 dotfiles, 0 scripts, 0 externals\n"
    );
    assert_eq!(
        fs::read_to_string(home.repo().join("chezmoi-import/dot_bad.tmpl")).unwrap(),
        "{{ range .list }}x{{ end }}\n"
    );
}
