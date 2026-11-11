//! adopt, review and restore <id> against a throwaway HOME and the shells
//! installed here.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{Home, git_env, has, sessions};
use predicates::prelude::*;
use rosamta_core::hooks::{BASH_PROFILE_SEED, strip_block};

const BUN: &str =
    "\n# bun\nexport BUN_INSTALL=\"$HOME/.bun\"\nexport PATH=\"$BUN_INSTALL/bin:$PATH\"\n";

/// A HOME with a repo, this host and hooks for every installed shell.
fn hooked(files: &[(&str, &str)]) -> Home {
    let home = Home::new();
    for (rel, text) in files {
        write(&home, rel, text);
    }
    // Ask, so the tests decide each item; the default adopts at once.
    write(
        &home,
        ".config/rosamta/host.toml",
        "id = \"test\"\nadopt = \"ask\"\n",
    );
    home.cmd("rsmt").arg("init").assert().success();
    run(&home, &["apply"]);
    home
}

fn write(home: &Home, rel: &str, text: &str) {
    let path = home.root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        text.replace("@HOME@", &home.root.display().to_string()),
    )
    .unwrap();
}

fn read(home: &Home, rel: &str) -> String {
    fs::read_to_string(home.root.join(rel)).unwrap_or_default()
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

fn set_host(home: &Home, key: &str, value: &str) {
    let path = home.host_config();
    let text = fs::read_to_string(&path).unwrap();
    let line = format!("{key} = {value}");
    let mut lines: Vec<String> = text
        .lines()
        .map(|old| {
            if old.starts_with(&format!("{key} =")) {
                line.clone()
            } else {
                old.to_string()
            }
        })
        .collect();
    if !lines.contains(&line) {
        lines.push(line);
    }
    fs::write(path, lines.join("\n") + "\n").unwrap();
}

fn repo_git(home: &Home, args: &[&str]) -> String {
    let mut cmd = assert_cmd::Command::new("git");
    git_env(&mut cmd);
    let out = cmd.arg("-C").arg(home.repo()).args(args).output().unwrap();
    String::from_utf8(out.stdout).unwrap()
}

fn adopted(home: &Home) -> Vec<PathBuf> {
    let dir = home.repo().join("modules/adopted");
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .map(|entry| entry.unwrap().path())
        .collect();
    out.sort();
    out
}

fn review(home: &Home) -> Vec<serde_json::Value> {
    serde_json::from_str::<Vec<serde_json::Value>>(&run(home, &["review", "--json"])).unwrap()
}

/// What a new shell ends up with: exported variables, PATH and aliases.
fn state(home: &Home) -> Vec<String> {
    let posix = "env | LC_ALL=C sort | grep -v -e '^ROSAMTA_' -e '^SHLVL=' -e '^_=' -e '^OLDPWD=' -e '^PWD='; alias";
    let fish = "env | LC_ALL=C sort | grep -v -e '^ROSAMTA_' -e '^SHLVL=' -e '^_=' -e '^OLDPWD=' -e '^PWD='; alias";
    sessions(posix, fish)
        .iter()
        .map(|argv| String::from_utf8_lossy(&home.shell(argv).stdout).into_owned())
        .collect()
}

/// HOME-relative files a fixture sources with `.` or `source`.
fn sourced_files(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        let Some(rest) = line
            .strip_prefix(". ")
            .or_else(|| line.strip_prefix("source "))
        else {
            continue;
        };
        let path = rest
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches('"');
        for home in ["$HOME/", "~/", "@HOME@/"] {
            if let Some(rel) = path.strip_prefix(home) {
                out.push(rel.to_string());
            }
        }
    }
    out
}

/// Nothing a shell had is lost or changed. Structured modules load in every
/// shell, so a shell may gain what another shell's file declared.
fn keeps(before: &str, after: &str, what: &str) {
    let path = |text: &str| -> Vec<String> {
        text.lines()
            .find_map(|line| line.strip_prefix("PATH="))
            .map(|path| path.split(':').map(str::to_string).collect())
            .unwrap_or_default()
    };
    let (old, new) = (path(before), path(after));
    let mut rest = new.iter();
    for entry in &old {
        assert!(
            rest.any(|seen| seen == entry),
            "{what}: PATH lost or reordered {entry}: {old:?} -> {new:?}"
        );
    }
    for line in before.lines().filter(|line| !line.starts_with("PATH=")) {
        assert!(
            after.lines().any(|seen| seen == line),
            "{what}: lost {line:?}"
        );
    }
}

