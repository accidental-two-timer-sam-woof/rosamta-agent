//! Externals downloaded from a local git repo and a local HTTP server into
//! a throwaway home.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, Once};

use rosamta_core::apply::Context;
use rosamta_core::externals::{self, Action};
use rosamta_core::paths::Paths;
use sha2::{Digest, Sha256};

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

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
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
        let ctx = Context {
            paths: Paths::from_vars(|name| (name == "HOME").then(|| OsString::from(&root)))
                .unwrap(),
            home: root,
            path_var: String::new(),
            interactive: false,
            session: Default::default(),
        };
        let home = Self { dir, ctx };
        write(
            &home.ctx.paths.repo().join("rosamta.toml"),
            "format = 1\nhosts = [\"mbp\"]\n",
        );
        write(&home.ctx.paths.host_config(), "id = \"mbp\"\n");
        home
    }

    fn at(&self, rel: &str) -> PathBuf {
        self.ctx.home.join(rel)
    }

    fn declare(&self, text: &str) {
        write(&self.ctx.paths.repo().join("externals.toml"), text);
    }

    fn apply(&self) -> Vec<(String, Action, Option<String>)> {
        externals::apply(&self.ctx)
            .unwrap()
            .into_iter()
            .map(|step| (step.path.display().to_string(), step.action, step.reason))
            .collect()
    }

    /// A repo outside the home with one commit holding `plugin.zsh`.
    fn upstream(&self, name: &str, text: &str) -> (PathBuf, String) {
        let dir = self.dir.path().join(name);
        fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "--quiet", "--initial-branch", "main"]);
        (dir.clone(), commit(&dir, text))
    }
}

fn commit(dir: &Path, text: &str) -> String {
    write(&dir.join("plugin.zsh"), text);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "--quiet", "-m", text]);
    git(dir, &["rev-parse", "HEAD"])
}

fn git_external(path: &str, url: &Path, rev: &str) -> String {
    format!(
        "[[external]]\npath = \"{path}\"\ngit = \"{}\"\nrev = \"{rev}\"\n",
        url.display()
    )
}

fn one(path: &str, action: Action) -> Vec<(String, Action, Option<String>)> {
    vec![(path.to_string(), action, None)]
}

#[test]
fn git_externals_follow_the_lock_and_keep_local_edits() {
    let home = Home::new();
    let (upstream, a) = home.upstream("omz", "v1\n");
    let omz = home.at(".oh-my-zsh");
    home.declare(&git_external(".oh-my-zsh", &upstream, &a));
    assert_eq!(home.apply(), one(".oh-my-zsh", Action::Downloaded));
    assert_eq!(read(&omz.join("plugin.zsh")), "v1\n");
    assert_eq!(git(&omz, &["rev-parse", "HEAD"]), a);
    assert_eq!(home.apply(), one(".oh-my-zsh", Action::Unchanged));

    let b = commit(&upstream, "v2\n");
    home.declare(&git_external(".oh-my-zsh", &upstream, &b));
    assert_eq!(home.apply(), one(".oh-my-zsh", Action::Updated));
    assert_eq!(read(&omz.join("plugin.zsh")), "v2\n");

    // The tool updating itself moves HEAD; untracked files do not matter.
    git(&omz, &["checkout", "--quiet", "--detach", &a]);
    write(&omz.join("custom/mine.zsh"), "mine\n");
    assert_eq!(
        home.apply(),
        vec![(
            ".oh-my-zsh".into(),
            Action::Updated,
            Some("moved back to the locked commit".into())
        )]
    );
    assert_eq!(git(&omz, &["rev-parse", "HEAD"]), b);

    write(&omz.join("plugin.zsh"), "edited\n");
    home.declare(&git_external(".oh-my-zsh", &upstream, &a));
    assert_eq!(
        home.apply(),
        vec![(
            ".oh-my-zsh".into(),
            Action::Changed,
            Some("has uncommitted changes".into())
        )]
    );
    assert_eq!(read(&omz.join("plugin.zsh")), "edited\n");
    assert_eq!(git(&omz, &["rev-parse", "HEAD"]), b);
}

