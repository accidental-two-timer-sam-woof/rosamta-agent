//! Deploying dotfiles into a throwaway home.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use rosamta_core::apply::Context;
use rosamta_core::config::Deploy;
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
            interactive: false,
            session: Default::default(),
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
fn create_writes_only_what_is_missing() {
    let home = Home::new("ask");
    let path = ".config/foo/config.toml";
    let target = home.at(path);
    home.repo_write("home/.config/foo/config.toml", "a = 1\n");
    home.repo_write(
        "dotfiles.toml",
        &format!("[[dotfile]]\npath = \"{path}\"\n"),
    );
    home.deploy();
    home.repo_write(
        "dotfiles.toml",
        &format!("[[dotfile]]\npath = \"{path}\"\ndeploy = \"create\"\npermissions = \"0600\"\n"),
    );
    // A link into the repo becomes the host's own copy.
    assert_eq!(home.deploy(), one(path, Action::Unchanged));
    assert!(!fs::symlink_metadata(&target).unwrap().is_symlink());
    assert_eq!(read(&target), "a = 1\n");
    let records = || {
        dotfiles::state::load(home.ctx.paths.state())
            .unwrap()
            .entries
    };
    assert!(records().is_empty());

    fs::remove_file(&target).unwrap();
    assert_eq!(home.deploy(), one(path, Action::Copied));
    assert_eq!(read(&target), "a = 1\n");
    assert_eq!(mode(&target), 0o600);
    assert!(records().is_empty());

    // From now on the file is the app's, whichever side changes.
    write(&target, "a = 2\n");
    home.repo_write("home/.config/foo/config.toml", "a = 3\n");
    assert_eq!(home.deploy(), one(path, Action::Unchanged));
    assert_eq!(read(&target), "a = 2\n");

    fs::remove_file(&target).unwrap();
    assert_eq!(home.deploy(), one(path, Action::Copied));
    assert_eq!(read(&target), "a = 3\n");
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
fn a_copy_turned_into_a_link_saves_what_it_shows() {
    let home = Home::new("auto");
    home.repo_write("home/.npmrc", "a\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".npmrc\"\ndeploy = \"copy\"\n",
    );
    home.deploy();
    write(&home.at("work/npmrc"), "b\n");
    fs::remove_file(home.at(".npmrc")).unwrap();
    std::os::unix::fs::symlink(home.at("work/npmrc"), home.at(".npmrc")).unwrap();
    assert_eq!(home.deploy(), one(".npmrc", Action::Saved));
    let source = home.repo().join("home/.npmrc");
    assert!(!source.symlink_metadata().unwrap().is_symlink());
    assert_eq!(read(&source), "b\n");
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
fn an_undeclared_private_link_stays_private() {
    let home = Home::new("ask");
    home.repo_write("home/.netrc", "machine x\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".netrc\"\npermissions = \"0600\"\n",
    );
    home.deploy();
    git(&home.repo(), &["add", "-A"]);
    git(
        &home.repo(),
        &["commit", "--quiet", "--no-verify", "-m", "add"],
    );
    // Another host forgot it; the pull deleted the source.
    home.repo_write("dotfiles.toml", "");
    git(&home.repo(), &["rm", "--quiet", "home/.netrc"]);
    git(
        &home.repo(),
        &["commit", "--quiet", "--no-verify", "-am", "forget"],
    );
    assert_eq!(home.deploy(), one(".netrc", Action::Released));
    assert_eq!(read(&home.at(".netrc")), "machine x\n");
    assert_eq!(mode(&home.at(".netrc")), 0o600);
}

#[test]
fn a_broken_declaration_fails_alone() {
    let home = Home::new("ask");
    home.repo_write("home/.vimrc", "set nu\n");
    home.repo_write("home/.npmrc", "save-exact = true\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".vimrc\"\n\n[[dotfile]]\npath = \".npmrc\"\n",
    );
    home.deploy();
    // Another host's add of an empty directory: git did not bring it here.
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".vimrc\"\npermissions = \"rw\"\n\n[[dotfile]]\npath = \".npmrc\"\n\n[[dotfile]]\npath = \".config/empty\"\n",
    );
    home.repo_write("home/.npmrc", "save-exact = false\n");

    let steps = home.deploy();

    assert!(
        steps.contains(&(".config/empty".into(), Action::Failed))
            && steps.contains(&(".vimrc".into(), Action::Failed)),
        "{steps:?}"
    );
    // Still linked: a broken declaration does not release what it deployed.
    assert!(home.at(".vimrc").symlink_metadata().unwrap().is_symlink());
    assert_eq!(read(&home.at(".npmrc")), "save-exact = false\n");
}

#[test]
fn duplicates_are_skipped_and_templates_render() {
    let home = Home::new("ask");
    home.repo_write("home/.npmrc", "");
    home.repo_write("home/.gitconfig", "");
    home.repo_write(
        "dotfiles.toml",
        r#"
[[dotfile]]
path = ".npmrc"
when = { os = ["macos", "linux"] }

[[dotfile]]
path = ".npmrc"

[[dotfile]]
path = ".gitconfig"
deploy = "template"
"#,
    );
    assert_eq!(
        home.deploy(),
        [
            (".gitconfig".to_string(), Action::Copied),
            (".npmrc".to_string(), Action::Skipped),
        ]
    );
    assert!(!home.at(".npmrc").exists());
}

#[test]
fn restore_original_removes_new_links() {
    let home = Home::new("ask");
    home.repo_write("home/.vimrc", "set nu\n");
    home.repo_write("dotfiles.toml", "[[dotfile]]\npath = \".vimrc\"\n");
    home.deploy();
    let snapshots = rosamta_core::snapshot::Snapshots::new(home.ctx.paths.state(), &home.ctx.home);
    let originals = snapshots.originals().unwrap();
    // What files held stays readable by this user alone.
    assert_eq!(mode(snapshots.dir()), 0o700);
    assert!(
        originals
            .iter()
            .any(|original| original.path == home.at(".vimrc") && original.content.is_none()),
        "{originals:?}"
    );
}

#[test]
fn restore_original_keeps_links_and_modes() {
    let home = Home::new("ask");
    let tool = home.at(".config/tool");
    write(&home.at("shared.conf"), "s\n");
    fs::set_permissions(home.at("shared.conf"), fs::Permissions::from_mode(0o600)).unwrap();
    write(&tool.join("token"), "t\n");
    fs::set_permissions(tool.join("token"), fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink("../../shared.conf", tool.join("shared")).unwrap();
    std::os::unix::fs::symlink("missing.conf", tool.join("dangling")).unwrap();
    write(&home.at("final.conf"), "f\n");
    std::os::unix::fs::symlink("final.conf", home.at("pointer.conf")).unwrap();
    std::os::unix::fs::symlink("../../pointer.conf", tool.join("chain")).unwrap();
    let snapshots = rosamta_core::snapshot::Snapshots::new(home.ctx.paths.state(), &home.ctx.home);
    snapshots
        .save(&dotfiles::tree::files(&tool).unwrap(), "test")
        .unwrap();
    fs::remove_dir_all(&tool).unwrap();
    fs::remove_file(home.at("shared.conf")).unwrap();
    fs::remove_file(home.at("final.conf")).unwrap();
    write(&tool.join("shared"), "other\n");
    // Someone holds a readable file where the private one was.
    write(&tool.join("token"), "x\n");
    fs::set_permissions(tool.join("token"), fs::Permissions::from_mode(0o644)).unwrap();
    let mut held = fs::File::open(tool.join("token")).unwrap();
    assert_eq!(
        rosamta_core::apply::plan_restore(&home.ctx).unwrap().len(),
        4
    );

    rosamta_core::apply::restore_original(&home.ctx).unwrap();

    assert_eq!(
        fs::read_link(home.at("pointer.conf")).unwrap(),
        Path::new("final.conf")
    );
    assert_eq!(read(&home.at("final.conf")), "f\n");
    let mut seen = String::new();
    std::io::Read::read_to_string(&mut held, &mut seen).unwrap();
    assert_eq!(
        seen, "x\n",
        "the restored bytes went into the readable file"
    );
    assert_eq!(mode(&home.at("shared.conf")), 0o600);

    assert_eq!(
        fs::read_link(tool.join("shared")).unwrap(),
        Path::new("../../shared.conf")
    );
    assert_eq!(read(&home.at("shared.conf")), "s\n");
    assert_eq!(
        fs::read_link(tool.join("dangling")).unwrap(),
        Path::new("missing.conf")
    );
    assert_eq!(read(&tool.join("token")), "t\n");
    assert_eq!(mode(&tool.join("token")), 0o600);
    assert!(
        rosamta_core::apply::plan_restore(&home.ctx)
            .unwrap()
            .is_empty()
    );
}

/// A snapshot taken before links and modes were kept writes through the
/// link that is there, as restores did then.
#[test]
fn restore_original_writes_old_snapshots_through_links() {
    let home = Home::new("ask");
    write(&home.at("dots/zshrc"), "old\n");
    std::os::unix::fs::symlink("dots/zshrc", home.at(".zshrc")).unwrap();
    let snapshots = rosamta_core::snapshot::Snapshots::new(home.ctx.paths.state(), &home.ctx.home);
    snapshots.save(&[home.at(".zshrc")], "test").unwrap();
    fs::remove_file(snapshots.dir().join("original-meta.json")).unwrap();
    write(&home.at("dots/zshrc"), "old\nhooked\n");
    assert_eq!(
        rosamta_core::apply::plan_restore(&home.ctx).unwrap().len(),
        1
    );

    rosamta_core::apply::restore_original(&home.ctx).unwrap();

    assert_eq!(
        fs::read_link(home.at(".zshrc")).unwrap(),
        Path::new("dots/zshrc")
    );
    assert_eq!(read(&home.at("dots/zshrc")), "old\n");
    assert!(
        rosamta_core::apply::plan_restore(&home.ctx)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn eject_leaves_regular_files() {
    let home = Home::new("ask");
    home.repo_write("home/.config/nvim/init.lua", "vim.o.number = true\n");
    home.repo_write("home/.npmrc", "save-exact = true\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".config/nvim\"\n\n[[dotfile]]\npath = \".npmrc\"\ndeploy = \"copy\"\n",
    );
    home.deploy();
    let nvim = home.at(".config/nvim");
    assert!(nvim.symlink_metadata().unwrap().is_symlink());
    let source = home.repo().join("home/.config/nvim");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();

    let released = dotfiles::manage::eject(&home.ctx).unwrap();

    assert_eq!(released, std::slice::from_ref(&nvim));
    assert!(nvim.symlink_metadata().unwrap().is_dir());
    assert_eq!(mode(&nvim), 0o700);
    assert_eq!(read(&nvim.join("init.lua")), "vim.o.number = true\n");
    assert_eq!(read(&home.at(".npmrc")), "save-exact = true\n");
    let deployed = dotfiles::state::load(home.ctx.paths.state()).unwrap();
    assert!(deployed.entries.is_empty());
}

#[test]
fn restore_original_puts_targets_back_without_touching_the_repo() {
    let home = Home::new("ask");
    home.repo_write("home/.config/nvim/init.lua", "repo\n");
    home.repo_write("home/.vimrc", "repo\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".config/nvim\"\n\n[[dotfile]]\npath = \".vimrc\"\n",
    );
    // Identical, so deploying takes it over without a conflict.
    write(&home.at(".vimrc"), "repo\n");
    home.deploy();
    assert!(home.at(".vimrc").symlink_metadata().unwrap().is_symlink());

    rosamta_core::apply::restore_original(&home.ctx).unwrap();

    // `.config/nvim` did not exist; `.vimrc` was a regular file.
    assert!(home.at(".config/nvim").symlink_metadata().is_err());
    let vimrc = home.at(".vimrc");
    assert!(!vimrc.symlink_metadata().unwrap().is_symlink());
    assert_eq!(read(&vimrc), "repo\n");
    assert_eq!(
        read(&home.repo().join("home/.config/nvim/init.lua")),
        "repo\n"
    );
    assert_eq!(read(&home.repo().join("home/.vimrc")), "repo\n");
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

mod encrypted {
    use super::*;
    use rosamta_core::dotfiles::crypt;

    const SSH: &str = "[[dotfile]]\npath = \".ssh/config\"\ndeploy = \"copy\"\npermissions = \"0600\"\nencrypted = true\n";

    /// A repo with an encrypted `.ssh/config`, made on another host.
    fn locked() -> Home {
        let home = Home::new("auto");
        crypt::create(&home.ctx, "correct horse").unwrap();
        let sealed = crypt::encrypt(&home.ctx, b"Host secret\n").unwrap();
        fs::create_dir_all(home.repo().join("home/.ssh")).unwrap();
        fs::write(home.repo().join("home/.ssh/config"), sealed).unwrap();
        home.repo_write("dotfiles.toml", SSH);
        fs::remove_file(home.ctx.paths.age_key()).unwrap();
        home
    }

    #[test]
    fn the_repo_holds_ciphertext_only() {
        let home = locked();
        let stored = read(&home.repo().join("home/.ssh/config"));
        assert!(
            stored.starts_with("-----BEGIN AGE ENCRYPTED FILE-----"),
            "{stored}"
        );
        assert!(!stored.contains("Host secret"));
        let key = read(&home.repo().join(crypt::REPO_KEY));
        assert!(
            key.starts_with("-----BEGIN AGE ENCRYPTED FILE-----")
                && !key.contains("AGE-SECRET-KEY")
        );
    }

    #[test]
    fn waits_for_the_passphrase_then_deploys() {
        let home = locked();
        assert!(crypt::locked(&home.ctx).unwrap());
        assert_eq!(home.deploy(), one(".ssh/config", Action::Skipped));
        assert!(!home.at(".ssh/config").exists());

        assert!(!crypt::unlock(&home.ctx, "wrong").unwrap());
        assert!(crypt::locked(&home.ctx).unwrap());
        assert!(crypt::unlock(&home.ctx, "correct horse").unwrap());
        assert_eq!(mode(&home.ctx.paths.age_key()), 0o600);
        assert_eq!(home.deploy(), one(".ssh/config", Action::Copied));
        assert_eq!(read(&home.at(".ssh/config")), "Host secret\n");
        assert_eq!(mode(&home.at(".ssh/config")), 0o600);
        assert_eq!(home.deploy(), one(".ssh/config", Action::Unchanged));
    }

    #[test]
    fn saved_changes_are_encrypted_again() {
        let home = locked();
        crypt::unlock(&home.ctx, "correct horse").unwrap();
        home.deploy();
        write(&home.at(".ssh/config"), "Host changed\n");
        assert_eq!(home.deploy(), one(".ssh/config", Action::Saved));
        let stored = fs::read(home.repo().join("home/.ssh/config")).unwrap();
        assert!(!String::from_utf8_lossy(&stored).contains("Host changed"));
        assert_eq!(
            crypt::decrypt(&home.ctx, &stored).unwrap().unwrap(),
            b"Host changed\n"
        );
        assert_eq!(home.deploy(), one(".ssh/config", Action::Unchanged));
    }

    #[test]
    fn a_second_key_is_refused() {
        let home = locked();
        let err = crypt::create(&home.ctx, "another").unwrap_err();
        assert!(err.to_string().contains("already has a key"), "{err}");
    }
}

mod manage {
    use super::*;
    use rosamta_core::dotfiles::{crypt, manage};

    fn log(home: &Home) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(home.repo())
            .args(["log", "--format=%s", "--name-only"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    }

    fn committed() -> Home {
        let home = Home::new("ask");
        git(&home.repo(), &["add", "-A"]);
        git(
            &home.repo(),
            &["commit", "--quiet", "--no-verify", "-m", "init"],
        );
        home
    }

    #[test]
    fn add_links_a_directory_and_commits() {
        let home = committed();
        write(&home.at(".config/ghostty/config"), "theme = dark\n");
        let added = manage::add(
            &home.ctx,
            &[home.at(".config/ghostty")],
            Deploy::Link,
            false,
        )
        .unwrap();
        assert_eq!(added, [PathBuf::from(".config/ghostty")]);
        let source = home.repo().join("home/.config/ghostty");
        assert_eq!(read(&source.join("config")), "theme = dark\n");
        assert_eq!(fs::read_link(home.at(".config/ghostty")).unwrap(), source);
        assert!(
            log(&home).starts_with("chore(add): .config/ghostty\n"),
            "{}",
            log(&home)
        );
        assert_eq!(home.deploy(), one(".config/ghostty", Action::Unchanged));
    }

    #[test]
    fn add_keeps_private_permissions() {
        let home = committed();
        write(&home.at(".ssh/config"), "Host lab\n");
        fs::set_permissions(home.at(".ssh/config"), fs::Permissions::from_mode(0o600)).unwrap();
        manage::add(&home.ctx, &[home.at(".ssh/config")], Deploy::Copy, false).unwrap();
        let manifest = read(&home.repo().join("dotfiles.toml"));
        assert!(
            manifest.contains("deploy = \"copy\"") && manifest.contains("permissions = \"0600\""),
            "{manifest}"
        );
        assert!(
            !fs::symlink_metadata(home.at(".ssh/config"))
                .unwrap()
                .is_symlink()
        );
    }

    #[test]
    fn add_refuses_what_it_should_not_take() {
        let home = committed();
        write(
            &home.at(".config/gh/hosts.yml"),
            "oauth_token: ghp_0123456789abcdefghijklmnopqrstuvwxyzAB\n",
        );
        let err = manage::add(
            &home.ctx,
            &[home.at(".config/gh/hosts.yml")],
            Deploy::Link,
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("add it with --encrypt"), "{err}");
        let err = manage::add(
            &home.ctx,
            &[PathBuf::from("/etc/hosts")],
            Deploy::Link,
            false,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("not inside the home directory"),
            "{err}"
        );
        let err = manage::add(&home.ctx, &[home.at(".config")], Deploy::Link, false).unwrap_err();
        assert!(err.to_string().contains("rosamta's own files"), "{err}");
        write(&home.at(".vimrc"), "set nu\n");
        manage::add(&home.ctx, &[home.at(".vimrc")], Deploy::Link, false).unwrap();
        let err = manage::add(&home.ctx, &[home.at(".vimrc")], Deploy::Link, false).unwrap_err();
        assert!(err.to_string().contains("already managed"), "{err}");
        assert!(!home.repo().join("home/.config/gh").exists());
        write(&home.at(".zshrc"), "alias ll='ls -l'\n");
        let err = manage::add(&home.ctx, &[home.at(".zshrc")], Deploy::Link, false).unwrap_err();
        assert!(err.to_string().contains("shell startup file"), "{err}");
        write(&home.at(".config/fish/config.fish"), "set -x A 1\n");
        let err =
            manage::add(&home.ctx, &[home.at(".config/fish")], Deploy::Link, false).unwrap_err();
        assert!(err.to_string().contains("config.fish"), "{err}");
        write(&home.at(".config/fish/conf.d/bun.fish"), "set -x B 1\n");
        let path = home.at(".config/fish/conf.d/bun.fish");
        let err = manage::add(&home.ctx, &[path], Deploy::Link, false).unwrap_err();
        assert!(err.to_string().contains("conf.d"), "{err}");
        fs::create_dir_all(home.at(".config/empty")).unwrap();
        let err =
            manage::add(&home.ctx, &[home.at(".config/empty")], Deploy::Link, false).unwrap_err();
        assert!(err.to_string().contains("empty directory"), "{err}");
    }

    #[test]
    fn add_encrypted_commits_ciphertext_and_the_key() {
        let home = committed();
        crypt::create(&home.ctx, "correct horse").unwrap();
        write(
            &home.at(".config/gh/hosts.yml"),
            "oauth_token: ghp_0123456789abcdefghijklmnopqrstuvwxyzAB\n",
        );
        manage::add(
            &home.ctx,
            &[home.at(".config/gh/hosts.yml")],
            Deploy::Link,
            true,
        )
        .unwrap();
        let stored = read(&home.repo().join("home/.config/gh/hosts.yml"));
        assert!(stored.starts_with("-----BEGIN AGE ENCRYPTED FILE-----"));
        let log = log(&home);
        assert!(
            log.contains("key.age") && log.contains("home/.config/gh/hosts.yml"),
            "{log}"
        );
        assert_eq!(
            home.deploy(),
            one(".config/gh/hosts.yml", Action::Unchanged)
        );
    }

    #[test]
    fn forget_leaves_a_regular_copy() {
        let home = committed();
        write(&home.at(".config/ghostty/config"), "theme = dark\n");
        manage::add(
            &home.ctx,
            &[home.at(".config/ghostty")],
            Deploy::Link,
            false,
        )
        .unwrap();
        manage::forget(&home.ctx, &home.at(".config/ghostty")).unwrap();
        let ghostty = home.at(".config/ghostty");
        assert!(!fs::symlink_metadata(&ghostty).unwrap().is_symlink());
        assert_eq!(read(&ghostty.join("config")), "theme = dark\n");
        assert!(!home.repo().join("home/.config/ghostty").exists());
        assert!(!read(&home.repo().join("dotfiles.toml")).contains("ghostty"));
        assert!(log(&home).starts_with("chore(forget): .config/ghostty\n"));
        assert!(home.deploy().is_empty());
        let err = manage::forget(&home.ctx, &ghostty).unwrap_err();
        assert!(err.to_string().contains("is not a dotfile"), "{err}");
    }

    #[test]
    fn add_leaves_other_staged_changes_out_of_its_commit() {
        let home = committed();
        home.repo_write("modules/wip.toml", "[env]\nWIP = \"1\"\n");
        git(&home.repo(), &["add", "modules/wip.toml"]);
        write(&home.at(".npmrc"), "save-exact = true\n");
        manage::add(&home.ctx, &[home.at(".npmrc")], Deploy::Link, false).unwrap();
        let log = log(&home);
        assert!(log.starts_with("chore(add): .npmrc\n"), "{log}");
        assert!(!log.contains("wip.toml"), "{log}");
        let out = Command::new("git")
            .arg("-C")
            .arg(home.repo())
            .args(["diff", "--cached", "--name-only"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(out.stdout).unwrap(), "modules/wip.toml\n");
    }

    #[test]
    fn forget_keeps_a_source_another_dotfile_uses() {
        let home = Home::new("ask");
        home.repo_write("home/.config/ghostty/config", "theme = dark\n");
        home.repo_write("home/.gitconfig", "[user]\n");
        home.repo_write(
            "dotfiles.toml",
            "[[dotfile]]\npath = \".config/ghostty\"\n\n\
             [[dotfile]]\npath = \".config/ghostty-work/config\"\nsource = \"home/.config/ghostty/config\"\n\n\
             [[dotfile]]\npath = \".gitconfig\"\n\n\
             [[dotfile]]\npath = \".gitconfig-work\"\nsource = \"./home/.gitconfig\"\n",
        );
        git(&home.repo(), &["add", "-A"]);
        git(
            &home.repo(),
            &["commit", "--quiet", "--no-verify", "-m", "init"],
        );
        home.deploy();
        manage::forget(&home.ctx, &home.at(".config/ghostty")).unwrap();
        manage::forget(&home.ctx, &home.at(".gitconfig")).unwrap();
        let manifest = read(&home.repo().join("dotfiles.toml"));
        assert!(
            !manifest.contains("path = \".config/ghostty\"\n"),
            "{manifest}"
        );
        assert!(!manifest.contains("path = \".gitconfig\"\n"), "{manifest}");
        assert_eq!(
            read(&home.repo().join("home/.config/ghostty/config")),
            "theme = dark\n"
        );
        assert_eq!(read(&home.repo().join("home/.gitconfig")), "[user]\n");
        assert_eq!(
            read(&home.at(".config/ghostty-work/config")),
            "theme = dark\n"
        );
        assert_eq!(read(&home.at(".gitconfig-work")), "[user]\n");
    }
}

#[test]
fn a_fish_conf_d_file_is_a_startup_file() {
    let home = Home::new("ask");
    home.repo_write("home/.config/fish/conf.d/bun.fish", "set -x B 1\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".config/fish/conf.d/bun.fish\"\n",
    );
    let manifest = dotfiles::load(&home.repo()).unwrap();
    let problems = dotfiles::check(&manifest, &home.repo());
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("startup file"), "{problems:?}");
}

#[test]
fn templates_report_syntax_errors_and_missing_vars() {
    let home = Home::new("ask");
    home.repo_write("home/.gitconfig", "[user]\n\temail = {{ vars.email }}\n");
    home.repo_write("home/.npmrc", "ok\n{% if %}\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".gitconfig\"\ndeploy = \"template\"\n\n[[dotfile]]\npath = \".npmrc\"\ndeploy = \"template\"\n",
    );
    let manifest = dotfiles::load(&home.repo()).unwrap();
    let problems = dotfiles::check(&manifest, &home.repo());
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains(".npmrc") && problems[0].contains("home/.npmrc:2"),
        "{problems:?}"
    );

    assert_eq!(dotfiles::missing_vars(&home.ctx).unwrap(), ["email"]);
    let host = home.ctx.paths.host_config();
    fs::write(&host, format!("# mine\n{}", read(&host))).unwrap();
    rosamta_core::host::set_var(&home.ctx.paths, "email", "").unwrap();
    assert!(dotfiles::missing_vars(&home.ctx).unwrap().is_empty());
    let text = read(&host);
    assert!(
        text.starts_with("# mine\n") && text.contains("[vars]\nemail = \"\""),
        "{text}"
    );
}

mod templates {
    use super::*;
    use rosamta_core::adopt::{self, Scope};
    use rosamta_core::dotfiles::review::{Reason, WaitingFor};

    const TEMPLATE: &str = "[user]\n\tname = {{ host.id }}\n\temail = {{ vars.email }}\n";

    fn with_template(vars: &str) -> Home {
        let home = Home::new("ask");
        home.repo_write("home/.gitconfig", TEMPLATE);
        home.repo_write(
            "dotfiles.toml",
            "[[dotfile]]\npath = \".gitconfig\"\ndeploy = \"template\"\n",
        );
        let host = home.ctx.paths.host_config();
        fs::write(&host, format!("{}{vars}", read(&host))).unwrap();
        home
    }

    fn set_email(home: &Home, email: &str) {
        rosamta_core::host::set_var(&home.ctx.paths, "email", email).unwrap();
    }

    #[test]
    fn output_is_private_and_follows_its_input() {
        let home = with_template("[vars]\nemail = \"a@example.com\"\n");
        assert_eq!(home.deploy(), one(".gitconfig", Action::Copied));
        let target = home.at(".gitconfig");
        assert_eq!(
            read(&target),
            "[user]\n\tname = mbp\n\temail = a@example.com\n"
        );
        assert_eq!(mode(&target), 0o600);
        assert_eq!(home.deploy(), one(".gitconfig", Action::Unchanged));

        // An existing file keeps the mode it has.
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        set_email(&home, "b@example.com");
        assert_eq!(home.deploy(), one(".gitconfig", Action::Updated));
        assert!(read(&target).contains("b@example.com"));
        assert_eq!(mode(&target), 0o644);
    }

    #[test]
    fn missing_vars_and_secret_commands_wait_outside_a_terminal() {
        let home = with_template("");
        let steps = dotfiles::deploy(&home.ctx).unwrap();
        assert_eq!(steps[0].action, Action::Skipped);
        assert!(
            steps[0].reason.as_deref().unwrap().contains("vars.email"),
            "{steps:?}"
        );
        assert!(!home.at(".gitconfig").exists());

        set_email(&home, "");
        home.repo_write(
            "secrets.toml",
            "[commands.op]\nrun = [\"op\", \"read\", \"{arg}\"]\n",
        );
        home.repo_write("home/.npmrc", "token={{ secret(\"op\", \"op://a/b\") }}\n");
        home.repo_write(
            "dotfiles.toml",
            "[[dotfile]]\npath = \".npmrc\"\ndeploy = \"template\"\n",
        );
        let steps = dotfiles::deploy(&home.ctx).unwrap();
        assert_eq!(steps[0].action, Action::Skipped);
        assert!(
            steps[0].reason.as_deref().unwrap().contains("`op`"),
            "{steps:?}"
        );
        assert!(!home.at(".npmrc").exists());
    }

    #[test]
    fn a_waiting_template_is_listed_but_not_a_decision() {
        let home = with_template("");
        home.deploy();
        let queue = adopt::queue(&home.ctx).unwrap();
        let item = queue[0].dotfile.as_ref().unwrap();
        assert_eq!(item.reason, Reason::Waiting);
        assert_eq!(
            item.waiting,
            Some(WaitingFor::Unset {
                vars: vec!["email".into()]
            })
        );
        let json = serde_json::to_string(&queue[0]).unwrap();
        assert!(
            json.contains(r#""reason":"waiting""#) && json.contains(r#""for":"unset""#),
            "{json}"
        );

        let id = queue[0].id;
        let refused = [
            adopt::accept(&home.ctx, id, Scope::Host).map(drop),
            adopt::ignore(&home.ctx, id).map(drop),
            rosamta_core::dotfiles::review::discard(&home.ctx, id).map(drop),
        ];
        for result in refused {
            let err = result.unwrap_err();
            assert!(err.to_string().contains("not a decision"), "{err}");
        }
        assert_eq!(adopt::queue(&home.ctx).unwrap().len(), 1);

        set_email(&home, "a@example.com");
        assert_eq!(home.deploy(), one(".gitconfig", Action::Copied));
        assert!(adopt::queue(&home.ctx).unwrap().is_empty());
    }

    fn edited() -> (Home, u32) {
        let home = with_template("[vars]\nemail = \"a@example.com\"\n");
        git(&home.repo(), &["add", "-A"]);
        git(
            &home.repo(),
            &["commit", "--quiet", "--no-verify", "-m", "init"],
        );
        home.deploy();
        write(&home.at(".gitconfig"), "[user]\n\tname = edited\n");
        assert_eq!(home.deploy(), one(".gitconfig", Action::Changed));
        let queue = adopt::queue(&home.ctx).unwrap();
        assert_eq!(queue[0].dotfile.as_ref().unwrap().reason, Reason::Changed);
        (home, queue[0].id)
    }

    #[test]
    fn accepting_edited_output_keeps_it_as_a_copy_for_this_host() {
        let (home, id) = edited();
        let err = adopt::accept(&home.ctx, id, Scope::All).unwrap_err();
        assert!(err.to_string().contains("--scope host"), "{err}");
        adopt::accept(&home.ctx, id, Scope::Host).unwrap();
        assert_eq!(
            read(&home.repo().join("hosts/mbp/.gitconfig")),
            "[user]\n\tname = edited\n"
        );
        let manifest = read(&home.repo().join("dotfiles.toml"));
        assert!(
            manifest.contains("source = \"hosts/mbp/.gitconfig\"\ndeploy = \"copy\"")
                && manifest.contains("when = { hosts = [\"mbp\"] }"),
            "{manifest}"
        );
        assert_eq!(read(&home.repo().join("home/.gitconfig")), TEMPLATE);
        assert!(adopt::queue(&home.ctx).unwrap().is_empty());
        assert_eq!(home.deploy(), one(".gitconfig", Action::Unchanged));
    }

    #[test]
    fn add_template_keeps_the_file_and_renders_it_unchanged() {
        let home = Home::new("ask");
        write(&home.at(".npmrc"), "registry=https://example.com\n");
        write(&home.at(".bad"), "{% if %}\n");
        let err = rosamta_core::dotfiles::manage::add(
            &home.ctx,
            &[home.at(".bad")],
            Deploy::Template,
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not a valid template"), "{err}");
        rosamta_core::dotfiles::manage::add(
            &home.ctx,
            &[home.at(".npmrc")],
            Deploy::Template,
            false,
        )
        .unwrap();
        let manifest = read(&home.repo().join("dotfiles.toml"));
        assert!(manifest.contains("deploy = \"template\""), "{manifest}");
        assert_eq!(
            read(&home.repo().join("home/.npmrc")),
            "registry=https://example.com\n"
        );
        assert!(fs::symlink_metadata(home.at(".npmrc")).unwrap().is_file());
        assert_eq!(home.deploy(), one(".npmrc", Action::Unchanged));
    }

    #[test]
    fn discarding_edited_output_renders_again() {
        let (home, id) = edited();
        rosamta_core::dotfiles::review::discard(&home.ctx, id).unwrap();
        assert!(read(&home.at(".gitconfig")).contains("a@example.com"));
        assert!(adopt::queue(&home.ctx).unwrap().is_empty());
    }

    #[test]
    fn the_review_diff_shows_the_output() {
        let (home, _) = edited();
        let queue = adopt::queue(&home.ctx).unwrap();
        let diff =
            rosamta_core::dotfiles::review::diff(&home.ctx, queue[0].dotfile.as_ref().unwrap())
                .unwrap();
        assert!(diff.contains("+\temail = a@example.com"), "{diff}");
        assert!(!diff.contains("{{"), "{diff}");
    }

    /// `.npmrc` rendered from a secret command that logs each run.
    fn with_secret(home: &mut Home) -> PathBuf {
        let log = home.ctx.home.join("runs");
        let program = home.ctx.home.join("fetch");
        write(
            &program,
            &format!("#!/bin/sh\necho run >> '{}'\necho \"$1\"\n", log.display()),
        );
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        home.repo_write(
            "secrets.toml",
            &format!(
                "[commands.fetch]\nrun = [\"{}\", \"{{arg}}\"]\n",
                program.display()
            ),
        );
        home.repo_write("home/.npmrc", "token={{ secret(\"fetch\", \"npm_1\") }}\n");
        home.repo_write(
            "dotfiles.toml",
            "[[dotfile]]\npath = \".npmrc\"\ndeploy = \"template\"\n",
        );
        log
    }

    fn runs(log: &Path) -> usize {
        fs::read_to_string(log).unwrap_or_default().lines().count()
    }

    #[test]
    fn a_terminal_unlocks_bw_once_for_the_whole_run() {
        let mut home = Home::new("ask");
        let bin = home.ctx.home.join("bin");
        write(
            &bin.join("bw"),
            r#"#!/bin/sh
echo "bw $*" >> "$(dirname "$0")/calls"
case "$1" in
status) [ "$BW_SESSION" = s3ss ] && echo '{"status":"unlocked"}' || echo '{"status":"locked"}' ;;
unlock) echo s3ss ;;
list) [ "$BW_SESSION" = s3ss ] || exit 1
  echo '[{"name":"GitHub","login":{"username":"me","password":"pw"}}]' ;;
esac
"#,
        );
        fs::set_permissions(bin.join("bw"), fs::Permissions::from_mode(0o755)).unwrap();
        home.ctx.path_var = bin.display().to_string();
        let host = home.ctx.paths.host_config();
        fs::write(
            &host,
            format!("{}[secrets]\nbitwarden = [\"bw\"]\n", read(&host)),
        )
        .unwrap();
        home.repo_write("home/.a", "{{ bitwarden(\"GitHub\") }}\n");
        home.repo_write("home/.b", "{{ bitwarden(\"GitHub\", \"username\") }}\n");
        home.repo_write(
            "dotfiles.toml",
            "[[dotfile]]\npath = \".a\"\ndeploy = \"template\"\n\n\
             [[dotfile]]\npath = \".b\"\ndeploy = \"template\"\n",
        );
        home.ctx.interactive = true;
        home.deploy();
        home.deploy();
        assert_eq!(read(&home.at(".a")), "pw\n");
        assert_eq!(read(&home.at(".b")), "me\n");
        let calls = read(&bin.join("calls"));
        assert_eq!(calls.matches("bw unlock").count(), 1, "{calls}");
        assert_eq!(calls.matches("bw list").count(), 1, "{calls}");
    }

    #[test]
    fn only_apply_in_a_terminal_renders_every_template() {
        let mut home = Home::new("ask");
        let log = with_secret(&mut home);
        home.repo_write("home/.gitconfig", "plain\n");
        home.repo_write(
            "dotfiles.toml",
            "[[dotfile]]\npath = \".npmrc\"\ndeploy = \"template\"\n\n\
             [[dotfile]]\npath = \".gitconfig\"\ndeploy = \"copy\"\n",
        );
        home.ctx.interactive = true;
        dotfiles::deploy::deploy_all(&home.ctx).unwrap();
        assert_eq!(runs(&log), 1);
        // A later run of `rsmt apply` asks again, for a changed secret.
        home.ctx.session = Default::default();
        dotfiles::deploy::deploy_all(&home.ctx).unwrap();
        assert_eq!(runs(&log), 2);

        // Discarding an unrelated file in a terminal leaves `.npmrc` alone.
        write(&home.at(".gitconfig"), "mine\n");
        home.ctx.interactive = false;
        home.deploy();
        let id = adopt::queue(&home.ctx).unwrap()[0].id;
        home.ctx.interactive = true;
        home.ctx.session = Default::default();
        rosamta_core::dotfiles::review::discard(&home.ctx, id).unwrap();
        assert_eq!(read(&home.at(".gitconfig")), "plain\n");
        assert_eq!(runs(&log), 2);

        // A new tag means nothing to a template that does not read tags.
        home.ctx.interactive = false;
        rosamta_core::host::add_tag(&home.ctx.paths, "work").unwrap();
        assert_eq!(
            home.deploy(),
            [
                (".gitconfig".to_string(), Action::Unchanged),
                (".npmrc".to_string(), Action::Unchanged)
            ]
        );
    }

    #[test]
    fn an_item_keeps_its_id_while_what_it_waits_for_changes() {
        let mut home = Home::new("ask");
        with_secret(&mut home);
        home.ctx.interactive = true;
        home.deploy();
        write(&home.at(".npmrc"), "token=mine\n");
        home.repo_write("home/.npmrc", "t={{ secret(\"fetch\", \"npm\") }}\n");
        assert_eq!(home.deploy(), one(".npmrc", Action::Conflict));
        let id = adopt::queue(&home.ctx).unwrap()[0].id;
        home.ctx.interactive = false;
        home.deploy();
        let queue = adopt::queue(&home.ctx).unwrap();
        assert_eq!(queue[0].dotfile.as_ref().unwrap().reason, Reason::Waiting);
        assert_eq!(queue[0].id, id);
    }

    #[test]
    fn a_link_becomes_a_template_without_a_conflict() {
        let home = with_template("[vars]\nemail = \"a@example.com\"\n");
        home.repo_write("home/.gitconfig", "plain\n");
        home.repo_write("dotfiles.toml", "[[dotfile]]\npath = \".gitconfig\"\n");
        assert_eq!(home.deploy(), one(".gitconfig", Action::Linked));
        home.repo_write("home/.gitconfig", TEMPLATE);
        home.repo_write(
            "dotfiles.toml",
            "[[dotfile]]\npath = \".gitconfig\"\ndeploy = \"template\"\n",
        );
        assert_eq!(home.deploy(), one(".gitconfig", Action::Updated));
        let target = home.at(".gitconfig");
        assert!(!fs::symlink_metadata(&target).unwrap().is_symlink());
        assert!(read(&target).contains("a@example.com"));
        assert_eq!(read(&home.repo().join("home/.gitconfig")), TEMPLATE);
    }

    #[test]
    fn output_holding_secrets_is_not_kept_in_the_repo() {
        let mut home = Home::new("ask");
        let log = with_secret(&mut home);
        home.ctx.interactive = true;
        assert_eq!(home.deploy(), one(".npmrc", Action::Copied));
        // One run serves every deploy of the same terminal session.
        assert_eq!(home.deploy(), one(".npmrc", Action::Unchanged));
        assert_eq!(runs(&log), 1);
        home.ctx.interactive = false;
        write(&home.at(".npmrc"), "token=npm_2\n");
        assert_eq!(home.deploy(), one(".npmrc", Action::Changed));
        let id = adopt::queue(&home.ctx).unwrap()[0].id;
        let err = adopt::accept(&home.ctx, id, Scope::Host).unwrap_err();
        assert!(err.to_string().contains("holds secrets"), "{err}");
        assert!(!home.repo().join("hosts").exists());
    }

    #[test]
    fn an_ignored_conflict_asks_no_secret_source() {
        let mut home = Home::new("ask");
        let log = with_secret(&mut home);
        write(&home.at(".npmrc"), "token=mine\n");
        home.ctx.interactive = true;
        assert_eq!(home.deploy(), one(".npmrc", Action::Conflict));
        let id = adopt::queue(&home.ctx).unwrap()[0].id;
        adopt::ignore(&home.ctx, id).unwrap();
        let steps = dotfiles::deploy(&home.ctx).unwrap();
        assert_eq!(steps[0].reason.as_deref(), Some("ignored"));
        home.ctx.interactive = false;
        let steps = dotfiles::deploy(&home.ctx).unwrap();
        assert_eq!(steps[0].reason.as_deref(), Some("ignored"));
        assert_eq!(runs(&log), 1);
        assert!(adopt::queue(&home.ctx).unwrap().is_empty());
    }

    #[test]
    fn output_already_in_place_is_not_a_conflict() {
        // A copy rosamta wrote becomes a template.
        let home = with_template("[vars]\nemail = \"a@example.com\"\n");
        home.repo_write("home/.gitconfig", "plain\n");
        home.repo_write(
            "dotfiles.toml",
            "[[dotfile]]\npath = \".gitconfig\"\ndeploy = \"copy\"\n",
        );
        assert_eq!(home.deploy(), one(".gitconfig", Action::Copied));
        home.repo_write("home/.gitconfig", TEMPLATE);
        home.repo_write(
            "dotfiles.toml",
            "[[dotfile]]\npath = \".gitconfig\"\ndeploy = \"template\"\n",
        );
        assert_eq!(home.deploy(), one(".gitconfig", Action::Updated));

        // The output landed but its record did not.
        set_email(&home, "b@example.com");
        write(
            &home.at(".gitconfig"),
            "[user]\n\tname = mbp\n\temail = b@example.com\n",
        );
        assert_eq!(home.deploy(), one(".gitconfig", Action::Unchanged));
        assert_eq!(home.deploy(), one(".gitconfig", Action::Unchanged));
    }
}

#[test]
fn a_declaration_for_this_host_alone_wins() {
    let home = Home::new("ask");
    home.repo_write("home/.gitconfig", "shared\n");
    home.repo_write("hosts/mbp/.gitconfig", "mine\n");
    home.repo_write(
        "dotfiles.toml",
        "[[dotfile]]\npath = \".gitconfig\"\ndeploy = \"copy\"\nwhen = { hosts = [\"mbp\", \"work\"] }\n\n\
         [[dotfile]]\npath = \".gitconfig\"\nsource = \"hosts/mbp/.gitconfig\"\ndeploy = \"copy\"\nwhen = { hosts = [\"mbp\"] }\n",
    );
    assert_eq!(home.deploy(), one(".gitconfig", Action::Copied));
    assert_eq!(read(&home.at(".gitconfig")), "mine\n");
}