/// Every installer fixture: after `rsmt adopt --yes`, rc files hold only the
/// hook and new shells keep their variables, PATH and aliases.
#[test]
fn adopting_installer_changes_keeps_shells_the_same() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/installers");
    let mut names: Vec<_> = fs::read_dir(&fixtures)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.join("after").is_dir())
        .collect();
    names.sort();
    for fixture in names {
        let after = fixture.join("after");
        let mut files = Vec::new();
        let mut stack = vec![after.clone()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let rel = path.strip_prefix(&after).unwrap().display().to_string();
                    files.push((rel, fs::read_to_string(&path).unwrap()));
                }
            }
        }
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let home = Home::new();
        // What the installer put in HOME besides rc files: the scripts they source.
        for (_, text) in &files {
            for sourced in sourced_files(text) {
                write(&home, &sourced, "");
            }
        }
        for (rel, text) in &refs {
            write(&home, rel, text);
        }
        home.cmd("rsmt").arg("init").assert().success();
        run(&home, &["apply"]);
        let before = state(&home);
        run(&home, &["adopt", "--yes"]);
        for (before, after) in before.iter().zip(state(&home)) {
            keeps(before, &after, &fixture.display().to_string());
        }
        for (rel, _) in &files {
            if rel.contains("conf.d") && !has("fish") {
                continue;
            }
            let left = strip_block(&read(&home, rel));
            let rest: Vec<&str> = left
                .lines()
                .filter(|line| !line.trim().is_empty() && *line != BASH_PROFILE_SEED.trim_end())
                .collect();
            let owner_hooked = match rel.as_str() {
                ".profile" => has("sh"),
                rel if rel.starts_with(".bash") => has("bash"),
                rel if rel.starts_with(".z") => has("zsh"),
                _ => has("fish"),
            };
            if owner_hooked {
                assert!(rest.is_empty(), "{}: {rel}: {rest:?}", fixture.display());
            }
        }
    }
}

#[test]
fn structured_content_becomes_one_module_and_commit() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", &format!("{BUN}alias cc=\"claude\"\n"))]);
    let host = read(&home, ".config/rosamta/host.toml");
    let host = host
        .lines()
        .next()
        .unwrap()
        .trim_start_matches("id = ")
        .trim_matches('"')
        .to_string();
    home.cmd("rsmt")
        .arg("adopt")
        .assert()
        .code(2)
        .stdout(predicate::str::contains(
            "~/.zshrc:2-5 → modules/adopted/bun.toml",
        ))
        .stderr(predicate::str::contains("--yes"));
    run(&home, &["adopt", "--yes"]);
    let module = read(&home, ".local/share/rosamta/repo/modules/adopted/bun.toml");
    assert!(module.contains("BUN_INSTALL = \"$HOME/.bun\""), "{module}");
    assert!(
        module.contains("prepend = [\"$BUN_INSTALL/bin\"]"),
        "{module}"
    );
    assert!(module.contains("cc = \"claude\""), "{module}");
    assert!(
        module.contains(&format!("hosts = [\"{host}\"]")),
        "{module}"
    );
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    assert!(module.contains(&format!("os = [\"{os}\"]")), "{module}");
    assert!(module.contains("shells = [\"zsh\"]"), "{module}");
    assert_eq!(
        repo_git(&home, &["log", "-1", "--format=%s"]).trim(),
        format!("adopt(zsh): add bun env, PATH and aliases from ~/.zshrc on {host}")
    );
}

#[test]
fn names_that_collide_get_a_number() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", &format!("{BUN}\nexport BUN_X=\"$HOME/.bun/x\"\n"))]);
    run(&home, &["adopt", "--yes"]);
    let names: Vec<String> = adopted(&home)
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["bun-2.toml", "bun.toml"]);
}