/// A server that refuses commits it did not advertise leaves `--unshallow`
/// as the way to an older locked commit past the shallow edge.
#[test]
fn an_older_commit_past_the_shallow_edge_is_unshallowed() {
    let home = Home::new();
    let (upstream, a) = home.upstream("omz", "v1\n");
    let b = commit(&upstream, "v2\n");
    let omz = home.at(".oh-my-zsh");
    home.declare(&git_external(".oh-my-zsh", &upstream, &b));
    assert_eq!(home.apply(), one(".oh-my-zsh", Action::Downloaded));
    assert_eq!(git(&omz, &["rev-parse", "--is-shallow-repository"]), "true");
    // Protocol v0 asks only for advertised tips: `a` alone is refused.
    git(&omz, &["config", "protocol.version", "0"]);
    home.declare(&git_external(".oh-my-zsh", &upstream, &a));
    assert_eq!(home.apply(), one(".oh-my-zsh", Action::Updated));
    assert_eq!(git(&omz, &["rev-parse", "HEAD"]), a);
    assert_eq!(
        git(&omz, &["rev-parse", "--is-shallow-repository"]),
        "false"
    );
}

#[test]
fn existing_clones_and_strangers() {
    let home = Home::new();
    let (upstream, a) = home.upstream("omz", "v1\n");
    let (other, _) = home.upstream("other", "x\n");
    let omz = home.at(".oh-my-zsh");
    fs::create_dir_all(&home.ctx.home).unwrap();
    git(
        &home.ctx.home,
        &[
            "clone",
            "--quiet",
            &upstream.display().to_string(),
            ".oh-my-zsh",
        ],
    );
    let b = commit(&upstream, "v2\n");
    home.declare(&git_external(".oh-my-zsh", &upstream, &b));
    // A clone of the same repo is taken over.
    assert_eq!(home.apply(), one(".oh-my-zsh", Action::Updated));
    assert_eq!(git(&omz, &["rev-parse", "HEAD"]), b);

    // The declared URL moves to a mirror: rosamta's checkout follows.
    let mirror = home.dir.path().join("mirror.git");
    git(
        home.dir.path(),
        &[
            "clone",
            "--quiet",
            "--bare",
            &upstream.display().to_string(),
            "mirror.git",
        ],
    );
    home.declare(&git_external(".oh-my-zsh", &mirror, &a));
    assert_eq!(home.apply(), one(".oh-my-zsh", Action::Updated));
    assert_eq!(
        git(&omz, &["remote", "get-url", "origin"]),
        mirror.display().to_string()
    );
    assert_eq!(git(&omz, &["rev-parse", "HEAD"]), a);

    // Anything else stays as it is.
    write(&home.at(".tmux/plugins/tpm/README"), "mine\n");
    git(
        &home.ctx.home,
        &["clone", "--quiet", &other.display().to_string(), ".zplug"],
    );
    home.declare(
        &(git_external(".tmux/plugins/tpm", &upstream, &a)
            + &git_external(".zplug", &upstream, &a)),
    );
    let steps = home.apply();
    assert_eq!(steps[0].1, Action::Released);
    assert_eq!(
        steps[1..],
        [
            (
                ".tmux/plugins/tpm".into(),
                Action::Conflict,
                Some("something else is there".into())
            ),
            (
                ".zplug".into(),
                Action::Conflict,
                Some(format!("a clone of {} is there", other.display()))
            ),
        ]
    );
    assert_eq!(read(&home.at(".tmux/plugins/tpm/README")), "mine\n");
    // Released, not removed.
    assert!(omz.join("plugin.zsh").exists());
}

