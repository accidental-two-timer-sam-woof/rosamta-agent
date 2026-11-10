//! Deploying dotfiles into a throwaway home.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use rosamta_core::apply::Context;
use rosamta_core::dotfiles::{self, Action};
use rosamta_core::paths::Paths;

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
    ctx: Context,
}

impl Home {
    fn new(adopt: &str) -> Self {
        isolate_git();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let ctx = Context {
            paths: Paths::from_vars(|name| (name == "HOME").then(|| OsString::from(&root)))
                .unwrap(),
            home: root,
            path_var: String::new(),
        };
        let home = Self { _dir: dir, ctx };
        home.repo_write("rosamta.toml", "format = 1\nhosts = [\"mbp\"]\n");
        let host = home.ctx.paths.host_config();
        fs::create_dir_all(host.parent().unwrap()).unwrap();
        fs::write(host, format!("id = \"mbp\"\nadopt = \"{adopt}\"\n")).unwrap();
        git(&home.repo(), &["init", "--quiet"]);
        home
    }

    fn repo(&self) -> PathBuf {
        self.ctx.paths.repo()
    }

    fn repo_write(&self, rel: &str, text: &str) {
        write(&self.repo().join(rel), text);
    }

    fn at(&self, rel: &str) -> PathBuf {
        self.ctx.home.join(rel)
    }

