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
    }
}

#[test]
fn changed_paths_reach_the_right_handlers() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(dir.path());
    // Events report real paths; the temp dir may sit behind a symlink.
    let home = real(dir.path());
    let rc = [home.join(".zshrc")];
    let routes = |paths: &[&str]| {
        let paths: Vec<PathBuf> = paths.iter().map(|path| home.join(path)).collect();
        route(&ctx, &rc, &paths)
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
}
