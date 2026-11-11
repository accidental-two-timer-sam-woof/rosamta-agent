//! Scripts run by `rsmt apply` and from the review queue. Tests have no
//! terminal, so they run what `scripts = "auto"` or the queue runs.

mod common;

use std::fs;

use common::Home;
use predicates::prelude::*;

fn set_up(policy: &str) -> Home {
    let home = Home::new();
    home.cmd("rsmt").arg("init").assert().success();
    let host = home.host_config();
    let text = fs::read_to_string(&host).unwrap();
    assert!(text.contains("scripts = \"ask\"\n"), "{text}");
    let text = text.replace("scripts = \"ask\"", &format!("scripts = \"{policy}\""));
    fs::write(&host, text).unwrap();
    home
}

fn stdout(home: &Home, args: &[&str]) -> String {
    let out = home.cmd("rsmt").args(args).output().unwrap();
    assert!(
        out.status.success(),
        "rsmt {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn before_scripts_run_ahead_of_the_shell_check_and_after_scripts_last() {
    let home = set_up("auto");
    // Unguarded: startup fails until the before script makes the file.
    let source = r#". "$HOME/.tool/env.sh""#;
    home.write(
        "modules/tool.toml",
        &format!("[raw]\nsh = '{source}'\nbash = '{source}'\nzsh = '{source}'\n"),
    );
    home.write(
        "scripts.toml",
        "[[script]]\nsource = \"scripts/install.sh\"\nrun = \"once\"\ntiming = \"before\"\n\n\
         [[script]]\nsource = \"scripts/after.sh\"\nrun = \"onchange\"\n",
    );
    home.write(
        "scripts/install.sh",
        "mkdir -p \"$HOME/.tool\"\necho 'TOOL=1' > \"$HOME/.tool/env.sh\"\n",
    );
    home.write(
        "scripts/after.sh",
        "test -L \"$HOME/.npmrc\" && echo after > \"$HOME/after\"\n",
    );
    home.write("dotfiles.toml", "[[dotfile]]\npath = \".npmrc\"\n");
    home.write("home/.npmrc", "x\n");

    let dry = stdout(&home, &["apply", "--dry-run"]);
    assert!(
        dry.contains("Will run scripts/install.sh: never ran on this host"),
        "{dry}"
    );
    assert!(!home.root.join(".tool").exists());
    let out = stdout(&home, &["apply"]);
    assert!(out.contains("Ran scripts/install.sh"), "{out}");
    assert!(out.contains("Ran scripts/after.sh"), "{out}");
    assert_eq!(
        fs::read_to_string(home.root.join("after")).unwrap(),
        "after\n"
    );
    assert_eq!(stdout(&home, &["apply"]), "Already up to date\n");
}

#[test]
fn ask_queues_scripts_and_review_runs_or_skips_them() {
    let home = set_up("ask");
    home.write(
        "scripts.toml",
        "[[script]]\nsource = \"a.sh\"\nrun = \"once\"\n\n\
         [[script]]\nsource = \"b.sh\"\nrun = \"onchange\"\n",
    );
    home.write(
        "a.sh",
        "[ -f \"$HOME/fail\" ] && exit 4\necho a >> \"$HOME/ran\"\n",
    );
    home.write("b.sh", "echo b >> \"$HOME/ran\"\n");
    let out = stdout(&home, &["apply"]);
    assert!(
        out.contains("Queued a.sh for review: never ran on this host"),
        "{out}"
    );
    assert!(!home.root.join("ran").exists());
    let listed = stdout(&home, &["review"]);
    let ids: Vec<&str> = listed
        .lines()
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();
    assert!(listed.contains("a.sh (once)"), "{listed}");
    assert!(listed.contains("b.sh (onchange)"), "{listed}");
    let (a, b) = (ids[0], ids[1]);

    fs::write(home.root.join("fail"), "").unwrap();
    home.cmd("rsmt")
        .args(["review", "accept", a])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "error: script a.sh: exited with 4",
        ));
    assert!(stdout(&home, &["review"]).contains("failed last time"));
    fs::remove_file(home.root.join("fail")).unwrap();
    assert_eq!(stdout(&home, &["review", "accept", a]), "Ran a.sh\n");

    home.cmd("rsmt")
        .args(["review", "discard", b])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("counts a script as run"));
    // Accepting runs only the version that was listed.
    home.write("b.sh", "echo b2 >> \"$HOME/ran\"\n");
    home.cmd("rsmt")
        .args(["review", "accept", b])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("changed since it was listed"));
    stdout(&home, &["review"]);
    stdout(&home, &["review", "ignore", b]);
    assert_eq!(stdout(&home, &["review"]), "Nothing to review\n");
    assert_eq!(fs::read_to_string(home.root.join("ran")).unwrap(), "a\n");
}
#[test]
fn validate_and_schema_cover_scripts_toml() {
    let home = Home::new();
    home.write("rosamta.toml", "format = 1\n");
    home.write(
        "scripts.toml",
        "[[script]]\nsource = \"scripts/missing.sh\"\nrun = \"once\"\n",
    );
    home.cmd("rsmt")
        .arg("validate")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "script `scripts/missing.sh` does not exist",
        ));
    let out = home
        .cmd("rsmt")
        .args(["schema", "scripts"])
        .output()
        .unwrap();
    let schema: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(schema["properties"]["script"].is_object());
    let out = home.cmd("rsmt").args(["schema", "host"]).output().unwrap();
    let schema: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(schema["properties"]["scripts"].is_object());
}
