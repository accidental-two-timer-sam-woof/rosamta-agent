//! `rsmt daemon run` against a throwaway HOME: it reacts to rc files, the
//! repo and host.toml without anyone running a command.

mod common;

use std::fs;
use std::path::Path;
use std::process::Child;
use std::time::{Duration, Instant};

use common::Home;
use predicates::prelude::*;

/// Kills the daemon when the test ends, passing or not.
struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(home: &Home) -> Daemon {
    let daemon = Daemon(home.spawn("rsmt", &["daemon", "run"]));
    wait_for(home, Duration::from_secs(20), || {
        log(home).contains("scanned rc files")
    });
    daemon
}

/// Poll until `done`; on timeout, show what the daemon logged.
fn wait_for(home: &Home, limit: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < limit,
            "timed out; daemon.log:\n{}",
            log(home)
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn log(home: &Home) -> String {
    fs::read_to_string(home.root.join(".local/state/rosamta/daemon.log")).unwrap_or_default()
}

fn queue(home: &Home) -> serde_json::Value {
    let text = fs::read_to_string(home.root.join(".local/state/rosamta/queue.json"))
        .unwrap_or_else(|_| "{\"items\":[]}".into());
    serde_json::from_str(&text).unwrap()
}

fn status(home: &Home) -> serde_json::Value {
    let out = home
        .cmd("rsmt")
        .args(["daemon", "status", "--json"])
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

/// A HOME with a repo and hooks for every installed shell. Adopting asks,
/// so the tests see the queue; the default adopts at once.
fn hooked() -> Home {
    let home = Home::new();
    home.write_host("test");
    append(&home.host_config(), "adopt = \"ask\"\n");
    home.cmd("rsmt").arg("init").assert().success();
    home.cmd("rsmt").arg("apply").assert().success();
    home
}

fn append(path: &Path, text: &str) {
    let old = fs::read_to_string(path).unwrap_or_default();
    fs::write(path, old + text).unwrap();
}

/// Replace one `key = value` line of host.toml.
fn set_host(home: &Home, key: &str, value: &str) {
    let text = fs::read_to_string(home.host_config()).unwrap();
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            if line.starts_with(&format!("{key} =")) {
                format!("{key} = {value}")
            } else {
                line.to_string()
            }
        })
        .collect();
    fs::write(home.host_config(), lines.join("\n") + "\n").unwrap();
}

#[test]
fn a_second_instance_exits_with_1() {
    let home = hooked();
    let _daemon = start(&home);
    let running = status(&home);
    assert_eq!(running["running"], true);
    assert_eq!(running["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(running["review"], 0);
    home.cmd("rsmt")
        .args(["daemon", "run"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("already running (pid"));
}

#[test]
fn the_state_directory_comes_from_the_login_shell() {
    let home = hooked();
    append(
        &home.root.join(".profile"),
        "export XDG_STATE_HOME=\"$HOME/state\"\n",
    );
    let state = home.root.join("state");
    let _daemon = Daemon(home.spawn("rsmt", &["daemon", "run"]));
    let log = || fs::read_to_string(state.join("rosamta/daemon.log")).unwrap_or_default();
    let start = Instant::now();
    while !log().contains("scanned rc files") {
        assert!(start.elapsed() < Duration::from_secs(20), "{}", log());
        std::thread::sleep(Duration::from_millis(200));
    }
    let running = home
        .cmd("rsmt")
        .args(["daemon", "status", "--json"])
        .env("XDG_STATE_HOME", &state)
        .output()
        .unwrap();
    let running: serde_json::Value = serde_json::from_slice(&running.stdout).unwrap();
    assert_eq!(running["running"], true);
    assert!(running["pid"].is_u64(), "{running}");
    home.cmd("rsmt")
        .args(["daemon", "run"])
        .env("SHELL", "/bin/sh")
        .timeout(Duration::from_secs(20))
        .assert()
        .code(1)
        .stderr(predicate::str::contains("already running (pid"));
}

#[test]
fn installer_lines_are_queued_and_left_in_place() {
    let home = hooked();
    let profile = home.root.join(".profile");
    let _daemon = start(&home);
    append(&profile, "export FROM_INSTALLER=1\n");
    wait_for(&home, Duration::from_secs(20), || {
        queue(&home)["items"].as_array().unwrap().len() == 1
    });
    assert!(
        fs::read_to_string(&profile)
            .unwrap()
            .contains("FROM_INSTALLER")
    );

    // Written to a new file and renamed over the old one.
    let tmp = home.root.join(".profile.tmp");
    fs::write(
        &tmp,
        fs::read_to_string(&profile).unwrap() + "\nexport RENAMED=1\n",
    )
    .unwrap();
    fs::rename(&tmp, &profile).unwrap();
    wait_for(&home, Duration::from_secs(20), || {
        queue(&home)["items"].as_array().unwrap().len() == 2
    });
    assert_eq!(status(&home)["review"], 2);
}

#[test]
fn adopting_does_not_trigger_another_scan() {
    let home = hooked();
    set_host(&home, "adopt", "\"auto\"");
    let _daemon = start(&home);
    append(&home.root.join(".profile"), "export AUTO=1\n");
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).contains("1 adopted")
    });
    assert!(
        !fs::read_to_string(home.root.join(".profile"))
            .unwrap()
            .contains("AUTO=1")
    );
    // Adoption rewrote .profile; that write is rosamta's own.
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(
        log(&home).matches("scanned rc files").count(),
        2,
        "{}",
        log(&home)
    );
}

