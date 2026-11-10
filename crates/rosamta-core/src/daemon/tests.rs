use std::ffi::OsString;
use std::sync::{Arc, Mutex};

use super::*;
use crate::adopt::store::Store;
use crate::paths::Paths;

fn ctx(home: &Path) -> Context {
    Context {
        paths: Paths::from_vars(|name| (name == "HOME").then(|| OsString::from(home))).unwrap(),
        home: home.to_path_buf(),
        path_var: String::new(),
        interactive: false,
        session: Default::default(),
    }
}

#[test]
fn changed_paths_reach_the_right_handlers() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(dir.path());
    // Events report real paths; the temp dir may sit behind a symlink.
    let home = real(dir.path());
    let rc = [home.join(".zshrc")];
    let dotfiles = [home.join(".gitconfig"), home.join(".config/ghostty")];
    let routes = |paths: &[&str]| {
        let paths: Vec<PathBuf> = paths.iter().map(|path| home.join(path)).collect();
        route(&ctx, &rc, &dotfiles, &paths)
    };
    let repo = ".local/share/rosamta/repo";
    assert_eq!(
        routes(&[".zshrc"]),
        Routes {
            rc: true,
            ..Routes::default()
        }
    );
    // Shell history, written after every command, is not an rc file.
    assert_eq!(routes(&[".zsh_history"]), Routes::default());
    for path in [".gitconfig", ".config/ghostty/config"] {
        assert_eq!(
            routes(&[path]),
            Routes {
                dotfiles: true,
                ..Routes::default()
            }
        );
    }
    assert_eq!(routes(&[".gitconfig.lock"]), Routes::default());
    assert_eq!(
        routes(&[&format!("{repo}/modules/a.toml")]),
        Routes {
            repo: true,
            ..Routes::default()
        }
    );
    assert_eq!(routes(&[&format!("{repo}/.git/index")]), Routes::default());
    assert_eq!(
        routes(&[".config/rosamta/host.toml"]),
        Routes {
            host: true,
            ..Routes::default()
        }
    );
    assert_eq!(
        routes(&[".config/rosamta/local/modules/key.toml"]),
        Routes {
            repo: true,
            ..Routes::default()
        }
    );
}

#[test]
fn fingerprints_follow_content() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(".zshrc");
    std::fs::write(&file, "a\n").unwrap();
    let files = [file.clone()];
    let before = fingerprint(&files);
    std::fs::write(&file, "a\n").unwrap();
    assert_eq!(fingerprint(&files), before);
    std::fs::write(&file, "b\n").unwrap();
    assert_ne!(fingerprint(&files), before);
}

/// Records the text of each notification.
struct Shown(Arc<Mutex<Vec<String>>>);

impl Notifier for Shown {
    fn show(&self, note: &Note) {
        self.0.lock().unwrap().push(note.body.clone());
    }
}

fn daemon(home: &Path) -> (Daemon, Arc<Mutex<Vec<String>>>) {
    let shown = Arc::default();
    let notifier = Box::new(Shown(Arc::clone(&shown)));
    let mut daemon = Daemon::new(ctx(home), HostConfig::new("test".into()), notifier);
    daemon.lang = Lang::En;
    (daemon, shown)
}

#[test]
fn a_sync_after_waking_retries_soon_while_offline() {
    crate::git::tests::isolate();
    let dir = tempfile::tempdir().unwrap();
    let (mut daemon, _) = daemon(dir.path());
    crate::repo::init_new(&daemon.ctx.paths).unwrap();
    let missing = dir.path().join("remote.git");
    let repo = daemon.ctx.paths.repo();
    crate::git::run(
        &repo,
        &["remote", "add", "origin", missing.to_str().unwrap()],
    )
    .unwrap();
    let interval = Duration::from_secs(u64::from(daemon.host.pull_interval) * 60);
    // Back from sleep: the wall clock moved on, the monotonic one did not.
    daemon.clock.1 -= Duration::from_secs(600);
    for expected in [Duration::from_secs(30), Duration::from_secs(120), interval] {
        let before = Instant::now();
        daemon.tick();
        let delay = daemon.next_sync - before;
        assert!(
            delay >= expected && delay < expected + Duration::from_secs(5),
            "{delay:?} after waking, expected {expected:?}"
        );
        daemon.next_sync = Instant::now();
    }
}

#[test]
fn problems_say_whether_publishing_or_pulling_failed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut daemon, shown) = daemon(dir.path());
    let state = daemon.ctx.paths.state().to_path_buf();
    instance::save(&state, &instance::DaemonState::default()).unwrap();
    let attempt = |outcome, reason: &str| Some(sync::Attempt::new(outcome, Some(reason.into())));
    let publish = |outcome, reason| sync::SyncState {
        publish: attempt(outcome, reason),
        ..Default::default()
    };
    let pull = |outcome, reason| sync::SyncState {
        pull: attempt(outcome, reason),
        ..Default::default()
    };
    let cases = [
        (
            publish(Outcome::Blocked, "commit signing failed: no pinentry"),
            "Publishing is blocked: commit signing failed: no pinentry",
        ),
        (
            pull(Outcome::Blocked, "origin refused access: denied"),
            "Pulling is blocked: origin refused access: denied",
        ),
        (
            publish(Outcome::Failed, "fatal: the remote hung up"),
            "Publishing failed: fatal: the remote hung up",
        ),
        (
            pull(Outcome::Failed, "fatal: bad object"),
            "Pulling failed: fatal: bad object",
        ),
        (
            sync::SyncState {
                pull: Some(sync::Attempt {
                    applying: true,
                    ..sync::Attempt::new(Outcome::Failed, Some("new errors\n  sh: x".into()))
                }),
                ..Default::default()
            },
            "New settings were not applied: new errors",
        ),
    ];
    for (sync_state, expected) in cases {
        Store::new(&state).save("sync.json", &sync_state).unwrap();
        daemon.report_problems();
        assert_eq!(shown.lock().unwrap().last().unwrap(), expected);
    }

    // A pull stopped by edits that could not be published carries the
    // publish attempt: notified once.
    let blocked = attempt(Outcome::Blocked, "possible secret in .zshrc");
    let shared = sync::SyncState {
        publish: blocked.clone(),
        pull: blocked,
        ..Default::default()
    };
    Store::new(&state).save("sync.json", &shared).unwrap();
    let before = shown.lock().unwrap().len();
    daemon.report_problems();
    assert_eq!(
        shown.lock().unwrap()[before..],
        ["Publishing is blocked: possible secret in .zshrc"]
    );
}

