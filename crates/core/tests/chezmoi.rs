//! Importing chezmoi sources into a throwaway home and repo. The comparison
//! with `chezmoi apply` runs only where chezmoi is installed.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use rosamta_core::apply::Context;
use rosamta_core::chezmoi::{self, Chezmoi, Options, Outcome, Reason, Report};
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
            unsafe { std::env::set_var(key, value) };
        }
    });
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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

struct Home {
    dir: tempfile::TempDir,
    ctx: Context,
}

impl Home {
    fn new() -> Self {
        isolate_git();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("home");
        fs::create_dir_all(&root).unwrap();
        let ctx = Context {
            paths: Paths::from_vars(|name| (name == "HOME").then(|| OsString::from(&root)))
                .unwrap(),
            home: root,
            path_var: std::env::var("PATH").unwrap_or_default(),
            interactive: false,
            session: Default::default(),
        };
        let repo = ctx.paths.repo();
        write(
            &repo.join("rosamta.toml"),
            "format = 1\nhosts = [\"mbp\"]\n",
        );
        write(&ctx.paths.host_config(), "id = \"mbp\"\n");
        git(&repo, &["init", "--quiet", "--initial-branch=main"]);
        git(&repo, &["add", "--all"]);
        git(&repo, &["commit", "--quiet", "-m", "init"]);
        Self { dir, ctx }
    }

    fn source(&self) -> PathBuf {
        self.dir.path().join("source")
    }

    fn add(&self, rel: &str, text: &str) {
        write(&self.source().join(rel), text);
    }

    fn import(&self, chezmoi: Option<Chezmoi>, dry_run: bool) -> Report {
        chezmoi::import(
            &self.ctx,
            &Options {
                source: Some(self.source()),
                dry_run,
                chezmoi,
            },
        )
        .unwrap()
    }

    fn try_import(&self) -> rosamta_core::Result<Report> {
        chezmoi::import(
            &self.ctx,
            &Options {
                source: Some(self.source()),
                dry_run: false,
                chezmoi: None,
            },
        )
    }

    fn repo(&self, rel: &str) -> String {
        fs::read_to_string(self.ctx.paths.repo().join(rel)).unwrap_or_default()
    }
}

fn find(report: &Report, source: &str) -> (Outcome, Reason) {
    let item = report
        .items
        .iter()
        .find(|item| item.source == source && item.outcome != Outcome::Note)
        .unwrap_or_else(|| panic!("{source} not in {:#?}", report.items));
    (item.outcome, item.reason)
}

/// A source with one of each kind the import handles.
fn fixture(home: &Home) {
    home.add("dot_gitconfig", "[user]\n\tname = me\n");
    home.add("private_dot_netrc", "machine example.com\n");
    home.add("dot_local/bin/executable_hello", "#!/bin/sh\necho hello\n");
    home.add(
        "dot_yabairc.tmpl",
        "{{ if eq .chezmoi.os \"darwin\" -}}\nyabai -m config layout bsp\n{{ end -}}\n",
    );
    home.add(
        "dot_config/i3/config",
        "bindsym $mod+Return exec alacritty\n",
    );
    home.add(
        ".chezmoiignore",
        "README.md\n{{ if ne .chezmoi.os \"linux\" }}\n.config/i3\n{{ end }}\n",
    );
    home.add("README.md", "my dotfiles\n");
    home.add("empty_dot_hushlogin", "");
    home.add("dot_blank", "  \n");
    home.add("dot_zshrc", "export EDITOR=vim\n");
    home.add("dot_env", "LEVEL=debug\n");
    home.add("dot_empty/.gitkeep", "");
    home.add("run_onchange_after_report.py", "print('done')\n");
    home.add("symlink_dot_vimrc", ".config/nvim/init.vim\n");
    home.add("dot_bad.tmpl", "{{ range .list }}x{{ end }}\n");
    home.add("dot_Brewfile", "brew \"git\"\n");
    home.add("run_once_before_10-install.sh", "#!/bin/sh\necho install\n");
    home.add(
        "run_onchange_brew.sh.tmpl",
        "#!/bin/sh\n# {{ include \"dot_Brewfile\" | sha256sum }}\nbrew bundle\n",
    );
    home.add(
        ".chezmoiexternal.toml",
        "[\".oh-my-zsh\"]\ntype = \"git-repo\"\nurl = \"https://github.com/ohmyzsh/ohmyzsh.git\"\nclone.args = [\"--branch\", \"master\"]\nrefreshPeriod = \"168h\"\n\n\
         [\".local/share/fonts/mono\"]\ntype = \"archive\"\nurl = \"https://example.com/mono.tar.gz\"\nstripComponents = 1\nchecksum.sha256 = \"ABCD\"\n\n\
         [\".local/bin/age\"]\ntype = \"archive-file\"\nurl = \"https://example.com/age.tar.gz\"\npath = \"age/age\"\n\n\
         [\".zsh/plugin\"]\ntype = \"git-repo\"\nurl = \"https://example.com/plugin.git\"\nclone.args = [\"--depth\", \"1\", \"--recurse-submodules\"]\n",
    );
}