#[test]
fn review_lists_accepts_and_ignores() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", &format!("{BUN}\nexport FOO=1\n"))]);
    let before = read(&home, ".zshrc");
    let items = review(&home);
    assert_eq!(items.len(), 2);
    assert_eq!(read(&home, ".zshrc"), before, "ask only lists");
    let listing = run(&home, &["review"]);
    assert!(
        listing.contains("~/.zshrc:2-4") && listing.contains("modules/adopted/bun.toml"),
        "{listing}"
    );

    let bun = items[0]["id"].to_string();
    let foo = items[1]["id"].to_string();
    home.cmd("rsmt")
        .args(["review", "discard", &bun])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("discard applies only to dotfiles"));
    run(&home, &["review", "accept", &bun, "--scope", "all"]);
    let module = read(&home, ".local/share/rosamta/repo/modules/adopted/bun.toml");
    assert!(!module.contains("[when]"), "{module}");
    assert!(module.contains("shells = [\"zsh\"]"), "{module}");
    assert!(!read(&home, ".zshrc").contains("BUN_INSTALL"));

    run(&home, &["review", "ignore", &foo]);
    assert!(review(&home).is_empty());
    assert!(read(&home, ".zshrc").contains("export FOO=1"));
    // Editing an ignored segment brings it back.
    let edited = read(&home, ".zshrc").replace("export FOO=1", "export FOO=2");
    fs::write(home.root.join(".zshrc"), edited).unwrap();
    assert_eq!(review(&home).len(), 1);
}

#[test]
fn adopt_policy_off_and_auto() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", BUN)]);
    set_host(&home, "adopt", "\"off\"");
    let before = read(&home, ".zshrc");
    assert!(review(&home).is_empty());
    assert_eq!(read(&home, ".zshrc"), before);
    assert!(!home.root.join(".local/state/rosamta/queue.json").exists());

    set_host(&home, "adopt", "\"auto\"");
    run(&home, &["review"]);
    assert!(!read(&home, ".zshrc").contains("BUN_INSTALL"));
    assert_eq!(adopted(&home).len(), 1);
}

#[test]
fn reinstalled_content_is_removed_without_a_new_module() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", BUN)]);
    run(&home, &["adopt", "--yes"]);
    let commits = repo_git(&home, &["rev-list", "--count", "HEAD"]);
    let zshrc = read(&home, ".zshrc");
    fs::write(home.root.join(".zshrc"), format!("{zshrc}{BUN}")).unwrap();
    assert!(review(&home).is_empty());
    assert_eq!(read(&home, ".zshrc"), zshrc);
    assert_eq!(adopted(&home).len(), 1);
    assert_eq!(repo_git(&home, &["rev-list", "--count", "HEAD"]), commits);
}

#[test]
fn reinstalled_content_comes_back_once_its_module_changes() {
    if !has("zsh") {
        return;
    }
    let orbstack = "\nsource ~/.orbstack/shell/init.zsh 2>/dev/null || :\n";
    let home = hooked(&[(".zshrc", &format!("{BUN}{orbstack}"))]);
    run(&home, &["adopt", "--yes"]);
    // bun's module is gone; orbstack's now runs something else.
    fs::remove_file(home.repo().join("modules/adopted/bun.toml")).unwrap();
    let snippet = adopted(&home)
        .into_iter()
        .find(|path| fs::read_to_string(path).unwrap().contains("init.zsh"))
        .unwrap();
    let text = fs::read_to_string(&snippet).unwrap();
    fs::write(&snippet, text.replace("init.zsh", "other.zsh")).unwrap();
    let zshrc = read(&home, ".zshrc");
    fs::write(home.root.join(".zshrc"), format!("{zshrc}{BUN}{orbstack}")).unwrap();
    assert_eq!(review(&home).len(), 2);
    let zshrc = read(&home, ".zshrc");
    assert!(
        zshrc.contains("BUN_INSTALL") && zshrc.contains("init.zsh"),
        "{zshrc}"
    );
}