#[test]
fn a_locked_key_is_notified_once() {
    crate::git::tests::isolate();
    let dir = tempfile::tempdir().unwrap();
    let (mut daemon, shown) = daemon(dir.path());
    crate::repo::init_new(&daemon.ctx.paths).unwrap();
    let repo = daemon.ctx.paths.repo();
    std::fs::write(repo.join(dotfiles::crypt::REPO_KEY), "sealed").unwrap();
    std::fs::create_dir(repo.join("home")).unwrap();
    std::fs::write(repo.join("home/.netrc"), "sealed").unwrap();
    std::fs::write(
        repo.join(dotfiles::MANIFEST),
        "[[dotfile]]\npath = \".netrc\"\ndeploy = \"copy\"\nencrypted = true\n",
    )
    .unwrap();
    instance::save(daemon.ctx.paths.state(), &instance::DaemonState::default()).unwrap();
    daemon.report_problems();
    daemon.report_problems();
    assert_eq!(
        *shown.lock().unwrap(),
        ["Encrypted dotfiles need the passphrase"]
    );
}

/// A host that applied once, with `.npmrc` rendered from `template`;
/// returns `host.toml` as file events name it.
fn with_template(daemon: &Daemon, template: &str) -> PathBuf {
    crate::repo::init_new(&daemon.ctx.paths).unwrap();
    let repo = daemon.ctx.paths.repo();
    std::fs::create_dir(repo.join("home")).unwrap();
    std::fs::write(repo.join("home/.npmrc"), template).unwrap();
    std::fs::write(
        repo.join(dotfiles::MANIFEST),
        "[[dotfile]]\npath = \".npmrc\"\ndeploy = \"template\"\n",
    )
    .unwrap();
    let host = daemon.ctx.paths.host_config();
    std::fs::create_dir_all(host.parent().unwrap()).unwrap();
    std::fs::write(&host, "id = \"test\"\n").unwrap();
    let generated = daemon.ctx.paths.generated();
    std::fs::create_dir_all(generated.join("v1")).unwrap();
    std::os::unix::fs::symlink("v1", generated.join(crate::generated::CURRENT)).unwrap();
    real(host.parent().unwrap()).join("host.toml")
}

#[test]
fn a_locked_tool_is_waited_for_not_unlocked() {
    use std::os::unix::fs::PermissionsExt;
    crate::git::tests::isolate();
    let dir = tempfile::tempdir().unwrap();
    let (mut daemon, shown) = daemon(dir.path());
    let host = with_template(&daemon, "pw={{ bitwarden(\"GitHub\") }}\n");
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(
        bin.join("rbw"),
        "#!/bin/sh\necho \"rbw $*\" >> \"$(dirname \"$0\")/calls\"\ncase \"$1\" in\n\
         config) echo \'{\"email\":\"me@example.com\"}\' ;;\nunlocked) exit 1 ;;\nesac\n",
    )
    .unwrap();
    std::fs::set_permissions(bin.join("rbw"), std::fs::Permissions::from_mode(0o755)).unwrap();
    daemon.ctx.path_var = bin.display().to_string();

    daemon.changed(std::slice::from_ref(&host));
    daemon.announce();
    assert_eq!(
        *shown.lock().unwrap(),
        ["~/.npmrc: waiting for rbw to unlock"]
    );
    let calls = std::fs::read_to_string(bin.join("calls")).unwrap();
    assert!(
        !calls.contains("rbw unlock\n") && !calls.contains("rbw get"),
        "{calls}"
    );
    assert!(!dir.path().join(".npmrc").exists());
}

#[test]
fn new_vars_in_host_toml_render_a_waiting_template() {
    crate::git::tests::isolate();
    let dir = tempfile::tempdir().unwrap();
    let (mut daemon, shown) = daemon(dir.path());
    let host = with_template(&daemon, "email={{ vars.email }}\n");

    daemon.changed(std::slice::from_ref(&host));
    daemon.announce();
    assert_eq!(
        *shown.lock().unwrap(),
        ["~/.npmrc: waiting for values of email"]
    );

    crate::host::set_var(&daemon.ctx.paths, "email", "a@example.com").unwrap();
    daemon.changed(&[host]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".npmrc")).unwrap(),
        "email=a@example.com\n"
    );
    assert!(adopt::queue(&daemon.ctx).unwrap().is_empty());
}