#[test]
fn lines_a_failed_scan_missed_are_queued_once_the_repo_applies() {
    let home = hooked();
    let _daemon = start(&home);
    home.write("modules/broken.toml", "[env\n");
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).contains("notified:")
    });
    append(&home.root.join(".profile"), "export LATER=1\n");
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).contains("scan failed")
    });
    fs::remove_file(home.repo().join("modules/broken.toml")).unwrap();
    wait_for(&home, Duration::from_secs(20), || {
        queue(&home)["items"].as_array().unwrap().len() == 1
    });
}

/// Make pushes to `remote` take a few seconds; `pushing` appears when one starts.
fn slow_down(remote: &Path, pushing: &Path) {
    let hook = remote.join("hooks/pre-receive");
    fs::write(
        &hook,
        format!("#!/bin/sh\ntouch '{}'\nsleep 6\n", pushing.display()),
    )
    .unwrap();
    fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

#[test]
fn an_edit_during_a_slow_push_is_adopted_too() {
    let home = Home::new();
    // A host ID from the hostname can look like a secret and stop the push.
    home.write_host("test");
    append(&home.host_config(), "adopt = \"auto\"\n");
    home.cmd("rsmt").arg("init").assert().success();
    home.cmd("rsmt").arg("apply").assert().success();
    let remote = home.root.join("remote.git");
    let pushing = home.root.join("pushing");
    common::git(&home.root, &["init", "--quiet", "--bare", "remote.git"]);
    common::git(
        &home.repo(),
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    common::git(&home.repo(), &["push", "--quiet", "-u", "origin", "HEAD"]);
    slow_down(&remote, &pushing);
    let profile = home.root.join(".profile");
    let _daemon = start(&home);
    append(&profile, "export FIRST=1\n");
    wait_for(&home, Duration::from_secs(20), || pushing.exists());
    append(&profile, "export DURING_PUSH=1\n");
    wait_for(&home, Duration::from_secs(40), || {
        !fs::read_to_string(&profile)
            .unwrap()
            .contains("DURING_PUSH")
    });
}

#[test]
fn settings_and_repo_edits_take_effect_without_a_restart() {
    let home = hooked();
    let _daemon = start(&home);
    home.write("modules/greeting.toml", "[env]\nGREETING = \"hi\"\n");
    wait_for(&home, Duration::from_secs(20), || {
        let out = home.shell(&[
            "sh".into(),
            "-l".into(),
            "-c".into(),
            "printf %s \"$GREETING\"".into(),
        ]);
        out.stdout == b"hi"
    });

    set_host(&home, "adopt", "\"auto\"");
    std::thread::sleep(Duration::from_secs(3));
    append(&home.root.join(".profile"), "export AFTER_RELOAD=1\n");
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).contains("1 adopted")
    });
    assert_eq!(queue(&home)["items"].as_array().unwrap().len(), 0);
}

#[test]
fn a_replaced_executable_restarts_the_daemon() {
    let home = hooked();
    let bin = home.root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("rsmt");
    fs::copy(assert_cmd::cargo::cargo_bin("rsmt"), &exe).unwrap();
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(["daemon", "run"])
        .env("HOME", &home.root)
        .env("SHELL", "/bin/sh")
        .env("ROSAMTA_NO_KEYCHAIN", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for var in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME"] {
        cmd.env_remove(var);
    }
    let _daemon = Daemon(cmd.spawn().unwrap());
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).contains("scanned rc files")
    });
    let before = status(&home)["started_at"].as_u64().unwrap();

    // Like a package manager: a new file renamed into place.
    std::thread::sleep(Duration::from_secs(1));
    let next = bin.join("rsmt.new");
    fs::copy(&exe, &next).unwrap();
    fs::rename(&next, &exe).unwrap();
    wait_for(&home, Duration::from_secs(30), || {
        log(&home).contains("restarting for the new version")
            && status(&home)["started_at"]
                .as_u64()
                .is_some_and(|now| now > before)
    });
    assert_eq!(status(&home)["running"], true);
}