#[test]
fn comment_mode_keeps_the_lines_as_comments() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", BUN)]);
    set_host(&home, "adopt_original", "\"comment\"");
    run(&home, &["adopt", "--yes"]);
    let zshrc = read(&home, ".zshrc");
    assert!(
        zshrc.contains(
            "# rosamta: adopted → modules/adopted/bun.toml\n# # bun\n# export BUN_INSTALL"
        ),
        "{zshrc}"
    );
    assert!(review(&home).is_empty());
}

#[test]
fn restore_puts_one_adoption_back() {
    if !has("zsh") {
        return;
    }
    let zprofile = "export EARLY=1\n\n# Added by OrbStack: command-line tools and integration\n# This won't be added again if you remove it.\nsource ~/.orbstack/shell/init.zsh 2>/dev/null || :\n\nexport LATE=1\n";
    let home = hooked(&[(".zprofile", zprofile)]);
    let hooked_text = read(&home, ".zprofile");
    let out = run(&home, &["adopt", "--yes"]);
    let id = out
        .lines()
        .find(|line| line.contains("orbstack"))
        .and_then(|line| line.strip_prefix("Adopted "))
        .and_then(|line| line.split(':').next())
        .unwrap()
        .to_string();
    // The other two lines stay adopted; only OrbStack's come back.
    home.cmd("rsmt")
        .args(["restore", &id])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--yes"));
    run(&home, &["restore", &id, "--yes"]);
    let text = read(&home, ".zprofile");
    assert!(text.contains("\n# Added by OrbStack: command-line tools and integration\n# This won't be added again if you remove it.\nsource ~/.orbstack/shell/init.zsh 2>/dev/null || :\n"), "{text}");
    assert!(!text.contains("EARLY") && !text.contains("LATE"), "{text}");
    assert!(!home.repo().join("modules/adopted/orbstack.toml").exists());
    assert!(
        repo_git(&home, &["log", "-1", "--format=%s"])
            .starts_with("restore(zsh): put back orbstack in ~/.zprofile")
    );
    // It stays in the rc file from now on.
    assert!(review(&home).is_empty());
    // Undoing every adoption brings back the original file.
    let ids: Vec<String> = out
        .lines()
        .filter_map(|line| line.strip_prefix("Adopted "))
        .filter_map(|line| line.split(':').next())
        .filter(|other| *other != id)
        .map(str::to_string)
        .collect();
    for other in ids {
        run(&home, &["restore", &other, "--yes"]);
    }
    assert_eq!(read(&home, ".zprofile"), hooked_text);
}

/// `.zprofile` snippets load where they did: in login zsh, after the
/// system zprofile (macOS path_helper), so their PATH stays in front.
#[test]
fn zprofile_snippets_keep_their_place_in_login_zsh() {
    if !has("zsh") {
        return;
    }
    let zprofile = "eval \"$(echo 'export PATH=/opt/fakebrew/bin:$PATH')\"\ntypeset -g LOGIN_ONLY=1; export LOGIN_ONLY\n";
    let home = hooked(&[(".zprofile", zprofile)]);
    let probe = |login: bool| {
        let mut argv: Vec<String> = vec!["zsh".into()];
        if login {
            argv.push("-l".into());
        }
        argv.extend(["-c".into(), "print -r -- \"$LOGIN_ONLY|$PATH\"".into()]);
        String::from_utf8_lossy(&home.shell(&argv).stdout).into_owned()
    };
    let (login, plain) = (probe(true), probe(false));
    run(&home, &["adopt", "--yes"]);
    assert!(
        read(
            &home,
            ".local/share/rosamta/repo/modules/adopted/zprofile.toml"
        )
        .contains("phase = \"login\"")
    );
    assert_eq!(probe(true), login);
    assert_eq!(probe(false), plain);
    let path = login.trim().split_once('|').unwrap().1;
    let entries: Vec<&str> = path.split(':').collect();
    let at = |entry: &str| entries.iter().position(|seen| *seen == entry);
    if let Some(system) = at("/usr/bin") {
        assert!(at("/opt/fakebrew/bin").unwrap() < system, "{path}");
    }
    // Eject writes the snippet back into .zprofile.
    run(&home, &["eject", "--yes"]);
    fs::remove_dir_all(home.root.join(".local/share/rosamta/generated")).unwrap();
    assert_eq!(probe(true), login);
}

