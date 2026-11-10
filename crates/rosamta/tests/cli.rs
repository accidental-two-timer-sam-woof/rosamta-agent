//! The CLI under both names, against a throwaway HOME.

mod common;

use std::fs;

use common::{Home, git};
use predicates::prelude::*;

#[test]
fn help_uses_the_typed_name() {
    let home = Home::new();
    for bin in ["rosamta", "rsmt"] {
        home.cmd(bin).arg("--help").assert().success().stdout(
            predicate::str::starts_with("Cross-shell")
                .and(predicate::str::contains(format!("Usage: {bin} "))),
        );
    }
}

#[test]
fn errors_use_the_typed_name() {
    let home = Home::new();
    home.cmd("rsmt")
        .arg("validate")
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("rsmt: no repo at"));
}

#[test]
fn both_names_behave_the_same() {
    let home = Home::new();
    home.write("rosamta.toml", "format = 1\nhosts = [\"mbp\"]\n");
    home.write("modules/x.toml", "[when]\nhosts = [\"new-laptop\"]\n");
    let a = home.cmd("rosamta").arg("validate").output().unwrap();
    let b = home.cmd("rsmt").arg("validate").output().unwrap();
    assert_eq!(
        (a.status.code(), &a.stdout, &a.stderr),
        (b.status.code(), &b.stdout, &b.stderr)
    );
}

#[test]
fn invalid_module_exits_2_with_location() {
    let home = Home::new();
    home.write("rosamta.toml", "format = 1\n");
    home.write("modules/bun.toml", "enabled = true\nphse = \"tools\"\n");
    let path = home.repo().join("modules/bun.toml");
    home.cmd("rsmt").arg("validate").assert().code(2).stderr(
        predicate::str::contains(format!("{}:2:", path.display()))
            .and(predicate::str::contains("phse")),
    );
}

#[test]
fn nested_dotfiles_exit_2() {
    let home = Home::new();
    home.write("rosamta.toml", "format = 1\n");
    home.write("home/.config/nvim/init.lua", "");
    home.write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".config/nvim\"\n\n[[dotfile]]\npath = \".config/nvim/init.lua\"\n",
    );
    home.cmd("rsmt")
        .arg("validate")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "dotfile `.config/nvim/init.lua` is inside dotfile `.config/nvim`",
        ));
}

#[test]
fn json_output_is_one_document() {
    let home = Home::new();
    home.write("rosamta.toml", "format = 1\n");
    home.write("modules/bun.toml", "phse = 1\n");
    let out = home
        .cmd("rsmt")
        .args(["validate", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["valid"], false);
    let module = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "modules/bun.toml")
        .unwrap();
    assert_eq!(module["errors"][0]["line"], 1);
}

#[test]
fn unknown_host_warns_and_exits_0() {
    let home = Home::new();
    home.write("rosamta.toml", "format = 1\nhosts = [\"mbp\"]\n");
    home.write("modules/x.toml", "[when]\nhosts = [\"new-laptop\"]\n");
    home.cmd("rsmt")
        .arg("validate")
        .assert()
        .success()
        .stderr(predicate::str::contains("warning").and(predicate::str::contains("new-laptop")));
}

#[test]
fn unreadable_repo_exits_1() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    home.write("rosamta.toml", "format = 1\n");
    home.write("modules/a.toml", "");
    let modules = home.repo().join("modules");
    fs::set_permissions(&modules, fs::Permissions::from_mode(0o000)).unwrap();
    let readable_anyway = fs::read_dir(&modules).is_ok(); // running as root
    let assert = home.cmd("rsmt").arg("validate").assert();
    fs::set_permissions(&modules, fs::Permissions::from_mode(0o755)).unwrap();
    if !readable_anyway {
        assert
            .code(1)
            .stderr(predicate::str::contains(modules.display().to_string()));
    }
}

#[test]
fn unreadable_repo_dir_exits_1() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    home.write("rosamta.toml", "format = 1\n");
    let repo = home.repo();
    fs::set_permissions(&repo, fs::Permissions::from_mode(0o000)).unwrap();
    let readable_anyway = fs::read_dir(&repo).is_ok(); // running as root
    let assert = home.cmd("rsmt").arg("validate").assert();
    fs::set_permissions(&repo, fs::Permissions::from_mode(0o755)).unwrap();
    if !readable_anyway {
        assert
            .code(1)
            .stderr(predicate::str::contains(repo.display().to_string()));
    }
}