#[test]
fn a_source_becomes_declarations() {
    let home = Home::new();
    fixture(&home);
    // As a global excludes file of the user's would.
    write(&home.ctx.paths.repo().join(".git/info/exclude"), ".env\n");
    let dry = home.import(None, true);
    assert_eq!(home.repo("dotfiles.toml"), "", "a dry run writes nothing");
    let report = home.import(None, false);
    assert_eq!(dry, report);

    let expect = [
        ("dot_gitconfig", Outcome::Imported, Reason::Dotfile),
        ("private_dot_netrc", Outcome::Imported, Reason::Dotfile),
        ("dot_yabairc.tmpl", Outcome::Imported, Reason::Dotfile),
        ("dot_config/i3/config", Outcome::Imported, Reason::Dotfile),
        ("README.md", Outcome::Skipped, Reason::Ignored),
        ("empty_dot_hushlogin", Outcome::Imported, Reason::Dotfile),
        ("dot_blank", Outcome::Skipped, Reason::Empty),
        ("dot_zshrc", Outcome::Skipped, Reason::StartupFile),
        ("dot_env", Outcome::Manual, Reason::GitIgnored),
        ("dot_empty", Outcome::Manual, Reason::EmptyDir),
        ("symlink_dot_vimrc", Outcome::Manual, Reason::Symlink),
        ("dot_bad.tmpl", Outcome::Manual, Reason::Template),
        (
            "run_once_before_10-install.sh",
            Outcome::Imported,
            Reason::Script,
        ),
        (
            "run_onchange_brew.sh.tmpl",
            Outcome::Imported,
            Reason::Script,
        ),
        (
            ".chezmoiexternal.toml: .oh-my-zsh",
            Outcome::Imported,
            Reason::External,
        ),
        (
            ".chezmoiexternal.toml: .local/share/fonts/mono",
            Outcome::Imported,
            Reason::External,
        ),
        (
            ".chezmoiexternal.toml: .local/bin/age",
            Outcome::Manual,
            Reason::ExternalType,
        ),
        (
            ".chezmoiexternal.toml: .zsh/plugin",
            Outcome::Manual,
            Reason::ExternalField,
        ),
    ];
    for (source, outcome, reason) in expect {
        assert_eq!(find(&report, source), (outcome, reason), "{source}");
    }

    let manifest = home.repo("dotfiles.toml");
    for line in [
        "path = \".gitconfig\"\ndeploy = \"copy\"\n",
        "path = \".netrc\"\ndeploy = \"copy\"\npermissions = \"0600\"\n",
        "path = \".yabairc\"\ndeploy = \"copy\"\nwhen = { os = [\"macos\"] }\n",
        "path = \".config/i3/config\"\ndeploy = \"copy\"\nwhen = { os = [\"linux\"] }\n",
    ] {
        assert!(manifest.contains(line), "{line}\n{manifest}");
    }
    let repo = home.ctx.paths.repo();
    assert_eq!(home.repo("home/.yabairc"), "yabai -m config layout bsp\n");
    assert_eq!(mode(&repo.join("home/.local/bin/hello")), 0o755);
    assert_eq!(
        home.repo("chezmoi-import/dot_bad.tmpl"),
        "{{ range .list }}x{{ end }}\n"
    );
    assert!(repo.join("chezmoi-import/symlink_dot_vimrc").exists());

    let scripts = home.repo("scripts.toml");
    assert!(
        scripts.contains(
            "source = \"scripts/10-install.sh\"\nrun = \"onchange\"\ntiming = \"before\"\n"
        ),
        "{scripts}"
    );
    assert!(
        scripts.contains(
            "source = \"scripts/brew.sh\"\nrun = \"onchange\"\nwatch = [\"home/.Brewfile\"]\n"
        ),
        "{scripts}"
    );
    assert_eq!(home.repo("scripts/brew.sh"), "#!/bin/sh\n# \nbrew bundle\n");
    assert_eq!(
        home.repo("scripts/report.py"),
        "#!/usr/bin/env python3\nprint('done')\n"
    );
    // chezmoi already ran them here.
    assert!(
        rosamta_core::scripts::apply::plan(&home.ctx)
            .unwrap()
            .is_empty()
    );

    let externals = home.repo("externals.toml");
    assert!(externals.contains(
        "path = \".oh-my-zsh\"\ngit = \"https://github.com/ohmyzsh/ohmyzsh.git\"\nbranch = \"master\"\n"
    ), "{externals}");
    assert!(
        externals.contains("sha256 = \"abcd\"\nstrip = 1\n"),
        "{externals}"
    );

    let log = git(&repo, &["log", "-1", "--format=%B"]);
    assert!(
        log.starts_with("import(chezmoi): 7 dotfiles, 3 scripts, 2 externals\n"),
        "{log}"
    );
    assert!(
        log.contains("- manual dot_bad.tmpl: template (line 1: `range`)"),
        "{log}"
    );
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");

    // Importing again leaves what is there, and what was set aside.
    fs::write(repo.join("chezmoi-import/dot_bad.tmpl"), "mine\n").unwrap();
    git(&repo, &["commit", "--quiet", "-am", "converted"]);
    let again = home.import(None, false);
    assert_eq!(
        find(&again, "dot_gitconfig"),
        (Outcome::Skipped, Reason::Exists)
    );
    assert_eq!(home.repo("chezmoi-import/dot_bad.tmpl"), "mine\n");

    // Changes not published yet would go into the import's commit.
    fs::write(repo.join("home/.gitconfig"), "edited\n").unwrap();
    home.add("dot_new", "new\n");
    let err = home.try_import().unwrap_err();
    assert!(err.to_string().contains("rsmt publish"), "{err}");
    assert!(!repo.join("home/.new").exists());
}