#[test]
fn secrets_stay_on_this_host() {
    if !has("zsh") {
        return;
    }
    let secret = "sk-proj-Zk3q9LmP2vR8tY6wB1nC4xJ7aa";
    let home = hooked(&[(".zshrc", &format!("export OPENAI_API_KEY={secret}\n"))]);
    let listing = run(&home, &["review"]);
    assert!(listing.contains("(this host only)"), "{listing}");
    run(&home, &["adopt", "--yes"]);
    let out = home.shell(&[
        "zsh".into(),
        "-i".into(),
        "-c".into(),
        "print -r -- $OPENAI_API_KEY".into(),
    ]);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), secret);
    assert!(!repo_git(&home, &["log", "-p", "--all"]).contains(secret));
    let local = ".config/rosamta/local/modules/openai-api-key.toml";
    assert!(read(&home, local).contains(secret));
    let mode = fs::metadata(home.root.join(local))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

/// Shell startup that appends to `.zshrc` once, the way another process
/// could edit it between rosamta reading the file and writing it.
const EDITS_ZSHRC: &str = "[ -e \"$HOME/edit-me\" ] && { rm \"$HOME/edit-me\"; echo 'export LATER=1' >> \"$HOME/.zshrc\"; }\n";

#[test]
fn an_rc_file_edited_after_reading_is_not_overwritten() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", &format!("{EDITS_ZSHRC}{BUN}"))]);
    // The shells apply starts first, before writing, run the edit.
    write(&home, "edit-me", "");
    home.cmd("rsmt")
        .args(["adopt", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("changed"));
    let zshrc = read(&home, ".zshrc");
    assert!(
        zshrc.contains("export LATER=1") && zshrc.contains("BUN_INSTALL"),
        "{zshrc}"
    );
    assert!(adopted(&home).is_empty());

    let out = run(&home, &["adopt", "--yes"]);
    let bun = out
        .lines()
        .find(|line| line.contains("bun.toml"))
        .and_then(|line| line.strip_prefix("Adopted "))
        .and_then(|line| line.split(':').next())
        .unwrap()
        .to_string();
    // Now the adopted snippet does the edit.
    write(&home, "edit-me", "");
    home.cmd("rsmt")
        .args(["restore", &bun, "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("changed"));
    let zshrc = read(&home, ".zshrc");
    assert!(
        zshrc.contains("export LATER=1") && !zshrc.contains("BUN_INSTALL"),
        "{zshrc}"
    );
    assert!(home.repo().join("modules/adopted/bun.toml").exists());
}

/// Structured content keeps to the shells that read its file.
#[test]
fn structured_content_stays_in_its_shells() {
    if !has("zsh") || !has("sh") {
        return;
    }
    let home = hooked(&[
        (".zshrc", "export ONLY_ZSH=1\n"),
        (".profile", "export EVERY_POSIX=1\n"),
    ]);
    run(&home, &["adopt", "--yes"]);
    let dir = home.repo().join("modules/adopted");
    let module = |name: &str| {
        fs::read_dir(&dir)
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .find(|text| text.contains(name))
            .unwrap_or_else(|| panic!("no module sets {name}"))
    };
    let zsh = module("ONLY_ZSH");
    assert!(zsh.contains("shells = [\"zsh\"]"), "{zsh}");
    let posix = module("EVERY_POSIX");
    assert!(
        posix.contains("shells = [\"sh\", \"bash\", \"zsh\"]"),
        "{posix}"
    );
}

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

/// The bun module as host `mbp` on `os` would have adopted it from
/// `.zshrc`, for `shells`.
fn bun_from_mbp(os: &str, shells: &str) -> String {
    let home = hooked(&[(".zshrc", BUN)]);
    run(&home, &["adopt", "--yes"]);
    read(&home, ".local/share/rosamta/repo/modules/adopted/bun.toml")
        .replace("hosts = [\"test\"]", "hosts = [\"mbp\"]")
        .replace(&format!("os = [\"{MY_OS}\"]"), &format!("os = [\"{os}\"]"))
        .replace("shells = [\"zsh\"]", &format!("shells = [{shells}]"))
}

#[test]
fn the_same_lines_on_a_second_host_join_its_module() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", BUN)]);
    home.write(
        "modules/adopted/bun.toml",
        &bun_from_mbp(OTHER_OS, "\"zsh\""),
    );
    let listing = run(&home, &["review"]);
    assert!(
        listing.contains("modules/adopted/bun.toml (adds this host)"),
        "{listing}"
    );
    let out = run(&home, &["adopt", "--yes"]);
    let bun = || read(&home, ".local/share/rosamta/repo/modules/adopted/bun.toml");
    let module = bun();
    assert!(module.contains("hosts = [\"mbp\", \"test\"]"), "{module}");
    assert!(
        module.contains(&format!("os = [\"{OTHER_OS}\", \"{MY_OS}\"]")),
        "{module}"
    );
    assert_eq!(adopted(&home).len(), 1);
    assert!(!read(&home, ".zshrc").contains("BUN_INSTALL"));
    assert_eq!(
        repo_git(&home, &["log", "-1", "--format=%s"]).trim(),
        "adopt(zsh): include bun from ~/.zshrc on test"
    );

    run(&home, &["restore", &adopted_id(&out), "--yes"]);
    assert!(read(&home, ".zshrc").contains("BUN_INSTALL"));
    let module = bun();
    assert!(module.contains("hosts = [\"mbp\"]\n"), "{module}");
}

/// The id `rsmt adopt` printed for its only adoption.
fn adopted_id(out: &str) -> String {
    out.lines()
        .find_map(|line| line.strip_prefix("Adopted "))
        .and_then(|rest| rest.split(':').next())
        .unwrap()
        .to_string()
}

const BUN_MODULE: &str = ".local/share/rosamta/repo/modules/adopted/bun.toml";

#[test]
fn restoring_keeps_a_module_another_host_joined() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", BUN)]);
    let out = run(&home, &["adopt", "--yes"]);
    let joined = read(&home, BUN_MODULE).replace("[\"test\"]", "[\"test\", \"box\"]");
    write(&home, BUN_MODULE, &joined);
    run(&home, &["restore", &adopted_id(&out), "--yes"]);
    let module = read(&home, BUN_MODULE);
    assert!(module.contains("hosts = [\"box\"]\n"), "{module}");
}