#[test]
fn apply_refuses_duplicate_module_names() {
    let local = ".config/rosamta/local/modules/bun.toml";
    for (first, second) in [
        (".local/share/rosamta/repo/modules/bun.toml", local),
        (
            ".local/share/rosamta/repo/modules/adopted/bun.toml",
            ".local/share/rosamta/repo/modules/bun.toml",
        ),
    ] {
        let home = Home::new();
        home.write("rosamta.toml", "format = 1\n");
        home.write_host("mbp");
        for rel in [first, second] {
            let path = home.root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        home.cmd("rsmt").arg("apply").assert().code(2).stderr(
            predicate::str::contains(home.root.join(first).display().to_string()).and(
                predicate::str::contains(home.root.join(second).display().to_string()),
            ),
        );
        assert!(!home.root.join(".local/share/rosamta/generated").exists());
    }
}

#[test]
fn newer_format_is_refused() {
    let home = Home::new();
    home.write("rosamta.toml", "format = 99\n");
    home.cmd("rsmt")
        .arg("validate")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("upgrade rosamta"));
}

#[test]
fn messages_stay_english_under_zh_tw() {
    let home = Home::new();
    home.write("rosamta.toml", "format = 1\n");
    home.write("modules/x.toml", "[when]\nos = [\"darwin\"]\n");
    home.cmd("rsmt")
        .env("LANG", "zh_TW.UTF-8")
        .env("LC_ALL", "zh_TW.UTF-8")
        .arg("validate")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "error: unknown variant `darwin`, expected `macos` or `linux`",
        ));
}

#[test]
fn schema_prints_module_schema() {
    let home = Home::new();
    let out = home
        .cmd("rsmt")
        .args(["schema", "module"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let schema: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(schema["properties"]["phase"].is_object());
    assert_eq!(schema["additionalProperties"], false);
}

#[test]
fn init_creates_repo_without_touching_rc_files() {
    let home = Home::new();
    fs::write(home.root.join(".zshrc"), "export A=1\n").unwrap();
    fs::write(home.root.join(".bashrc"), "alias ll='ls -l'\n").unwrap();
    let before = home.snapshot(&home.root.join(".local"));

    home.cmd("rsmt")
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Created repo at"));
    assert!(home.repo().join("rosamta.toml").is_file());
    assert!(home.host_config().is_file());
    let after: Vec<_> = home
        .snapshot(&home.root.join(".local"))
        .into_iter()
        .filter(|(path, _)| !path.starts_with(home.root.join(".config")))
        .collect();
    assert_eq!(after, before);
    home.cmd("rsmt").arg("validate").assert().success();
}

#[test]
fn init_twice_exits_2_and_changes_nothing() {
    let home = Home::new();
    home.cmd("rsmt").arg("init").assert().success();
    let before = home.snapshot(&home.repo().join(".git"));
    home.cmd("rsmt")
        .arg("init")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("already exists"));
    assert_eq!(home.snapshot(&home.repo().join(".git")), before);
}

#[test]
fn init_with_url_clones_and_registers_host() {
    let remote = tempfile::tempdir().unwrap();
    let work = remote.path().join("work");
    fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "--quiet"]);
    fs::write(work.join("rosamta.toml"), "format = 1\nhosts = [\"mbp\"]\n").unwrap();
    git(&work, &["add", "--all"]);
    git(&work, &["commit", "--quiet", "--message", "seed"]);
    let bare = remote.path().join("remote.git");
    git(
        remote.path(),
        &["clone", "--quiet", "--bare", "work", "remote.git"],
    );

    let home = Home::new();
    home.write_host("lab-server");
    fs::write(home.root.join(".zshrc"), "export A=1\n").unwrap();
    let out = home
        .cmd("rsmt")
        .args(["init", bare.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        (doc["cloned"].as_bool(), doc["host_id"].as_str()),
        (Some(true), Some("lab-server"))
    );
    let manifest = fs::read_to_string(home.repo().join("rosamta.toml")).unwrap();
    assert!(
        manifest.contains("\"mbp\"") && manifest.contains("\"lab-server\""),
        "{manifest}"
    );
    assert_eq!(
        fs::read_to_string(home.root.join(".zshrc")).unwrap(),
        "export A=1\n"
    );
}
