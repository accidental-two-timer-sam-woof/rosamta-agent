//! `rsmt recent` and `rsmt include`: what other hosts added, and adding
//! this host to it.

mod common;

use std::fs;

use assert_cmd::Command;
use common::{Home, git_env, has};
use predicates::prelude::*;

const MY_OS: &str = if cfg!(target_os = "macos") {
    "macos"
} else {
    "linux"
};
const OTHER_OS: &str = if cfg!(target_os = "macos") {
    "linux"
} else {
    "macos"
};

/// bun as host `mbp` on `os` adopted it from `.zshrc`.
fn bun(os: &str) -> String {
    format!(
        "order = 7001\nshells = [\"zsh\"]\n\n[when]\nhosts = [\"mbp\"]\nos = [\"{os}\"]\n\n[env]\nBUN_INSTALL = \"$HOME/.bun\"\n\n[path]\nprepend = [\"$BUN_INSTALL/bin\"]\n"
    )
}

const GHOSTTY: &str =
    "[[dotfile]]\npath = \".config/ghostty/config\"\nwhen = { hosts = [\"mbp\"] }\n";

/// A dotfile every host gets, and mbp's own version of it.
const GITCONFIG: &str = "[[dotfile]]\npath = \".gitconfig\"\nwhen = { hosts = [\"test\"] }\n\n[[dotfile]]\npath = \".gitconfig\"\nsource = \"hosts/mbp/.gitconfig\"\nwhen = { hosts = [\"mbp\"] }\n";

/// A HOME with a repo, host `test` and hooks for every installed shell.
fn host() -> Home {
    let home = Home::new();
    home.write_host("test");
    home.cmd("rsmt").arg("init").assert().success();
    run(&home, &["apply"]);
    home
}