    fn deploy(&self) -> Vec<(String, Action)> {
        dotfiles::deploy(&self.ctx)
            .unwrap()
            .into_iter()
            .map(|step| (step.path.display().to_string(), step.action))
            .collect()
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn git(dir: &Path, args: &[&str]) {
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
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn one(path: &str, action: Action) -> Vec<(String, Action)> {
    vec![(path.to_string(), action)]
}

#[test]
fn links_new_and_identical_targets() {
    let home = Home::new("ask");
    home.repo_write("home/.config/nvim/init.lua", "vim.o.number = true\n");
    home.repo_write("home/.gitconfig", "[user]\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".config/nvim\"\n\n[[dotfile]]\npath = \".gitconfig\"\n",
    );
    write(&home.at(".gitconfig"), "[user]\n");
    assert_eq!(
        home.deploy(),
        [
            (".config/nvim".to_string(), Action::Linked),
            (".gitconfig".to_string(), Action::Linked),
        ]
    );
    assert_eq!(
        fs::read_link(home.at(".config/nvim")).unwrap(),
        home.repo().join("home/.config/nvim")
    );
    assert_eq!(
        read(&home.at(".config/nvim/init.lua")),
        "vim.o.number = true\n"
    );
    assert!(
        home.deploy()
            .iter()
            .all(|(_, action)| *action == Action::Unchanged)
    );
}

#[test]
fn differing_target_is_a_conflict() {
    let home = Home::new("auto");
    home.repo_write("home/.gitconfig", "[user]\nname = repo\n");
    home.repo_write("dotfiles.toml", "[[dotfile]]\npath = \".gitconfig\"\n");
    write(&home.at(".gitconfig"), "[user]\nname = local\n");
    assert_eq!(home.deploy(), one(".gitconfig", Action::Conflict));
    assert_eq!(read(&home.at(".gitconfig")), "[user]\nname = local\n");
}

#[test]
fn replaced_link_is_saved_and_relinked() {
    let home = Home::new("off");
    home.repo_write("home/.gitconfig", "[user]\n");
    home.repo_write("dotfiles.toml", "[[dotfile]]\npath = \".gitconfig\"\n");
    home.deploy();
    // An editor saving by rename.
    fs::remove_file(home.at(".gitconfig")).unwrap();
    write(&home.at(".gitconfig"), "[user]\nname = new\n");
    assert_eq!(home.deploy(), one(".gitconfig", Action::Saved));
    assert_eq!(
        read(&home.repo().join("home/.gitconfig")),
        "[user]\nname = new\n"
    );
    assert!(
        fs::symlink_metadata(home.at(".gitconfig"))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn copies_follow_the_repo_until_changed_here() {
    let home = Home::new("ask");
    home.repo_write("home/.ssh/config", "Host a\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".ssh/config\"\ndeploy = \"copy\"\npermissions = \"0600\"\n",
    );
    assert_eq!(home.deploy(), one(".ssh/config", Action::Copied));
    assert_eq!(mode(&home.at(".ssh/config")), 0o600);

    home.repo_write("home/.ssh/config", "Host b\n");
    assert_eq!(home.deploy(), one(".ssh/config", Action::Updated));
    assert_eq!(read(&home.at(".ssh/config")), "Host b\n");

    write(&home.at(".ssh/config"), "Host here\n");
    assert_eq!(home.deploy(), one(".ssh/config", Action::Changed));
    home.repo_write("home/.ssh/config", "Host c\n");
    assert_eq!(home.deploy(), one(".ssh/config", Action::Conflict));
    assert_eq!(read(&home.at(".ssh/config")), "Host here\n");
}

#[test]
fn auto_policy_saves_copies() {
    let home = Home::new("auto");
    home.repo_write("home/.npmrc", "a\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".npmrc\"\ndeploy = \"copy\"\n",
    );
    home.deploy();
    write(&home.at(".npmrc"), "b\n");
    assert_eq!(home.deploy(), one(".npmrc", Action::Saved));
    assert_eq!(read(&home.repo().join("home/.npmrc")), "b\n");
    assert_eq!(home.deploy(), one(".npmrc", Action::Unchanged));
}

#[test]
fn link_permissions_go_on_the_source() {
    let home = Home::new("ask");
    home.repo_write("home/.netrc", "machine x\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".netrc\"\npermissions = \"600\"\n",
    );
    home.deploy();
    assert_eq!(mode(&home.repo().join("home/.netrc")), 0o600);
}

#[test]
fn undeclared_links_become_files() {
    let home = Home::new("ask");
    home.repo_write("home/.config/ghostty/config", "theme = dark\n");
    home.repo_write("home/.vimrc", "set nu\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".config/ghostty\"\n\n[[dotfile]]\npath = \".vimrc\"\n",
    );
    home.deploy();
    git(&home.repo(), &["add", "-A"]);
    git(
        &home.repo(),
        &["commit", "--quiet", "--no-verify", "-m", "add"],
    );

    // Another host forgot both; the pull deleted one source.
    home.repo_write("dotfiles.toml", "");
    git(
        &home.repo(),
        &["rm", "--quiet", "-r", "home/.config/ghostty"],
    );
    git(
        &home.repo(),
        &["commit", "--quiet", "--no-verify", "-am", "forget"],
    );
    assert_eq!(
        home.deploy(),
        [
            (".config/ghostty".to_string(), Action::Released),
            (".vimrc".to_string(), Action::Released),
        ]
    );
    let ghostty = home.at(".config/ghostty");
    assert!(!fs::symlink_metadata(&ghostty).unwrap().is_symlink());
    assert_eq!(read(&ghostty.join("config")), "theme = dark\n");
    assert!(
        !fs::symlink_metadata(home.at(".vimrc"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(read(&home.at(".vimrc")), "set nu\n");
    assert!(home.deploy().is_empty());
}

#[test]
fn duplicates_and_templates_are_skipped() {
    let home = Home::new("ask");
    home.repo_write("home/.zprofile", "");
    home.repo_write("home/.gitconfig", "");
    home.repo_write(
        "dotfiles.toml",
        r#"
[[dotfile]]
path = ".zprofile"
when = { os = ["macos", "linux"] }

[[dotfile]]
path = ".zprofile"

[[dotfile]]
path = ".gitconfig"
deploy = "template"
"#,
    );
    assert_eq!(
        home.deploy(),
        [
            (".gitconfig".to_string(), Action::Skipped),
            (".zprofile".to_string(), Action::Skipped),
        ]
    );
    assert!(!home.at(".zprofile").exists());
}

#[test]
fn restore_original_removes_new_links() {
    let home = Home::new("ask");
    home.repo_write("home/.vimrc", "set nu\n");
    home.repo_write("dotfiles.toml", "[[dotfile]]\npath = \".vimrc\"\n");
    home.deploy();
    let snapshots = rosamta_core::snapshot::Snapshots::new(home.ctx.paths.state(), &home.ctx.home);
    let originals = snapshots.originals().unwrap();
    assert!(
        originals
            .iter()
            .any(|original| original.path == home.at(".vimrc") && original.content.is_none()),
        "{originals:?}"
    );
}

mod review {
    use super::*;
    use rosamta_core::adopt::{self, Scope};
    use rosamta_core::dotfiles::review::Reason;

    /// A repo `.gitconfig` and a different one already on this host.
    fn conflict() -> (Home, u32) {
        let home = Home::new("ask");
        home.repo_write("home/.gitconfig", "[user]\nname = repo\n");
        home.repo_write("dotfiles.toml", "[[dotfile]]\npath = \".gitconfig\"\n");
        git(&home.repo(), &["add", "-A"]);
        git(
            &home.repo(),
            &["commit", "--quiet", "--no-verify", "-m", "init"],
        );
        write(&home.at(".gitconfig"), "[user]\nname = local\n");
        home.deploy();
        let queue = adopt::queue(&home.ctx).unwrap();
        assert_eq!(queue.len(), 1, "{queue:?}");
        let dotfile = queue[0].dotfile.as_ref().unwrap();
        assert_eq!(
            (dotfile.path.as_path(), dotfile.reason),
            (Path::new(".gitconfig"), Reason::Conflict)
        );
        (home, queue[0].id)
    }

    fn log(home: &Home) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(home.repo())
            .args(["log", "--format=%s", "--name-only"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn listed_once_with_a_notice() {
        let (home, id) = conflict();
        home.deploy();
        assert_eq!(adopt::queue(&home.ctx).unwrap()[0].id, id);
        let notice = read(&adopt::notice_path(&home.ctx));
        assert_eq!(notice, "rosamta: 1 change to review. Run: rsmt review\n");
    }

    #[test]
    fn accept_keeps_a_host_variant() {
        let (home, id) = conflict();
        adopt::accept(&home.ctx, id, Scope::Host).unwrap();
        let variant = home.repo().join("hosts/mbp/.gitconfig");
        assert_eq!(read(&variant), "[user]\nname = local\n");
        assert_eq!(
            read(&home.repo().join("home/.gitconfig")),
            "[user]\nname = repo\n"
        );
        assert_eq!(fs::read_link(home.at(".gitconfig")).unwrap(), variant);
        let manifest = read(&home.repo().join("dotfiles.toml"));
        assert!(
            manifest.contains("source = \"hosts/mbp/.gitconfig\""),
            "{manifest}"
        );
        assert!(adopt::queue(&home.ctx).unwrap().is_empty());
        let log = log(&home);
        assert!(
            log.contains("hosts/mbp/.gitconfig") && log.contains("dotfiles.toml"),
            "{log}"
        );
    }

    #[test]
    fn accept_for_all_hosts_replaces_the_source() {
        let (home, id) = conflict();
        adopt::accept(&home.ctx, id, Scope::All).unwrap();
        assert_eq!(
            read(&home.repo().join("home/.gitconfig")),
            "[user]\nname = local\n"
        );
        assert!(!home.repo().join("hosts").exists());
        assert!(
            fs::symlink_metadata(home.at(".gitconfig"))
                .unwrap()
                .is_symlink()
        );
    }

    #[test]
    fn discard_takes_the_repo_version() {
        let (home, id) = conflict();
        rosamta_core::dotfiles::review::discard(&home.ctx, id).unwrap();
        assert_eq!(read(&home.at(".gitconfig")), "[user]\nname = repo\n");
        assert!(adopt::queue(&home.ctx).unwrap().is_empty());
        let out = Command::new("git")
            .arg("-C")
            .arg(home.ctx.paths.state().join("snapshots"))
            .args(["show", "HEAD:files/home/.gitconfig"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "[user]\nname = local\n"
        );
    }

    #[test]
    fn ignore_holds_until_a_side_changes() {
        let (home, id) = conflict();
        adopt::ignore(&home.ctx, id).unwrap();
        assert!(adopt::queue(&home.ctx).unwrap().is_empty());
        assert_eq!(home.deploy(), one(".gitconfig", Action::Skipped));
        write(&home.at(".gitconfig"), "[user]\nname = other\n");
        assert_eq!(home.deploy(), one(".gitconfig", Action::Conflict));
        assert_eq!(adopt::queue(&home.ctx).unwrap().len(), 1);
    }

    #[test]
    fn stale_items_are_refused() {
        let (home, id) = conflict();
        write(&home.at(".gitconfig"), "[user]\nname = newer\n");
        let err = adopt::accept(&home.ctx, id, Scope::Host).unwrap_err();
        assert!(
            err.to_string().contains("changed since it was listed"),
            "{err}"
        );
        assert_eq!(read(&home.at(".gitconfig")), "[user]\nname = newer\n");
    }

    #[test]
    fn secrets_stay_out() {
        let (home, id) = conflict();
        write(
            &home.at(".gitconfig"),
            "[github]\ntoken = ghp_0123456789abcdefghijklmnopqrstuvwxyzAB\n",
        );
        home.deploy();
        let id = adopt::queue(&home.ctx)
            .unwrap()
            .first()
            .map_or(id, |item| item.id);
        let err = adopt::accept(&home.ctx, id, Scope::All).unwrap_err();
        assert!(
            err.to_string().contains("looks like it contains a secret"),
            "{err}"
        );
        assert_eq!(
            read(&home.repo().join("home/.gitconfig")),
            "[user]\nname = repo\n"
        );
    }
}