#[test]
fn changes_close_together_share_one_notification() {
    let home = hooked();
    let profile = home.root.join(".profile");
    let _daemon = start(&home);
    for name in ["ONE", "TWO", "THREE"] {
        append(&profile, &format!("\nexport {name}=1\n"));
        // Longer than the debounce, so each is its own scan.
        std::thread::sleep(Duration::from_secs(3));
    }
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).contains("notified:")
    });
    std::thread::sleep(Duration::from_secs(2));
    let log = log(&home);
    let notes: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("notified:"))
        .collect();
    assert_eq!(notes.len(), 1, "{log}");
    assert!(notes[0].contains('3'), "{log}");
}

#[test]
fn a_restart_does_not_announce_the_same_items_again() {
    let home = hooked();
    let daemon = start(&home);
    append(&home.root.join(".profile"), "export ONCE=1\n");
    wait_for(&home, Duration::from_secs(30), || {
        log(&home).contains("notified:")
    });
    drop(daemon);
    let _daemon = Daemon(home.spawn("rsmt", &["daemon", "run"]));
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).matches("scanned rc files").count() == 3
    });
    // Past the batch window, when a new item would be announced.
    std::thread::sleep(Duration::from_secs(15));
    assert_eq!(log(&home).matches("notified:").count(), 1, "{}", log(&home));
}

#[test]
fn a_problem_is_notified_once_until_it_clears() {
    let home = hooked();
    let _daemon = start(&home);
    home.write("modules/broken.toml", "[env\n");
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).contains("notified:")
    });
    // Another edit while the same file is still broken.
    home.write("modules/other.toml", "[env]\nOTHER = \"1\"\n");
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(log(&home).matches("notified:").count(), 1, "{}", log(&home));

    fs::remove_file(home.repo().join("modules/broken.toml")).unwrap();
    std::thread::sleep(Duration::from_secs(4));
    home.write("modules/broken.toml", "[env\n");
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).matches("notified:").count() == 2
    });
}

#[test]
fn before_init_the_daemon_waits() {
    let home = Home::new();
    let _daemon = Daemon(home.spawn("rsmt", &["daemon", "run"]));
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).contains("waiting for `rsmt init`")
    });
    home.cmd("rsmt").arg("init").assert().success();
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).contains("scanned rc files")
    });
    assert_eq!(status(&home)["running"], true);
}

#[test]
fn an_invalid_host_toml_is_waited_out() {
    let home = hooked();
    let valid = fs::read_to_string(home.host_config()).unwrap();
    fs::write(home.host_config(), "adopt = [\n").unwrap();
    let mut daemon = Daemon(home.spawn("rsmt", &["daemon", "run"]));
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).contains("host.toml")
    });
    std::thread::sleep(Duration::from_secs(1));
    assert!(daemon.0.try_wait().unwrap().is_none(), "{}", log(&home));
    fs::write(home.host_config(), valid).unwrap();
    wait_for(&home, Duration::from_secs(20), || {
        log(&home).contains("scanned rc files")
    });
}

/// A HOME whose `~/.config/app/settings.json` is a copied dotfile.
fn with_copied_dotfile(adopt: &str) -> (Home, std::path::PathBuf) {
    let home = hooked();
    set_host(&home, "adopt", adopt);
    let target = home.root.join(".config/app/settings.json");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "a\n").unwrap();
    home.cmd("rsmt")
        .args(["add", "--copy"])
        .arg(&target)
        .assert()
        .success();
    (home, target)
}

/// What an app saving its settings does: write a new file, rename it over.
fn save_as_app(target: &Path, text: &str) {
    let tmp = target.with_extension("tmp");
    fs::write(&tmp, text).unwrap();
    fs::rename(&tmp, target).unwrap();
}

#[test]
fn an_app_rewriting_a_copied_dotfile_updates_the_repo() {
    let (home, target) = with_copied_dotfile("\"auto\"");
    let source = home.repo().join("home/.config/app/settings.json");
    let _daemon = start(&home);
    save_as_app(&target, "b\n");
    wait_for(&home, Duration::from_secs(20), || {
        fs::read_to_string(&source).unwrap() == "b\n"
    });
    assert_eq!(fs::read_to_string(&target).unwrap(), "b\n");
}

#[test]
fn a_dotfile_changed_here_is_queued_and_notified() {
    let (home, target) = with_copied_dotfile("\"ask\"");
    let _daemon = start(&home);
    save_as_app(&target, "b\n");
    wait_for(&home, Duration::from_secs(30), || {
        // The text itself follows the system language.
        log(&home).contains("notified: ~/.config/app/settings.json")
    });
    let items = queue(&home)["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["dotfile"]["reason"], "changed");
    assert_eq!(
        fs::read_to_string(home.repo().join("home/.config/app/settings.json")).unwrap(),
        "a\n"
    );
}
