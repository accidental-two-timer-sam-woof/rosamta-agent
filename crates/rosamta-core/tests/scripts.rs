//! Running scripts in a throwaway home, as the background service would:
//! no terminal, `scripts = "auto"` unless a test says otherwise.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use rosamta_core::apply::Context;
use rosamta_core::paths::Paths;
use rosamta_core::scripts::{Run, Script, Timing};
use rosamta_core::scripts::apply::{self, Action, Why};

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

struct Home {
    _dir: tempfile::TempDir,
    ctx: Context,
}

impl Home {
    fn new(policy: &str) -> Self {
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
        write(
            &home.ctx.paths.host_config(),
            &format!("id = \"mbp\"\nscripts = \"{policy}\"\n"),
        );
        home
    }

    fn repo_write(&self, rel: &str, text: &str) {
        write(&self.ctx.paths.repo().join(rel), text);
    }

    fn at(&self, rel: &str) -> PathBuf {
        self.ctx.home.join(rel)
    }

    fn apply(&self) -> Vec<(String, Action, Option<Why>, Option<String>)> {
        apply::apply(&self.ctx, Timing::After)
            .unwrap()
            .into_iter()
            .map(|step| (step.source, step.action, step.why, step.reason))
            .collect()
    }

    fn log(&self) -> String {
        fs::read_to_string(self.at("ran")).unwrap_or_default()
    }
}

const SCRIPTS: &str = "[[script]]\nsource = \"scripts/install.sh\"\nrun = \"once\"\n\n\
                       [[script]]\nsource = \"scripts/brew\"\nrun = \"onchange\"\nwatch = [\"Brewfile\"]\n";

#[test]
fn once_runs_until_it_succeeds_and_onchange_follows_what_it_watches() {
    let mut home = Home::new("auto");
    home.repo_write("scripts.toml", SCRIPTS);
    // No shebang and no execute bit: sh runs it.
    home.repo_write(
        "scripts/install.sh",
        "[ -f \"$HOME/fail\" ] && exit 3\necho \"install $ROSAMTA_HOST_ID $PWD\" >> \"$HOME/ran\"\n",
    );
    home.repo_write(
        "scripts/brew",
        "#!/bin/sh -e\ntest -d \"$ROSAMTA_REPO\"\necho brew >> \"$HOME/ran\"\n",
    );
    home.repo_write("Brewfile", "brew \"git\"\n");
    write(&home.at("fail"), "");
    let steps = home.apply();
    assert_eq!(
        steps,
        [
            (
                "scripts/install.sh".into(),
                Action::Failed,
                Some(Why::New),
                Some("exited with 3".into())
            ),
            ("scripts/brew".into(), Action::Ran, Some(Why::New), None),
        ]
    );
    fs::remove_file(home.at("fail")).unwrap();
    // In the background a failed script waits for the user; a terminal runs it.
    let failed = |action| [("scripts/install.sh".into(), action, Some(Why::Failed), None)];
    assert_eq!(home.apply(), failed(Action::Queued));
    home.ctx.interactive = true;
    assert_eq!(home.apply(), failed(Action::Ran));
    home.ctx.interactive = false;
    assert_eq!(
        home.log(),
        format!(
            "brew\ninstall mbp {}\n",
            fs::canonicalize(&home.ctx.home).unwrap().display()
        )
    );
    assert!(home.apply().is_empty());

    // A once script that changes stays done; a watched file reruns onchange.
    home.repo_write("scripts/install.sh", "echo again >> \"$HOME/ran\"\n");
    home.repo_write("Brewfile", "brew \"git\"\nbrew \"jq\"\n");
    assert_eq!(
        home.apply(),
        [("scripts/brew".into(), Action::Ran, Some(Why::Changed), None)]
    );
    assert!(!home.log().contains("again"));
}

#[test]
fn logs_keep_the_output_in_order_privately_and_capped() {
    let home = Home::new("auto");
    home.repo_write(
        "scripts.toml",
        "[[script]]\nsource = \"noisy.sh\"\nrun = \"once\"\n",
    );
    home.repo_write(
        "noisy.sh",
        "echo out\necho err >&2\necho out2\nhead -c 2097152 /dev/zero | tr '\\0' a\n",
    );
    assert_eq!(home.apply()[0].1, Action::Ran);
    let log = apply_log(&home, "noisy.sh");
    let text = fs::read_to_string(&log).unwrap();
    assert!(text.contains("out\nerr\nout2\n"), "{}", &text[..100]);
    assert!(text.contains("# output past 1 MiB is not logged"));
    assert!(text.ends_with("# exited with 0\n"));
    assert!(text.len() < (1 << 20) + 1000);
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&log), 0o600);
    assert_eq!(mode(log.parent().unwrap()), 0o700);
}

fn apply_log(home: &Home, source: &str) -> PathBuf {
    rosamta_core::scripts::run::log_path(&home.ctx, source)
}

#[test]
fn ask_lists_scripts_instead_of_running_them() {
    let home = Home::new("ask");
    home.repo_write("scripts.toml", SCRIPTS);
    home.repo_write("scripts/install.sh", "echo install >> \"$HOME/ran\"\n");
    home.repo_write("scripts/brew", "echo brew >> \"$HOME/ran\"\n");
    home.repo_write("Brewfile", "");
    let actions: Vec<Action> = home.apply().into_iter().map(|step| step.1).collect();
    assert_eq!(actions, [Action::Queued, Action::Queued]);
    assert_eq!(home.log(), "");
}

#[test]
fn a_service_the_script_starts_does_not_hold_up_the_run() {
    let home = Home::new("auto");
    home.repo_write(
        "scripts.toml",
        "[[script]]\nsource = \"start.sh\"\nrun = \"once\"\n",
    );
    // The background sleep keeps the output pipe open after the script exits.
    home.repo_write("start.sh", "sleep 20 &\necho started\n");
    let started = std::time::Instant::now();
    assert_eq!(home.apply()[0].1, Action::Ran);
    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    let text = fs::read_to_string(apply_log(&home, "start.sh")).unwrap();
    assert!(text.ends_with("started\n# exited with 0\n"), "{text}");
}

#[test]
fn how_the_watch_list_is_written_does_not_count_as_a_change() {
    let home = Home::new("auto");
    home.repo_write("brew", "");
    home.repo_write("a", "1");
    home.repo_write("b", "2");
    let script = |watch: &[&str]| Script {
        source: "brew".into(),
        run: Run::Onchange,
        timing: Timing::After,
        watch: watch.iter().map(|path| path.to_string()).collect(),
        when: Default::default(),
    };
    let repo = home.ctx.paths.repo();
    let hash = |watch: &[&str]| apply::hash(&repo, &script(watch)).unwrap();
    assert_eq!(hash(&["a", "b"]), hash(&["b", "./a", "a"]));
    assert_ne!(hash(&["a", "b"]), hash(&["a"]));
}