#[test]
fn restoring_keeps_an_every_host_module_for_the_others() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", BUN)]);
    home.write("rosamta.toml", "format = 1\nhosts = [\"test\", \"box\"]\n");
    let item = review(&home).remove(0);
    let out = run(
        &home,
        &[
            "review",
            "accept",
            &item["id"].to_string(),
            "--scope",
            "all",
        ],
    );
    assert!(!read(&home, BUN_MODULE).contains("hosts"));
    run(&home, &["restore", &adopted_id(&out), "--yes"]);
    let module = read(&home, BUN_MODULE);
    assert!(module.contains("hosts = [\"box\"]\n"), "{module}");
    assert!(read(&home, ".zshrc").contains("BUN_INSTALL"));
}

#[test]
fn joining_for_every_host_comes_undone() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", BUN)]);
    home.write(
        "modules/adopted/bun.toml",
        &bun_from_mbp(OTHER_OS, "\"zsh\""),
    );
    set_host(&home, "adopt_original", "\"comment\"");
    let item = review(&home).remove(0);
    assert_eq!(item["include"], true, "{item}");
    run(
        &home,
        &[
            "review",
            "accept",
            &item["id"].to_string(),
            "--scope",
            "all",
        ],
    );
    let module = read(&home, BUN_MODULE);
    assert!(
        !module.contains("hosts") && !module.contains("os ="),
        "{module}"
    );
    let zshrc = read(&home, ".zshrc");
    assert!(
        zshrc.contains("# rosamta: adopted → modules/adopted/bun.toml\n"),
        "{zshrc}"
    );
    let adoptions: serde_json::Value = serde_json::from_str(&adoptions_json(&home)).unwrap();
    let id = adoptions["adoptions"][0]["id"].to_string();
    run(&home, &["restore", &id, "--yes"]);
    let module = read(&home, BUN_MODULE);
    assert!(
        module.contains(&format!("hosts = [\"mbp\"]\nos = [\"{OTHER_OS}\"]")),
        "{module}"
    );
}

