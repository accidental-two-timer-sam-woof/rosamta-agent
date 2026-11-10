//! apply, hook, unhook, eject and restore against a throwaway HOME and the
//! shells installed here. Checks for a missing shell are skipped.

mod common;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{Home, git_env, has, sessions};
use predicates::prelude::*;
use rosamta_core::hooks::{BEGIN, strip_block};

const MODULE: &str = r#"
[env]
ROSAMTA_TEST = "it's $HOME"

[path]
prepend = ["~/mybin"]
append = ["~/later"]

[aliases]
ll = "ls -l"
"#;

/// Every file a hook may live in, relative to HOME.
const RC_FILES: [&str; 11] = [
    ".profile",
    ".bash_profile",
    ".bash_login",
    ".bashrc",
    ".zshenv",
    ".zshrc",
    ".zprofile",
    ".config/zsh/.zshenv",
    ".config/zsh/.zshrc",
    ".config/zsh/.zprofile",
    ".config/fish/conf.d/rosamta.fish",
];

fn set_up() -> Home {
    let home = Home::new();
    home.cmd("rsmt").arg("init").assert().success();
    home.write("modules/basics.toml", MODULE);
    home
}

fn apply(home: &Home) {
    let out = home.cmd("rsmt").arg("apply").output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn rc_files(home: &Home) -> Vec<(&'static str, Option<Vec<u8>>)> {
    RC_FILES
        .iter()
        .map(|rel| (*rel, fs::read(home.root.join(rel)).ok()))
        .collect()
}

fn read(home: &Home, rel: &str) -> String {
    fs::read_to_string(home.root.join(rel)).unwrap_or_default()
}

fn current(home: &Home) -> PathBuf {
    home.root.join(".local/share/rosamta/generated/current")
}

#[test]
fn dry_run_shows_changes_and_writes_nothing() {
    let home = set_up();
    fs::write(home.root.join(".zshrc"), "export A=1\n").unwrap();
    let before = rc_files(&home);

    home.cmd("rsmt")
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("--- /dev/null")
                .and(predicate::str::contains("ROSAMTA_TEST"))
                .and(predicate::str::contains(BEGIN)),
        );
    let out = home
        .cmd("rsmt")
        .args(["apply", "--dry-run", "--json"])
        .output()
        .unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(!doc["hook_changes"].as_array().unwrap().is_empty());

    assert_eq!(rc_files(&home), before);
    assert!(!home.root.join(".local/share/rosamta/generated").exists());
    assert!(!home.root.join(".local/state/rosamta/snapshots").exists());
}

#[test]
fn apply_twice_leaves_one_block_and_keeps_other_lines() {
    let home = set_up();
    fs::write(home.root.join(".zshrc"), "export A=1\n").unwrap();
    fs::write(home.root.join(".bashrc"), "alias x='y'\n").unwrap();
    home.cmd("rsmt")
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("Applied "));
    let first = rc_files(&home);

    home.cmd("rsmt")
        .arg("apply")
        .assert()
        .success()
        .stdout("Already up to date\n");
    assert_eq!(rc_files(&home), first);
    for (rel, content) in first {
        let text = String::from_utf8(content.unwrap_or_default()).unwrap();
        assert!(text.matches(BEGIN).count() <= 1, "{rel}");
    }
    if has("zsh") {
        assert_eq!(read(&home, ".zshrc").matches(BEGIN).count(), 1);
        assert_eq!(strip_block(&read(&home, ".zshrc")), "export A=1\n");
    }
    if has("bash") {
        assert_eq!(strip_block(&read(&home, ".bashrc")), "alias x='y'\n");
    }
}

#[test]
fn zsh_hooks_follow_zdotdir() {
    if !has("zsh") {
        return;
    }
    // Set in ~/.zshenv, the usual way.
    let home = set_up();
    fs::write(home.root.join(".zshenv"), "ZDOTDIR=\"$HOME/.config/zsh\"\n").unwrap();
    apply(&home);
    assert!(read(&home, ".zshenv").contains(BEGIN));
    assert!(read(&home, ".config/zsh/.zshrc").contains(BEGIN));
    assert!(!home.root.join(".zshrc").exists());
    let out = home.shell(&["zsh".into(), "-i".into(), "-c".into(), "alias ll".into()]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "ll='ls -l'\n");

    // Set in the environment before zsh starts.
    let home = set_up();
    let zdotdir = home.root.join("zdot");
    fs::create_dir_all(&zdotdir).unwrap();
    home.cmd("rsmt")
        .arg("apply")
        .env("ZDOTDIR", &zdotdir)
        .assert()
        .success();
    assert!(read(&home, "zdot/.zshenv").contains(BEGIN));
    assert!(read(&home, "zdot/.zshrc").contains(BEGIN));
    assert!(!home.root.join(".zshenv").exists());

    // Exported in ~/.zshenv, so rsmt inherits it from the zsh it runs in;
    // a new zsh still reads ~/.zshenv first.
    let home = set_up();
    fs::write(
        home.root.join(".zshenv"),
        "export ZDOTDIR=\"$HOME/.config/zsh\"\n",
    )
    .unwrap();
    home.cmd("rsmt")
        .arg("apply")
        .env("ZDOTDIR", home.root.join(".config/zsh"))
        .assert()
        .success();
    assert!(read(&home, ".zshenv").contains(BEGIN));
    assert!(!home.root.join(".config/zsh/.zshenv").exists());
    let out = home.shell(&["zsh".into(), "-c".into(), "echo \"$ROSAMTA_TEST\"".into()]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("it's {}\n", home.root.display())
    );
}

