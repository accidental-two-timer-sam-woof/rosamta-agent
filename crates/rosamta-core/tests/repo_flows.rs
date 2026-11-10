//! Repo and host flows against real git, in throwaway directories.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use rosamta_core::config::{HostConfig, Module, RepoManifest};
use rosamta_core::paths::Paths;
use rosamta_core::{host, repo};

/// Keep git away from the developer's config (signing, hooks, identity).
fn isolate_git() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let vars = [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_AUTHOR_NAME", "test"),
            ("GIT_AUTHOR_EMAIL", "test@example.com"),
            ("GIT_COMMITTER_NAME", "test"),
            ("GIT_COMMITTER_EMAIL", "test@example.com"),
        ];
        for (key, value) in vars {
            // SAFETY: set once, before any test in this binary spawns git.
            unsafe { std::env::set_var(key, value) };
        }
    });
}

struct Home {
    _dir: tempfile::TempDir,
    root: PathBuf,
    paths: Paths,
}

fn home() -> Home {
    isolate_git();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let paths = Paths::from_vars(|name| (name == "HOME").then(|| OsString::from(&root))).unwrap();
    Home {
        _dir: dir,
        root,
        paths,
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn last_commit(dir: &Path) -> String {
    git(dir, &["log", "-1", "--format=%s"]).trim().to_string()
}

fn manifest(dir: &Path) -> RepoManifest {
    toml::from_str(&fs::read_to_string(dir.join("rosamta.toml")).unwrap()).unwrap()
}

fn write_host(home: &Home, id: &str) {
    let path = home.paths.host_config();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, toml::to_string(&HostConfig::new(id.into())).unwrap()).unwrap();
}

/// A bare repo holding the given files, standing in for a GitHub remote.
fn remote(files: &[(&str, &str)]) -> (tempfile::TempDir, String) {
    isolate_git();
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "--quiet"]);
    for (path, text) in files {
        let path = work.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    git(&work, &["add", "--all"]);
    git(&work, &["commit", "--quiet", "--message", "seed"]);
    let bare = dir.path().join("remote.git");
    git(
        dir.path(),
        &[
            "clone",
            "--quiet",
            "--bare",
            work.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let url = bare.to_string_lossy().into_owned();
    (dir, url)
}

#[test]
fn init_creates_repo_and_host() {
    let home = home();
    let outcome = repo::init_new(&home.paths).unwrap();
    assert!(outcome.host_created);
    let repo_dir = home.paths.repo();
    assert_eq!(
        manifest(&repo_dir),
        RepoManifest {
            format: 1,
            hosts: vec![outcome.host_id.clone()]
        }
    );
    assert_eq!(last_commit(&repo_dir), "chore(repo): initialize");
    assert_eq!(
        host::load(&home.paths.host_config()).unwrap().id,
        outcome.host_id
    );
    assert!(git(&repo_dir, &["status", "--porcelain"]).is_empty());
}

#[test]
fn init_refuses_existing_repo() {
    let home = home();
    repo::init_new(&home.paths).unwrap();
    let host_before = fs::read_to_string(home.paths.host_config()).unwrap();
    let head_before = git(&home.paths.repo(), &["rev-parse", "HEAD"]);

    assert!(repo::init_new(&home.paths).unwrap_err().is_usage());
    assert_eq!(
        fs::read_to_string(home.paths.host_config()).unwrap(),
        host_before
    );
    assert_eq!(git(&home.paths.repo(), &["rev-parse", "HEAD"]), head_before);
}

#[test]
fn init_adds_this_host_to_a_repo_without_host_toml() {
    let home = home();
    write_host(&home, "mbp");
    repo::init_new(&home.paths).unwrap();
    let repo_dir = home.paths.repo();
    fs::remove_file(home.paths.host_config()).unwrap();
    fs::write(repo_dir.join("notes.txt"), "hand edit\n").unwrap();

    let outcome = repo::init_new(&home.paths).unwrap();
    assert!(outcome.existing && outcome.host_created && !outcome.cloned);
    let id = host::load(&home.paths.host_config()).unwrap().id;
    assert_eq!(outcome.host_id, id);
    assert_eq!(manifest(&repo_dir).hosts, ["mbp".to_string(), id.clone()]);
    assert_eq!(
        last_commit(&repo_dir),
        format!("chore(repo): add host {id}")
    );
    // Only the host list is committed; the hand edit stays as it was.
    assert_eq!(git(&repo_dir, &["status", "--porcelain"]), "?? notes.txt\n");

    // With host.toml back, a second init refuses again.
    assert!(repo::init_new(&home.paths).unwrap_err().is_usage());
}

#[test]
fn init_finishes_after_an_interrupted_init() {
    let home = home();
    let repo_dir = home.paths.repo();
    fs::create_dir_all(&repo_dir).unwrap();
    git(&repo_dir, &["init", "--quiet"]);

    let outcome = repo::init_new(&home.paths).unwrap();
    assert!(!outcome.existing);
    assert_eq!(manifest(&repo_dir).hosts, [outcome.host_id]);
    assert_eq!(last_commit(&repo_dir), "chore(repo): initialize");
}

#[test]
fn existing_host_id_survives_hostname_changes() {
    let home = home();
    write_host(&home, "mbp");
    // Whatever this machine's hostname is now, the stored ID wins.
    let outcome = repo::init_new(&home.paths).unwrap();
    assert_eq!(outcome.host_id, "mbp");
    assert!(!outcome.host_created);
}

#[test]
fn join_registers_host_and_leaves_rc_files() {
    let (_remote, url) = remote(&[("rosamta.toml", "format = 1\nhosts = [\"mbp\"]\n")]);
    let home = home();
    fs::write(home.root.join(".zshrc"), "export A=1\n").unwrap();
    write_host(&home, "lab-server");

    let outcome = repo::join(&home.paths, &url).unwrap();
    assert!(outcome.cloned);
    let repo_dir = home.paths.repo();
    assert_eq!(manifest(&repo_dir).hosts, ["mbp", "lab-server"]);
    assert_eq!(last_commit(&repo_dir), "chore(repo): add host lab-server");
    assert_eq!(
        fs::read_to_string(home.root.join(".zshrc")).unwrap(),
        "export A=1\n"
    );
    let entries: Vec<_> = fs::read_dir(&home.root)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(entries.len(), 3, "{entries:?}"); // .zshrc, .config, .local
}

#[test]
fn join_picks_a_free_host_id() {
    let generated = host::generate_id();
    let manifest_text = format!("format = 1\nhosts = [\"{generated}\", \"{generated}-2\"]\n");
    let (_remote, url) = remote(&[("rosamta.toml", &manifest_text)]);
    let home = home();

    let outcome = repo::join(&home.paths, &url).unwrap();
    let id = format!("{generated}-3");
    assert_eq!(outcome.host_id, id);
    assert_eq!(outcome.host_id_taken.as_deref(), Some(generated.as_str()));
    assert_eq!(host::load(&home.paths.host_config()).unwrap().id, id);
    assert_eq!(
        manifest(&home.paths.repo()).hosts,
        [generated.clone(), format!("{generated}-2"), id]
    );
}

#[test]
fn join_keeps_an_existing_host_id() {
    let (_remote, url) = remote(&[("rosamta.toml", "format = 1\nhosts = [\"mbp\"]\n")]);
    let home = home();
    write_host(&home, "mbp");
    let outcome = repo::join(&home.paths, &url).unwrap();
    assert_eq!(
        (outcome.host_id.as_str(), outcome.host_id_taken),
        ("mbp", None)
    );
    assert_eq!(manifest(&home.paths.repo()).hosts, ["mbp"]);
}

#[test]
fn join_refuses_newer_format_and_cleans_up() {
    let (_remote, url) = remote(&[("rosamta.toml", "format = 99\n")]);
    let home = home();
    assert!(repo::join(&home.paths, &url).unwrap_err().is_usage());
    assert!(!home.paths.repo().exists());
}

#[test]
fn join_refuses_non_rosamta_repo() {
    let (_remote, url) = remote(&[("README.md", "hi\n")]);
    let home = home();
    assert!(repo::join(&home.paths, &url).unwrap_err().is_usage());
    assert!(!home.paths.repo().exists());
}

#[test]
fn older_format_is_migrated_in_one_commit() {
    fn add_marker(dir: &Path) -> rosamta_core::Result<()> {
        fs::write(dir.join("migrated-from-1"), "").unwrap();
        Ok(())
    }
    let migrations = [repo::Migration {
        from: 1,
        run: add_marker,
    }];
    let home = home();
    repo::init_new(&home.paths).unwrap();
    let repo_dir = home.paths.repo();

    assert_eq!(
        repo::migrate(&repo_dir, 2, &migrations).unwrap(),
        Some((1, 2))
    );
    assert_eq!(manifest(&repo_dir).format, 2);
    assert!(repo_dir.join("migrated-from-1").exists());
    assert_eq!(last_commit(&repo_dir), "chore(repo): migrate format 1 to 2");
    assert_eq!(repo::migrate(&repo_dir, 2, &migrations).unwrap(), None);
}

#[test]
fn missing_migration_step_is_an_error() {
    let home = home();
    repo::init_new(&home.paths).unwrap();
    assert!(repo::migrate(&home.paths.repo(), 3, &[]).is_err());
}

#[test]
fn rename_updates_repo_in_one_commit() {
    let home = home();
    write_host(&home, "mbp");
    repo::init_new(&home.paths).unwrap();
    let repo_dir = home.paths.repo();
    fs::create_dir_all(repo_dir.join("modules/adopted")).unwrap();
    fs::write(
        repo_dir.join("modules/a.toml"),
        "# mine\n[when]\nhosts = [\"mbp\", \"lab\"] # two hosts\n",
    )
    .unwrap();
    fs::write(
        repo_dir.join("modules/adopted/b.toml"),
        "when = { hosts = [\"mbp\"] }\n",
    )
    .unwrap();
    fs::write(repo_dir.join("modules/c.toml"), "phase = \"env\"\n").unwrap();
    fs::write(
        repo_dir.join("dotfiles.toml"),
        "[[dotfile]]\npath = \".vimrc\"\n\n[[dotfile]]\npath = \".gitconfig\"\nwhen = { hosts = [\"mbp\"] }\n",
    )
    .unwrap();
    git(&repo_dir, &["add", "--all"]);
    git(&repo_dir, &["commit", "--quiet", "--message", "modules"]);

    host::rename(&home.paths, "work-mbp").unwrap();

    assert_eq!(
        fs::read_to_string(repo_dir.join("dotfiles.toml")).unwrap(),
        "[[dotfile]]\npath = \".vimrc\"\n\n[[dotfile]]\npath = \".gitconfig\"\nwhen = { hosts = [\"work-mbp\"] }\n"
    );

    assert_eq!(manifest(&repo_dir).hosts, ["work-mbp"]);
    let a = fs::read_to_string(repo_dir.join("modules/a.toml")).unwrap();
    assert_eq!(
        a,
        "# mine\n[when]\nhosts = [\"work-mbp\", \"lab\"] # two hosts\n"
    );
    let b: Module =
        toml::from_str(&fs::read_to_string(repo_dir.join("modules/adopted/b.toml")).unwrap())
            .unwrap();
    assert_eq!(b.when.hosts, ["work-mbp"]);
    assert_eq!(
        fs::read_to_string(repo_dir.join("modules/c.toml")).unwrap(),
        "phase = \"env\"\n"
    );
    assert_eq!(
        last_commit(&repo_dir),
        "chore(repo): rename host mbp to work-mbp"
    );
    assert_eq!(
        git(&repo_dir, &["show", "--name-only", "--format=", "HEAD"])
            .lines()
            .count(),
        4
    );
    assert_eq!(
        host::load(&home.paths.host_config()).unwrap().id,
        "work-mbp"
    );
}

#[test]
fn rename_updates_local_modules_and_commits_only_its_files() {
    let home = home();
    write_host(&home, "mbp");
    repo::init_new(&home.paths).unwrap();
    let repo_dir = home.paths.repo();
    fs::write(repo_dir.join("secrets.txt"), "token\n").unwrap();
    let local = home.paths.local_modules().join("token.toml");
    fs::create_dir_all(local.parent().unwrap()).unwrap();
    fs::write(&local, "[when]\nhosts = [\"mbp\"]\n").unwrap();

    host::rename(&home.paths, "work-mbp").unwrap();

    assert_eq!(
        fs::read_to_string(&local).unwrap(),
        "[when]\nhosts = [\"work-mbp\"]\n"
    );
    assert_eq!(
        git(&repo_dir, &["show", "--name-only", "--format=", "HEAD"]),
        "rosamta.toml\n"
    );
    assert_eq!(
        git(&repo_dir, &["status", "--porcelain"]),
        "?? secrets.txt\n"
    );
}

#[test]
fn rename_with_a_broken_file_changes_nothing() {
    let home = home();
    write_host(&home, "mbp");
    repo::init_new(&home.paths).unwrap();
    let repo_dir = home.paths.repo();
    fs::create_dir_all(repo_dir.join("modules")).unwrap();
    fs::write(
        repo_dir.join("modules/a.toml"),
        "[when]\nhosts = [\"mbp\"]\n",
    )
    .unwrap();
    fs::write(repo_dir.join("modules/b.toml"), "[when\n").unwrap();
    let head = git(&repo_dir, &["rev-parse", "HEAD"]);

    assert!(
        host::rename(&home.paths, "work-mbp")
            .unwrap_err()
            .is_usage()
    );
    assert_eq!(manifest(&repo_dir).hosts, ["mbp"]);
    assert_eq!(
        fs::read_to_string(repo_dir.join("modules/a.toml")).unwrap(),
        "[when]\nhosts = [\"mbp\"]\n"
    );
    assert_eq!(host::load(&home.paths.host_config()).unwrap().id, "mbp");
    assert_eq!(git(&repo_dir, &["rev-parse", "HEAD"]), head);
}

#[test]
fn rename_to_another_hosts_id_is_refused() {
    let home = home();
    write_host(&home, "mbp");
    repo::init_new(&home.paths).unwrap();
    repo::register_host(&home.paths.repo(), "lab").unwrap();

    assert!(host::rename(&home.paths, "lab").unwrap_err().is_usage());
    assert_eq!(manifest(&home.paths.repo()).hosts, ["mbp", "lab"]);
    assert_eq!(host::load(&home.paths.host_config()).unwrap().id, "mbp");
}

#[test]
fn rename_keeps_the_comment_on_the_id() {
    let home = home();
    let path = home.paths.host_config();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "id = \"mbp\" # my laptop\n").unwrap();
    host::rename(&home.paths, "work-mbp").unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "id = \"work-mbp\" # my laptop\n"
    );
}

#[test]
fn rename_rejects_bad_id() {
    let home = home();
    write_host(&home, "mbp");
    assert!(
        host::rename(&home.paths, "Work MBP")
            .unwrap_err()
            .is_usage()
    );
    assert_eq!(host::load(&home.paths.host_config()).unwrap().id, "mbp");
}

#[test]
fn tags_stay_sorted_and_unique() {
    let home = home();
    write_host(&home, "mbp");
    assert!(host::add_tag(&home.paths, "personal").unwrap());
    assert!(host::add_tag(&home.paths, "lab").unwrap());
    assert!(!host::add_tag(&home.paths, "lab").unwrap());
    assert_eq!(
        host::load(&home.paths.host_config()).unwrap().tags,
        ["lab", "personal"]
    );
    assert!(host::remove_tag(&home.paths, "lab").unwrap());
    assert_eq!(
        host::load(&home.paths.host_config()).unwrap().tags,
        ["personal"]
    );
}
