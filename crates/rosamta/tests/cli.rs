//! The CLI under both names, against a throwaway HOME.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;

struct Home {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        Self { _dir: dir, root }
    }

    fn repo(&self) -> PathBuf {
        self.root.join(".local/share/rosamta/repo")
    }

    fn host_config(&self) -> PathBuf {
        self.root.join(".config/rosamta/host.toml")
    }

    fn cmd(&self, bin: &str) -> Command {
        let mut cmd = Command::cargo_bin(bin).unwrap();
        cmd.env("HOME", &self.root);
        for var in [
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "LANG",
            "LC_ALL",
        ] {
            cmd.env_remove(var);
        }
        git_env(&mut cmd);
        cmd
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.repo().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn write_host(&self, id: &str) {
        fs::create_dir_all(self.host_config().parent().unwrap()).unwrap();
        fs::write(self.host_config(), format!("id = \"{id}\"\n")).unwrap();
    }

    /// Every file under HOME with its content, for "nothing else changed" checks.
    fn snapshot(&self, skip: &Path) -> Vec<(PathBuf, Vec<u8>)> {
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
fn git_env(cmd: &mut Command) {
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com");
}

fn git(dir: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    git_env(&mut cmd);
    cmd.arg("-C").arg(dir).args(args).assert().success();
}

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