#[test]
fn inner_externals_wait_for_their_git_external() {
    let home = Home::new();
    let (upstream, a) = home.upstream("plugin", "p\n");
    let missing = home.dir.path().join("missing");
    home.declare(
        &(git_external(".oh-my-zsh", &missing, &a)
            + &git_external(".oh-my-zsh/custom/plugins/p", &upstream, &a)),
    );
    let steps = home.apply();
    assert_eq!(steps[0].1, Action::Failed);
    assert_eq!(
        steps[1],
        (
            ".oh-my-zsh/custom/plugins/p".into(),
            Action::Skipped,
            Some("waits for external `.oh-my-zsh`".into())
        )
    );
    assert!(!home.at(".oh-my-zsh").exists());
    // Nothing is left beside the target either.
    assert!(fs::read_dir(&home.ctx.home).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("rosamta-")
    }));
}

/// Serves `files` by path over HTTP on a local port.
fn serve(files: Arc<Mutex<HashMap<String, Vec<u8>>>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let path = line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let body = files.lock().unwrap().get(&path).cloned();
            let head = match &body {
                Some(body) => format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                ),
                None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .into(),
            };
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body.unwrap_or_default());
        }
    });
    base
}

fn tar_gz(files: &[(&str, &str)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, text) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(text.len() as u64);
        header.set_mode(0o644);
        builder
            .append_data(&mut header, name, text.as_bytes())
            .unwrap();
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&builder.into_inner().unwrap()).unwrap();
    encoder.finish().unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn archives_replace_only_what_rosamta_extracted() {
    let home = Home::new();
    let files = Arc::new(Mutex::new(HashMap::new()));
    let base = serve(files.clone());
    let declare = |bytes: &[u8], sum: &str| {
        files
            .lock()
            .unwrap()
            .insert("/font.tar.gz".into(), bytes.to_vec());
        home.declare(&format!(
            "[[external]]\npath = \".fonts/Mono\"\narchive = \"{base}/font.tar.gz\"\nsha256 = \"{sum}\"\nstrip = 1\n"
        ));
    };
    let fonts = home.at(".fonts/Mono");
    let v1 = tar_gz(&[("Mono-1/A.ttf", "a1")]);
    declare(&v1, &sha256(&v1));
    assert_eq!(home.apply(), one(".fonts/Mono", Action::Downloaded));
    assert_eq!(read(&fonts.join("A.ttf")), "a1");
    assert_eq!(home.apply(), one(".fonts/Mono", Action::Unchanged));

    let v2 = tar_gz(&[("Mono-2/B.ttf", "b2")]);
    declare(&v2, &sha256(&v1));
    let steps = home.apply();
    assert_eq!(steps[0].1, Action::Unchanged, "the lock did not change");
    files
        .lock()
        .unwrap()
        .insert("/font.tar.gz".into(), v2.clone());
    declare(&v2, &"0".repeat(64));
    let steps = home.apply();
    assert_eq!(steps[0].1, Action::Failed);
    assert!(
        steps[0].2.as_ref().unwrap().contains("not the locked"),
        "{steps:?}"
    );
    assert_eq!(read(&fonts.join("A.ttf")), "a1");

    declare(&v2, &sha256(&v2));
    assert_eq!(home.apply(), one(".fonts/Mono", Action::Updated));
    assert!(!fonts.join("A.ttf").exists());
    assert_eq!(read(&fonts.join("B.ttf")), "b2");

    write(&fonts.join("B.ttf"), "mine");
    declare(&v1, &sha256(&v1));
    assert_eq!(
        home.apply(),
        vec![(
            ".fonts/Mono".into(),
            Action::Changed,
            Some("changed on this host".into())
        )]
    );
    assert_eq!(read(&fonts.join("B.ttf")), "mine");

    home.declare("");
    assert_eq!(home.apply(), one(".fonts/Mono", Action::Released));
    assert_eq!(read(&fonts.join("B.ttf")), "mine");
}