/// Names that leave the source or the repo are refused, and symlinks are
/// not followed: what they point to is not the user's dotfiles.
#[test]
fn a_source_cannot_reach_outside_itself() {
    let home = Home::new();
    let outside = home.dir.path().join("outside");
    write(&outside, "password = hunter2-outside\n");
    home.add("dot_gitconfig", "[user]\n");
    fs::create_dir_all(home.source()).unwrap();
    std::os::unix::fs::symlink(&outside, home.source().join("private_dot_pgpass")).unwrap();
    let report = home.import(None, false);
    assert_eq!(
        find(&report, "private_dot_pgpass"),
        (Outcome::Manual, Reason::Link)
    );
    let repo = home.ctx.paths.repo();
    assert!(!git(&repo, &["log", "-p"]).contains("hunter2"));

    home.add("dot_./dot_git/hooks/executable_post-commit", "#!/bin/sh\n");
    assert!(home.try_import().is_err());
    fs::remove_dir_all(home.source().join("dot_.")).unwrap();
    home.add(".chezmoiroot", &home.dir.path().display().to_string());
    assert!(home.try_import().is_err());
    assert!(!repo.join(".git/hooks/post-commit").exists());
}

fn sandbox_chezmoi(home: &Home, destination: &Path) -> Option<Chezmoi> {
    let program = rosamta_core::shells::which("chezmoi", &home.ctx.path_var)?;
    let state = home.dir.path().join("chezmoi");
    fs::create_dir_all(&state).unwrap();
    let path = |name: &str| state.join(name).display().to_string();
    Some(Chezmoi {
        program,
        args: vec![
            "--source".into(),
            home.source().display().to_string(),
            "--destination".into(),
            destination.display().to_string(),
            "--config".into(),
            path("chezmoi.toml"),
            "--cache".into(),
            path("cache"),
            "--persistent-state".into(),
            path("state.boltdb"),
            "--no-tty".into(),
        ],
        terminal: false,
    })
}