#[test]
fn secrets_never_join_another_hosts_module() {
    if !has("zsh") {
        return;
    }
    let secret = "sk-proj-Zk3q9LmP2vR8tY6wB1nC4xJ7aa";
    let home = hooked(&[(".zshrc", &format!("export OPENAI_API_KEY={secret}\n"))]);
    home.write(
        "modules/adopted/openai.toml",
        &format!(
            "order = 7001\nshells = [\"zsh\"]\n\n[when]\nhosts = [\"mbp\"]\n\n[env]\nOPENAI_API_KEY = \"{secret}\"\n"
        ),
    );
    let item = review(&home).remove(0);
    assert!(
        item["local"] == true && item.get("include").is_none(),
        "{item}"
    );
}

#[test]
fn the_same_lines_from_another_shell_get_their_own_module() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", BUN)]);
    let theirs = bun_from_mbp(MY_OS, "\"bash\"");
    home.write("modules/adopted/bun.toml", &theirs);
    run(&home, &["adopt", "--yes"]);
    assert_eq!(adopted(&home).len(), 2);
    assert_eq!(
        read(&home, ".local/share/rosamta/repo/modules/adopted/bun.toml"),
        theirs
    );
}

/// The same line in two files: adopting the zsh one leaves `.profile`'s,
/// since a zsh snippet does nothing for sh.
#[test]
fn a_snippet_replaces_only_lines_in_its_own_shells() {
    if !has("zsh") || !has("sh") {
        return;
    }
    let line = ". \"$HOME/.cargo/env\"\n";
    let home = hooked(&[(".profile", line), (".zshrc", line), (".cargo/env", "")]);
    let items = review(&home);
    let zsh = items
        .iter()
        .find(|item| item["file"].as_str().unwrap().ends_with(".zshrc"))
        .unwrap()["id"]
        .to_string();
    run(&home, &["review", "accept", &zsh]);
    let items = review(&home);
    assert_eq!(items.len(), 1, "{items:?}");
    assert!(read(&home, ".profile").contains(line));
}

/// A module for another host, or one turned off, does not stand in for
/// this host's lines.
#[test]
fn modules_that_do_not_load_here_replace_nothing() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", "export FOO=1\n\nexport BAR=1\n")]);
    home.write(
        "modules/adopted/foo.toml",
        "[when]\nhosts = [\"other\"]\n\n[env]\nFOO = \"1\"\n",
    );
    home.write(
        "modules/adopted/bar.toml",
        "enabled = false\n\n[env]\nBAR = \"1\"\n",
    );
    let items = review(&home);
    assert_eq!(items.len(), 2);
    // A module from before `shells` was recorded is never joined.
    assert!(items.iter().all(|item| item.get("include").is_none()));
    let zshrc = read(&home, ".zshrc");
    assert!(zshrc.contains("FOO") && zshrc.contains("BAR"), "{zshrc}");
}