#[test]
fn bash_hooks_the_profile_file_login_bash_reads() {
    if !has("bash") {
        return;
    }
    // No bash profile yet: a new ~/.bash_profile must keep ~/.profile working.
    let home = set_up();
    fs::write(home.root.join(".profile"), "export FROM_PROFILE=1\n").unwrap();
    apply(&home);
    assert!(read(&home, ".bash_profile").starts_with("[ -r \"$HOME/.profile\" ]"));
    let out = home.shell(&[
        "bash".into(),
        "-l".into(),
        "-i".into(),
        "-c".into(),
        "echo \"$FROM_PROFILE|$ROSAMTA_TEST\"; alias ll".into(),
    ]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("1|it's {}\nalias ll='ls -l'\n", home.root.display())
    );

    // ~/.bash_login is what login bash reads when there is no ~/.bash_profile.
    let home = set_up();
    fs::write(home.root.join(".bash_login"), "export FROM_LOGIN=1\n").unwrap();
    apply(&home);
    assert!(read(&home, ".bash_login").contains(BEGIN));
    assert!(!home.root.join(".bash_profile").exists());
}

#[test]
fn unhook_removes_one_shell_until_hooked_again() {
    if !has("bash") {
        return;
    }
    let home = set_up();
    apply(&home);
    home.cmd("rsmt")
        .args(["unhook", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Unhooked"));
    assert!(!read(&home, ".bashrc").contains(BEGIN));
    assert!(!read(&home, ".bash_profile").contains(BEGIN));
    if has("zsh") {
        // HOME or ~/.config/zsh, whichever this zsh reads (Alpine picks by itself).
        assert!(
            [".zshrc", ".config/zsh/.zshrc"]
                .iter()
                .any(|rel| read(&home, rel).contains(BEGIN))
        );
    }

    home.write("modules/more.toml", "[env]\nMORE = \"1\"\n");
    apply(&home);
    assert!(!read(&home, ".bashrc").contains(BEGIN));

    home.cmd("rsmt").args(["hook", "bash"]).assert().success();
    assert!(read(&home, ".bashrc").contains(BEGIN));
}

#[test]
fn snapshots_are_local_and_tag_the_original() {
    let home = set_up();
    apply(&home);
    let snapshots = home.root.join(".local/state/rosamta/snapshots");
    let git = |args: &[&str]| {
        let mut cmd = assert_cmd::Command::new("git");
        git_env(&mut cmd);
        let out = cmd.arg("-C").arg(&snapshots).args(args).output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(git(&["remote"]), "");
    assert_eq!(git(&["tag", "--list"]), "original\n");
    assert!(git(&["log", "--format=%s"]).contains("install hooks"));
}

#[test]
fn restore_original_puts_back_every_byte() {
    let home = set_up();
    fs::write(
        home.root.join(".zshrc"),
        "export A=1\r\n# no newline at end",
    )
    .unwrap();
    fs::write(home.root.join(".bashrc"), "alias x='y'\n").unwrap();
    let before = rc_files(&home);
    apply(&home);
    assert_ne!(rc_files(&home), before);

    home.cmd("rsmt")
        .args(["restore", "--original"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--yes"));
    home.cmd("rsmt")
        .args(["restore", "--original", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Restored"));
    assert_eq!(rc_files(&home), before);

    home.cmd("rsmt")
        .args(["restore", "--original", "--yes"])
        .assert()
        .success()
        .stdout("Nothing to restore\n");
    // Restoring also stops apply from hooking the shells again.
    home.write("modules/more.toml", "[env]\nMORE = \"1\"\n");
    apply(&home);
    assert_eq!(rc_files(&home), before);
}

#[test]
fn a_module_that_prints_errors_is_rolled_back() {
    let home = set_up();
    apply(&home);
    let before = fs::read_link(current(&home)).unwrap();

    let noisy = "echo noisy-startup >&2";
    home.write(
        "modules/noisy.toml",
        &format!("[raw]\nsh = '{noisy}'\nbash = '{noisy}'\nzsh = '{noisy}'\nfish = '{noisy}'\n"),
    );
    home.cmd("rsmt").arg("apply").assert().code(1).stderr(
        predicate::str::contains("module noisy").and(predicate::str::contains("noisy-startup")),
    );
    assert_eq!(fs::read_link(current(&home)).unwrap(), before);
}

#[test]
fn a_login_snippet_that_prints_errors_is_rolled_back() {
    if !has("zsh") {
        return;
    }
    let home = set_up();
    apply(&home);
    let before = fs::read_link(current(&home)).unwrap();
    home.write(
        "modules/brew.toml",
        "phase = \"login\"\n[raw]\nzsh = \"echo login-noise >&2\"\n",
    );
    home.cmd("rsmt").arg("apply").assert().code(1).stderr(
        predicate::str::contains("module brew").and(predicate::str::contains("login-noise")),
    );
    assert_eq!(fs::read_link(current(&home)).unwrap(), before);
}

/// The shell already ran past the smoke test's deadline before this apply,
/// and what its rc file started keeps the output open.
#[test]
fn a_shell_that_was_already_slow_does_not_block_apply() {
    if !has("zsh") {
        return;
    }
    let home = set_up();
    apply(&home);
    for rel in [".zshrc", ".config/zsh/.zshrc"] {
        let text = read(&home, rel);
        if text.contains(BEGIN) {
            fs::write(home.root.join(rel), format!("{text}sleep 120\n")).unwrap();
        }
    }
    home.write("modules/more.toml", "[env]\nMORE = \"1\"\n");
    home.cmd("rsmt")
        .arg("apply")
        .timeout(Duration::from_secs(60))
        .assert()
        .success();
}

#[test]
fn a_shell_below_the_minimum_version_is_skipped_with_a_warning() {
    use std::os::unix::fs::PermissionsExt;
    let home = set_up();
    let fake = home.root.join("fake-bin");
    fs::create_dir_all(&fake).unwrap();
    let zsh = fake.join("zsh");
    fs::write(&zsh, "#!/bin/sh\necho 5.0\n").unwrap();
    fs::set_permissions(&zsh, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", fake.display(), std::env::var("PATH").unwrap());
    home.cmd("rsmt")
        .arg("apply")
        .env("PATH", path)
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "warning: zsh 5.0 is older than 5.8; upgrade it to load rosamta",
        ));
    for rel in [".zshenv", ".zshrc", ".config/zsh/.zshrc"] {
        assert!(!home.root.join(rel).exists(), "{rel}");
    }
}

#[test]
fn login_zsh_puts_module_path_before_system_path() {
    if !has("zsh") {
        return;
    }
    let home = set_up();
    apply(&home);
    let out = home.shell(&[
        "zsh".into(),
        "-l".into(),
        "-c".into(),
        "print -r -- $PATH".into(),
    ]);
    let path = String::from_utf8(out.stdout).unwrap();
    let entries: Vec<&str> = path.trim().split(':').collect();
    let mine = home.root.join("mybin").display().to_string();
    let position = |entry: &str| entries.iter().position(|seen| *seen == entry);
    let later = home.root.join("later").display().to_string();
    assert!(
        position(&mine).is_some() && position(&later).is_some(),
        "{path}"
    );
    if let Some(system) = position("/usr/bin") {
        assert!(position(&mine).unwrap() < system, "{path}");
    }
}

#[test]
fn shells_start_cleanly_without_the_generated_dir() {
    let home = set_up();
    let baseline: Vec<_> = sessions("exit", "exit")
        .iter()
        .map(|argv| home.shell(argv).stderr)
        .collect();
    apply(&home);
    fs::remove_dir_all(home.root.join(".local/share/rosamta/generated")).unwrap();
    for (argv, expected) in sessions("exit", "exit").iter().zip(baseline) {
        let out = home.shell(argv);
        assert!(out.status.success(), "{argv:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&expected),
            "{argv:?}"
        );
    }
}

#[test]
#[ignore = "timing; CI runs it on its own on Linux"]
fn hooks_add_at_most_5ms_to_zsh_startup() {
    assert!(has("zsh"), "zsh is required for this measurement");
    let plain = Home::new();
    let hooked = Home::new();
    hooked.cmd("rsmt").arg("init").assert().success();
    apply(&hooked);
    // The same rc files exist on both sides, so only the hook differs.
    for rel in [".zshenv", ".zshrc"] {
        fs::write(plain.root.join(rel), "").unwrap();
    }
    let argv: Vec<String> = ["zsh", "-i", "-c", "exit"].map(String::from).to_vec();
    let time = |home: &Home| {
        let start = Instant::now();
        home.shell(&argv);
        start.elapsed()
    };
    // Interleaved, so the machine's drift hits both sides alike.
    let (mut without, mut with) = (Vec::new(), Vec::new());
    for _ in 0..30 {
        without.push(time(&plain));
        with.push(time(&hooked));
    }
    let median = |runs: &mut Vec<Duration>| {
        runs.sort();
        runs[runs.len() / 2]
    };
    let (without, with) = (median(&mut without), median(&mut with));
    println!("zsh -i -c exit median: {without:?} without hooks, {with:?} with");
    assert!(with.saturating_sub(without) <= Duration::from_millis(5));
}

#[test]
fn eject_keeps_every_shell_the_same_without_rosamta() {
    let home = set_up();
    apply(&home);
    let probes = sessions(
        "echo \"$ROSAMTA_TEST|$PATH\"; alias ll",
        "echo \"$ROSAMTA_TEST|$PATH\"; alias",
    );
    let before: Vec<String> = probes
        .iter()
        .map(|argv| String::from_utf8_lossy(&home.shell(argv).stdout).into_owned())
        .collect();
    for (argv, out) in probes.iter().zip(&before) {
        assert!(out.starts_with("it's "), "{argv:?}: {out}");
    }

    home.cmd("rsmt")
        .arg("eject")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--yes"));
    home.cmd("rsmt")
        .args(["eject", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Ejected"));
    for (rel, content) in rc_files(&home) {
        let text = String::from_utf8(content.unwrap_or_default()).unwrap();
        assert!(!text.contains(BEGIN), "{rel}");
        assert!(!text.contains("Generated by rosamta"), "{rel}");
    }
    // Nothing rosamta generated is needed any more.
    fs::remove_dir_all(home.root.join(".local/share/rosamta")).unwrap();
    for (argv, expected) in probes.iter().zip(before) {
        assert_eq!(
            String::from_utf8_lossy(&home.shell(argv).stdout),
            expected,
            "{argv:?}"
        );
    }
}

#[test]
fn a_start_marker_without_its_end_stops_apply() {
    let home = set_up();
    let text = format!("echo mine-1\n{BEGIN}\necho mine-2\n");
    fs::write(home.root.join(".profile"), &text).unwrap();
    home.cmd("rsmt")
        .arg("apply")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(".profile"));
    assert_eq!(read(&home, ".profile"), text);
}

#[test]
fn apply_deploys_dotfiles_after_a_dry_run() {
    let home = set_up();
    home.write("home/.config/ghostty/config", "theme = dark\n");
    home.write("home/.npmrc", "save-exact = true\n");
    home.write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".config/ghostty\"\n\n[[dotfile]]\npath = \".npmrc\"\ndeploy = \"copy\"\n",
    );
    let ghostty = home.root.join(".config/ghostty");
    let npmrc = home.root.join(".npmrc");
    home.cmd("rsmt")
        .args(["apply", "--dry-run"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains(format!("Linked {}", ghostty.display()))
                .and(predicate::str::contains("+save-exact = true")),
        );
    assert!(!ghostty.exists() && !npmrc.exists());

    home.cmd("rsmt")
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Copied {}",
            npmrc.display()
        )));
    assert_eq!(read(&home, ".config/ghostty/config"), "theme = dark\n");
    home.cmd("rsmt")
        .arg("apply")
        .assert()
        .success()
        .stdout("Already up to date\n");
}

#[test]
fn conflicts_wait_in_the_review_queue() {
    let home = set_up();
    home.write("home/.gitconfig", "[user]\nname = repo\n");
    home.write("dotfiles.toml", "[[dotfile]]\npath = \".gitconfig\"\n");
    fs::write(home.root.join(".gitconfig"), "[user]\nname = local\n").unwrap();
    let gitconfig = home.root.join(".gitconfig");
    home.cmd("rsmt")
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Not deployed {}: differs from the repo",
            gitconfig.display()
        )));
    let out = home.cmd("rsmt").arg("review").output().unwrap();
    let listed = String::from_utf8(out.stdout).unwrap();
    assert!(
        listed.contains("~/.gitconfig") && listed.contains("differs from the repo"),
        "{listed}"
    );
    let id = listed.split_whitespace().next().unwrap().to_string();
    home.cmd("rsmt")
        .args(["review", "discard", &id])
        .assert()
        .success()
        .stdout("Replaced ~/.gitconfig with the repo's version\n");
    assert_eq!(read(&home, ".gitconfig"), "[user]\nname = repo\n");
    home.cmd("rsmt")
        .args(["review", "discard", &id])
        .assert()
        .code(2);
}
