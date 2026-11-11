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

/// A repo outside the repo dir with `plugin.zsh`, and its commit.
fn upstream(home: &Home) -> (std::path::PathBuf, String) {
    let dir = home.root.join("upstream");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plugin.zsh"), "export FROM_PLUGIN=1\n").unwrap();
    common::git(&dir, &["init", "--quiet"]);
    common::git(&dir, &["add", "-A"]);
    common::git(&dir, &["commit", "--quiet", "-m", "plugin"]);
    let mut cmd = assert_cmd::Command::new("git");
    common::git_env(&mut cmd);
    let out = cmd
        .arg("-C")
        .arg(&dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    (
        dir,
        String::from_utf8(out.stdout).unwrap().trim().to_string(),
    )
}

#[test]
fn externals_download_before_the_shells_are_checked() {
    let home = Home::new();
    home.cmd("rsmt").arg("init").assert().success();
    let (url, rev) = upstream(&home);
    // Unguarded, as a module loading oh-my-zsh would be.
    let source = r#". "$HOME/.oh-my-zsh/plugin.zsh""#;
    home.write(
        "modules/omz.toml",
        &format!("[raw]\nsh = '{source}'\nbash = '{source}'\nzsh = '{source}'\n"),
    );
    home.write(
        "externals.toml",
        &format!(
            "[[external]]\npath = \".oh-my-zsh\"\ngit = \"{}\"\nrev = \"{rev}\"\n",
            url.display()
        ),
    );
    home.write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".oh-my-zsh/custom/aliases.zsh\"\n",
    );
    home.write("home/.oh-my-zsh/custom/aliases.zsh", "alias l=ls\n");
    let omz = home.root.join(".oh-my-zsh");

    home.cmd("rsmt")
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Downloaded ~/.oh-my-zsh"));
    assert!(!omz.exists());
    let out = home.cmd("rsmt").args(["apply", "--json"]).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let applied: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(applied["externals"][0]["action"], "downloaded");
    assert_eq!(applied["dotfiles"][0]["action"], "linked");
    assert!(omz.join("plugin.zsh").is_file());
    assert!(omz.join("custom/aliases.zsh").is_symlink());
}

#[test]
fn something_else_at_the_path_is_left_alone() {
    let home = Home::new();
    home.cmd("rsmt").arg("init").assert().success();
    let (url, rev) = upstream(&home);
    std::fs::create_dir_all(home.root.join(".zplug")).unwrap();
    std::fs::write(home.root.join(".zplug/mine"), "x").unwrap();
    home.write(
        "externals.toml",
        &format!(
            "[[external]]\npath = \".zplug\"\ngit = \"{}\"\nrev = \"{rev}\"\n",
            url.display()
        ),
    );
    home.cmd("rsmt")
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Not downloaded ~/.zplug: something else is there",
        ));
    assert_eq!(
        std::fs::read_to_string(home.root.join(".zplug/mine")).unwrap(),
        "x"
    );
}

#[test]
fn update_locks_the_newest_commit_and_downloads_it() {
    let home = Home::new();
    home.cmd("rsmt").arg("init").assert().success();
    let (url, first) = upstream(&home);
    home.write(
        "externals.toml",
        &format!(
            "# plugins\n[[external]]\npath = \".zplug\"\ngit = \"{}\"\n",
            url.display()
        ),
    );
    common::git(&home.repo(), &["add", "-A"]);
    common::git(&home.repo(), &["commit", "--quiet", "-m", "externals"]);
    home.cmd("rsmt")
        .args(["externals", "update"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains(format!("Locked ~/.zplug to {}", &first[..7]))
                .and(predicate::str::contains("Downloaded ~/.zplug")),
        );
    let manifest = std::fs::read_to_string(home.repo().join("externals.toml")).unwrap();
    assert!(manifest.starts_with("# plugins\n"), "{manifest}");
    assert!(
        manifest.contains(&format!("rev = \"{first}\"")),
        "{manifest}"
    );
    let mut log = assert_cmd::Command::new("git");
    common::git_env(&mut log);
    let out = log
        .arg("-C")
        .arg(home.repo())
        .args(["log", "-1", "--format=%s"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "update(externals): .zplug\n"
    );

    home.cmd("rsmt")
        .args(["externals", "update", ".other"])
        .current_dir(&home.root)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("is not an external"));
    home.cmd("rsmt")
        .args(["externals", "update", ".zplug"])
        .current_dir(&home.root)
        .assert()
        .success()
        .stdout("Already up to date\n");
}

/// A lock committed by a run that could not download goes out once a later
/// run downloads it.
#[test]
fn a_lock_goes_out_once_a_retry_downloads_it() {
    let home = Home::new();
    home.cmd("rsmt").arg("init").assert().success();
    let remote = home.root.join("remote.git");
    common::git(&home.root, &["init", "--quiet", "--bare", "remote.git"]);
    common::git(
        &home.repo(),
        &["remote", "add", "origin", &remote.display().to_string()],
    );
    let (url, _) = upstream(&home);
    home.write(
        "externals.toml",
        &format!(
            "[[external]]\npath = \".zplug/plugins\"\ngit = \"{}\"\n",
            url.display()
        ),
    );
    common::git(&home.repo(), &["add", "-A"]);
    common::git(&home.repo(), &["commit", "--quiet", "-m", "externals"]);
    // A file where its folder goes stops the download, even for root.
    std::fs::write(home.root.join(".zplug"), "").unwrap();
    let failed = home
        .cmd("rsmt")
        .args(["externals", "update"])
        .output()
        .unwrap();
    std::fs::remove_file(home.root.join(".zplug")).unwrap();
    assert!(!failed.status.success());
    let pushed = || {
        let mut cmd = assert_cmd::Command::new("git");
        common::git_env(&mut cmd);
        let out = cmd
            .arg("-C")
            .arg(&remote)
            .args(["log", "--all", "--format=%s"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert!(!pushed().contains("update(externals)"));

    let retry = home
        .cmd("rsmt")
        .args(["externals", "update"])
        .output()
        .unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&retry.stdout),
        String::from_utf8_lossy(&retry.stderr)
    );
    assert!(retry.status.success(), "{said}");
    let log = pushed();
    assert!(
        log.contains("update(externals): .zplug/plugins"),
        "{said}\n{log}"
    );
}