/// Commit `files` to the repo the way another host would have, at `at`.
fn commit(home: &Home, files: &[(&str, &str)], at: u64) {
    for (rel, text) in files {
        home.write(rel, text);
    }
    let date = format!("@{at} +0000");
    for args in [&["add", "-A"][..], &["commit", "-q", "-m", "from mbp"]] {
        let mut cmd = Command::new("git");
        git_env(&mut cmd);
        cmd.env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .arg("-C")
            .arg(home.repo())
            .args(args)
            .assert()
            .success();
    }
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

fn repo_file(home: &Home, rel: &str) -> String {
    fs::read_to_string(home.repo().join(rel)).unwrap()
}

#[test]
fn recent_lists_what_other_hosts_added_newest_first() {
    let home = host();
    commit(
        &home,
        &[("modules/adopted/bun.toml", &bun(MY_OS))],
        1_700_000_000,
    );
    commit(
        &home,
        &[
            ("dotfiles.toml", &format!("{GHOSTTY}\n{GITCONFIG}")),
            ("home/.config/ghostty/config", "font-size = 14\n"),
            ("home/.gitconfig", "[user]\n"),
            ("hosts/mbp/.gitconfig", "[user]\nname = mbp\n"),
        ],
        1_700_600_000,
    );
    let out = run(&home, &["recent"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(
        lines[0].starts_with("~/.config/ghostty/config  2023-11-21  mbp"),
        "{out}"
    );
    assert!(lines[1].starts_with("bun"), "{out}");
    assert!(
        lines[1].ends_with(&format!("2023-11-14  mbp · {MY_OS} · zsh")),
        "{out}"
    );

    let items: serde_json::Value =
        serde_json::from_str(&run(&home, &["recent", "--json"])).unwrap();
    assert_eq!(items[1]["name"], "bun");
    assert_eq!(items[1]["kind"], "module");
    assert_eq!(items[1]["added"], 1_700_000_000);
    assert_eq!(items[1]["widening"]["host"], "test");
}

#[test]
fn including_a_module_adds_this_host_and_its_os() {
    if !has("zsh") {
        return;
    }
    let home = host();
    commit(
        &home,
        &[("modules/adopted/bun.toml", &bun(OTHER_OS))],
        1_700_000_000,
    );
    home.cmd("rsmt")
        .args(["include", "bun"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--yes"));

    let out = run(&home, &["include", "bun", "--yes"]);
    assert!(out.contains("Included bun"), "{out}");
    let module = repo_file(&home, "modules/adopted/bun.toml");
    assert!(module.contains("hosts = [\"mbp\", \"test\"]"), "{module}");
    assert!(
        module.contains(&format!("os = [\"{OTHER_OS}\", \"{MY_OS}\"]")),
        "{module}"
    );
    let mut log = Command::new("git");
    let subject = log
        .args([
            "-C",
            &home.repo().to_string_lossy(),
            "log",
            "-1",
            "--format=%s",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&subject.stdout).trim(),
        "include: bun on test"
    );
    let shell = home.shell(&[
        "zsh".into(),
        "-i".into(),
        "-c".into(),
        "print -r -- $BUN_INSTALL".into(),
    ]);
    assert_eq!(
        String::from_utf8_lossy(&shell.stdout).trim(),
        home.root.join(".bun").display().to_string()
    );
    assert_eq!(
        run(&home, &["recent"]),
        "Everything in the repo loads on this host\n"
    );

    home.cmd("rsmt")
        .args(["include", "bun", "--yes"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("bun already loads on this host"));
    home.cmd("rsmt")
        .args(["include", "nope", "--yes"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("nope is not in the repo"));
}

#[test]
fn including_a_dotfile_deploys_it() {
    let home = host();
    commit(
        &home,
        &[
            ("dotfiles.toml", GHOSTTY),
            ("home/.config/ghostty/config", "font-size = 14\n"),
        ],
        1_700_000_000,
    );
    run(&home, &["include", ".config/ghostty/config", "--yes"]);
    assert!(
        repo_file(&home, "dotfiles.toml").contains("when = { hosts = [\"mbp\", \"test\"] }"),
        "{}",
        repo_file(&home, "dotfiles.toml")
    );
    assert_eq!(
        fs::read_to_string(home.root.join(".config/ghostty/config")).unwrap(),
        "font-size = 14\n"
    );
}

/// A dotfile changes no shell script, so a shell that cannot be checked
/// this time does not stand in its way.
#[test]
fn including_a_dotfile_needs_no_shell_check() {
    use std::os::unix::fs::PermissionsExt;
    if !has("zsh") {
        return;
    }
    let home = host();
    commit(
        &home,
        &[
            ("dotfiles.toml", GHOSTTY),
            ("home/.config/ghostty/config", "font-size = 14\n"),
        ],
        1_700_000_000,
    );
    let fake = home.root.join("fake-bin");
    fs::create_dir_all(&fake).unwrap();
    fs::write(fake.join("zsh"), "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(fake.join("zsh"), fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", fake.display(), std::env::var("PATH").unwrap());
    home.cmd("rsmt")
        .args(["include", ".config/ghostty/config", "--yes"])
        .env("PATH", path)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(home.root.join(".config/ghostty/config")).unwrap(),
        "font-size = 14\n"
    );
}

#[test]
fn what_cannot_load_here_is_skipped_with_a_reason() {
    let home = host();
    let work = "[when]\ntags = [\"work\"]\n\n[env]\nWORK = \"1\"\n";
    let mut files = vec![
        ("modules/work.toml", work.to_string()),
        ("modules/adopted/bun.toml", bun(MY_OS)),
    ];
    // Only a host without fish has no hook for it.
    let fish = !has("fish");
    if fish {
        files.push((
            "modules/fishy.toml",
            "shells = [\"fish\"]\n\n[when]\nhosts = [\"mbp\"]\n\n[raw]\nfish = \"set -gx FISHY 1\\n\"\n"
                .to_string(),
        ));
    }
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, text)| (*rel, text.as_str()))
        .collect();
    commit(&home, &files, 1_700_000_000);
    let out = run(&home, &["recent"]);
    assert!(out.contains("tag work (needs tag work)"), "{out}");

    let out = run(&home, &["include", "work", "--yes"]);
    assert!(out.contains("Skipped work: needs tag work"), "{out}");
    assert!(out.contains("Nothing to include"), "{out}");
    assert_eq!(repo_file(&home, "modules/work.toml"), work);

    let out = run(&home, &["include", "--all", "--yes"]);
    assert!(out.contains("Included bun"), "{out}");
    if fish {
        assert!(out.contains("Skipped fishy: only for fish"), "{out}");
    }
}

#[test]
fn names_paths_moves_and_several_entries() {
    let home = host();
    let off = "enabled = false\n\n[when]\nhosts = [\"mbp\"]\n\n[env]\nOFF = \"1\"\n";
    // mbp's own version, and box's, which this host would need more for.
    let tool = format!(
        "[[dotfile]]\npath = \"bin/tool\"\nsource = \"hosts/mbp/bin/tool\"\n[dotfile.when]\nhosts = [\"mbp\"]\nos = [\"{MY_OS}\"]\n\n[[dotfile]]\npath = \"bin/tool\"\nsource = \"hosts/box/bin/tool\"\n[dotfile.when]\nhosts = [\"box\"]\nos = [\"{OTHER_OS}\"]\n"
    );
    commit(
        &home,
        &[
            ("modules/adopted/bun.toml", &bun(MY_OS)),
            ("modules/off.toml", off),
            ("dotfiles.toml", &tool),
            ("hosts/mbp/bin/tool", "mbp\n"),
            ("hosts/box/bin/tool", "box\n"),
        ],
        1_700_000_000,
    );
    fs::create_dir_all(home.repo().join("modules/tools")).unwrap();
    let mut mv = Command::new("git");
    git_env(&mut mv);
    mv.arg("-C")
        .arg(home.repo())
        .args(["mv", "modules/adopted/bun.toml", "modules/tools/bun.toml"])
        .assert()
        .success();
    commit(&home, &[], 1_700_600_000);

    let out = run(&home, &["recent"]);
    assert!(!out.contains("not committed"), "{out}");
    assert!(!out.contains("off"), "{out}");
    home.cmd("rsmt")
        .args(["include", "off", "--yes"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("off is turned off"));

    run(&home, &["include", "bin/tool", "--yes"]);
    let manifest = repo_file(&home, "dotfiles.toml");
    let mbp = format!("hosts = [\"mbp\", \"test\"]\nos = [\"{MY_OS}\"]");
    assert!(manifest.contains(&mbp), "{manifest}");
    assert!(manifest.contains("hosts = [\"box\"]\n"), "{manifest}");
    assert_eq!(
        fs::read_to_string(home.root.join("bin/tool")).unwrap(),
        "mbp\n"
    );
}
