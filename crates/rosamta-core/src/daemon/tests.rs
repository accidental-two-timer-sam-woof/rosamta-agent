use std::ffi::OsString;

use super::*;
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