fn zsh_prints(home: &Home, expr: &str) -> String {
    let out = home.shell(&[
        "zsh".into(),
        "-i".into(),
        "-c".into(),
        format!("print -r -- {expr}"),
    ]);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// In a late file, an assignment after a snippet still runs after it.
#[test]
fn env_after_a_snippet_keeps_its_place() {
    if !has("zsh") {
        return;
    }
    let zshrc = "eval \"$(echo export FOO=early)\"\n\nexport FOO=late\n";
    let home = hooked(&[(".zshrc", zshrc)]);
    assert_eq!(zsh_prints(&home, "$FOO"), "late\n");
    run(&home, &["adopt", "--yes"]);
    assert_eq!(zsh_prints(&home, "$FOO"), "late\n");
}

/// An ignored snippet stays in the file and still changes PATH there, so
/// a PATH change after it keeps its place too.
#[test]
fn an_ignored_snippet_keeps_path_order() {
    if !has("zsh") {
        return;
    }
    let zshrc =
        "eval \"$(echo 'export PATH=/opt/a/bin:$PATH')\"\n\nexport PATH=\"/opt/b/bin:$PATH\"\n";
    let home = hooked(&[(".zshrc", zshrc)]);
    let items = review(&home);
    let eval = items
        .iter()
        .find(|item| item["text"].as_str().unwrap().starts_with("eval"))
        .unwrap()["id"]
        .to_string();
    run(&home, &["review", "ignore", &eval]);
    // Only the two entries, after the review notice line.
    let order = || {
        let out = zsh_prints(&home, "${(M)path:#/opt/[ab]/bin}");
        out.lines().last().unwrap_or_default().to_string()
    };
    assert_eq!(order(), "/opt/b/bin /opt/a/bin");
    run(&home, &["adopt", "--yes"]);
    assert_eq!(order(), "/opt/b/bin /opt/a/bin");
}

/// A file that is not UTF-8 is left exactly as it is.
#[test]
fn files_that_are_not_utf8_are_left_alone() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[]);
    let zlogin = b"export A=1\n# caf\xe9\n".to_vec();
    fs::write(home.root.join(".zlogin"), &zlogin).unwrap();
    assert!(review(&home).is_empty());
    run(&home, &["adopt", "--yes"]);
    assert_eq!(fs::read(home.root.join(".zlogin")).unwrap(), zlogin);
    assert!(adopted(&home).is_empty());
}

fn adoptions_json(home: &Home) -> String {
    read(home, ".local/state/rosamta/adoptions.json")
}

/// A commit that fails for a reason other than signing (here a stale
/// `index.lock`) still leaves the adoption recorded and the warning shown.
#[test]
fn a_failed_commit_keeps_the_adoption_record() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", BUN)]);
    let lock = home.repo().join(".git/index.lock");
    fs::write(&lock, "").unwrap();
    home.cmd("rsmt")
        .args(["adopt", "--yes"])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning: commit failed"));
    assert!(!read(&home, ".zshrc").contains("BUN_INSTALL"));
    assert!(adoptions_json(&home).contains("bun.toml"));
    fs::remove_file(&lock).unwrap();
    // Re-added content is recognized from the record.
    let zshrc = read(&home, ".zshrc");
    fs::write(home.root.join(".zshrc"), format!("{zshrc}{BUN}")).unwrap();
    assert!(review(&home).is_empty());
    assert_eq!(read(&home, ".zshrc"), zshrc);
}

#[test]
fn a_failed_commit_on_restore_keeps_the_lines() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", BUN)]);
    let out = run(&home, &["adopt", "--yes"]);
    let id = out
        .lines()
        .find_map(|line| line.strip_prefix("Adopted "))
        .and_then(|line| line.split(':').next())
        .unwrap()
        .to_string();
    fs::write(home.repo().join(".git/index.lock"), "").unwrap();
    home.cmd("rsmt")
        .args(["restore", &id, "--yes"])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning: commit failed"));
    assert!(!adoptions_json(&home).contains("bun.toml"));
    // The next review must not take the restored lines out again.
    run(&home, &["review"]);
    assert!(read(&home, ".zshrc").contains("BUN_INSTALL"));
}

#[test]
fn interactive_shells_print_the_review_notice() {
    if !has("zsh") {
        return;
    }
    let home = hooked(&[(".zshrc", BUN)]);
    run(&home, &["review"]);
    let interactive = |home: &Home| {
        let out = home.shell(&["zsh".into(), "-i".into(), "-c".into(), "exit".into()]);
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert_eq!(
        interactive(&home),
        "rosamta: 1 change to review. Run: rsmt review\n"
    );
    let out = home.shell(&["zsh".into(), "-c".into(), "exit".into()]);
    assert!(out.stdout.is_empty());

    set_host(&home, "shell_notice", "false");
    run(&home, &["review"]);
    assert_eq!(interactive(&home), "");
}