/// The files `chezmoi apply` writes and the ones rosamta deploys after the
/// import are the same, contents and modes.
#[test]
fn a_host_gets_what_chezmoi_gave_it() {
    let home = Home::new();
    let Some(chezmoi) = sandbox_chezmoi(&home, &home.ctx.home) else {
        eprintln!("chezmoi is not installed; skipped");
        return;
    };
    let state = home.dir.path().join("chezmoi");
    let key = state.join("key.txt");
    let keygen = Command::new(&chezmoi.program)
        .args(["age-keygen", "--output"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(keygen.status.success());
    let recipient = fs::read_to_string(&key)
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("# public key: ").map(str::to_string))
        .unwrap();
    write(
        &state.join("chezmoi.toml"),
        &format!(
            "encryption = \"age\"\nuseBuiltinAge = \"on\"\n\n[age]\nidentity = \"{}\"\nrecipient = \"{recipient}\"\n\n\
             [data]\nemail = \"me@example.com\"\nwork = false\n",
            key.display()
        ),
    );
    let hostname = String::from_utf8(
        Command::new(&chezmoi.program)
            .args(&chezmoi.args)
            .args(["execute-template", "{{ .chezmoi.hostname }}"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    fixture(&home);
    // chezmoi reads archives even when told to skip externals.
    fs::remove_file(home.source().join(".chezmoiexternal.toml")).unwrap();
    home.add(
        ".chezmoidata.toml",
        "font = \"JetBrains Mono\"\nemail = \"shared@example.com\"\nlist = [\"a\"]\nsize = 14.0\n",
    );
    home.add(
        "private_dot_app/settings.tmpl",
        &format!(
            "font = {{{{ .font }}}}\nemail = {{{{ .email }}}}\nos = {{{{ .chezmoi.os }}}}\n\
             {{{{ if eq .chezmoi.hostname \"{hostname}\" }}}}here{{{{ else }}}}elsewhere{{{{ end }}}}\n\
             {{{{ if .work }}}}work{{{{ else }}}}home{{{{ end }}}}\n${{#array}}\n"
        ),
    );
    home.add(
        "dot_local/bin/executable_tool.tmpl",
        "#!/bin/sh\necho {{ .chezmoi.arch }}\n",
    );
    let ignore = fs::read_to_string(home.source().join(".chezmoiignore")).unwrap();
    home.add(
        ".chezmoiignore",
        &format!(
            "{ignore}.cache/**\n!.cache/keep\n\
             {{{{ if ne .chezmoi.os \"linux\" }}}}.chezmoiscripts/linux/**{{{{ end }}}}\n"
        ),
    );
    home.add("dot_cache/a/x", "x\n");
    home.add("dot_cache/keep", "keep\n");
    home.add(
        "dot_config/.chezmoiignore",
        "app/secret # kept here\nnvim/\n",
    );
    home.add("dot_config/nvim/init.vim", "set number\n");
    home.add("dot_workconf.tmpl", "{{ if .work }}[work]\n{{ end }}");
    home.add(
        "dot_settings.json.tmpl",
        "{\n{{- if eq .chezmoi.os \"darwin\" }}\n  \"mac\": true\n{{- end }}\n}\n",
    );
    home.add("dot_size.tmpl", "size = {{ .size }}\n");
    home.add(
        ".chezmoiscripts/linux/run_once_setup.sh",
        "#!/bin/sh\necho linux\n",
    );
    home.add(".chezmoiscripts/run_once_setup.sh", "#!/bin/sh\necho any\n");
    home.add("run_once_dot_hidden.sh", "#!/bin/sh\necho hidden\n");
    home.add("dot_config/app/config", "config\n");
    home.add("dot_config/app/secret", "secret\n");
    let plain = home.dir.path().join("plain");
    write(&plain.join(".token"), "token = abc\n");
    let adder = sandbox_chezmoi(&home, &plain).unwrap();
    let added = Command::new(&adder.program)
        .args(&adder.args)
        .args(["add", "--encrypt"])
        .arg(plain.join(".token"))
        .output()
        .unwrap();
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );

    let theirs = home.dir.path().join("theirs");
    fs::create_dir_all(&theirs).unwrap();
    let reference = sandbox_chezmoi(&home, &theirs).unwrap();
    let applied = Command::new(&reference.program)
        .args(&reference.args)
        .args(["apply", "--exclude", "scripts,externals"])
        .output()
        .unwrap();
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );

    rosamta_core::dotfiles::crypt::create(&home.ctx, "correct horse").unwrap();
    let report = home.import(Some(chezmoi), false);
    assert_eq!(
        find(&report, "encrypted_dot_token.age"),
        (Outcome::Imported, Reason::Dotfile)
    );
    assert_eq!(
        find(&report, "dot_workconf.tmpl"),
        (Outcome::Manual, Reason::MayBeEmpty)
    );
    assert_eq!(
        find(&report, ".chezmoiscripts/run_once_setup.sh"),
        (Outcome::Imported, Reason::Script)
    );
    assert_eq!(
        find(&report, ".chezmoiscripts/linux/run_once_setup.sh"),
        (Outcome::Imported, Reason::Script)
    );
    let scripts = home.repo("scripts.toml");
    for declared in [
        "source = \"scripts/linux/setup.sh\"\nrun = \"onchange\"\nwhen = { os = [\"linux\"] }\n",
        "source = \"scripts/.hidden.sh\"\n",
    ] {
        assert!(scripts.contains(declared), "{declared}\n{scripts}");
    }
    let repo = home.ctx.paths.repo();
    // The key made for the import is committed with what it seals.
    assert_eq!(
        git(&repo, &["ls-files", "key.age", "home/.token"])
            .lines()
            .count(),
        2
    );
    let sealed = fs::read(repo.join("home/.token")).unwrap();
    assert!(!String::from_utf8_lossy(&sealed).contains("abc"));
    assert_eq!(
        find(&report, "private_dot_app/settings.tmpl"),
        (Outcome::Imported, Reason::Dotfile)
    );
    assert_eq!(
        find(&report, "dot_local/bin/executable_tool.tmpl"),
        (Outcome::Imported, Reason::Dotfile)
    );
    let host = fs::read_to_string(home.ctx.paths.host_config()).unwrap();
    assert!(
        host.contains("email = \"me@example.com\"") && host.contains("work = \"false\""),
        "{host}"
    );

    rosamta_core::dotfiles::deploy(&home.ctx).unwrap();
    for target in [
        ".gitconfig",
        ".netrc",
        ".local/bin/hello",
        ".hushlogin",
        ".app/settings",
        ".local/bin/tool",
        ".yabairc",
        ".config/i3/config",
        ".cache/a/x",
        ".cache/keep",
        ".config/app/config",
        ".config/app/secret",
        ".token",
        ".config/nvim/init.vim",
        ".workconf",
        ".settings.json",
        ".size",
    ] {
        let (ours, theirs) = (home.ctx.home.join(target), theirs.join(target));
        assert_eq!(ours.exists(), theirs.exists(), "{target}");
        if !theirs.exists() {
            continue;
        }
        assert_eq!(
            fs::read(&ours).unwrap(),
            fs::read(&theirs).unwrap(),
            "{target}"
        );
        assert_eq!(mode(&ours), mode(&theirs), "{target}");
    }
}
