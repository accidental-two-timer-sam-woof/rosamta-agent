//! Rendered scripts sourced by real shells. Shells that are not installed
//! are skipped, so this runs fully only where every shell exists (the
//! Docker images); macOS covers bash 3.2, zsh and dash.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rosamta_core::config::{Module, Shell};
use rosamta_core::render::{Layout, render};
use rosamta_core::shells::which;

struct Rendered {
    _dir: tempfile::TempDir,
    root: PathBuf,
    layout: Layout,
}

impl Rendered {
    fn new(modules: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let layout = Layout {
            version_dir: root.join("generated/v-1"),
            version_id: "v-1".into(),
            profile_log: root.join("state/profile.log"),
        };
        std::fs::create_dir_all(root.join("state")).unwrap();
        let modules: Vec<(String, Module)> = modules
            .iter()
            .map(|(name, text)| (name.to_string(), toml::from_str(text).unwrap()))
            .collect();
        for (rel, text) in render(&modules, &Shell::ALL, &layout) {
            let path = layout.version_dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        Self {
            _dir: dir,
            root,
            layout,
        }
    }

    fn file(&self, shell: Shell, name: &str) -> String {
        self.layout
            .version_dir
            .join(shell.name())
            .join(format!("{name}.{}", shell.ext()))
            .display()
            .to_string()
    }
}

/// A shell to run, the script dialect it reads, and how to start it.
struct Runner {
    label: &'static str,
    dialect: Shell,
    argv: Vec<String>,
}

fn runners() -> Vec<Runner> {
    let path = std::env::var("PATH").unwrap_or_default();
    let mut out = Vec::new();
    let mut add = |label, dialect, exe: &str, extra: &[&str]| {
        if let Some(found) = which(exe, &path) {
            let mut argv = vec![found.display().to_string()];
            argv.extend(extra.iter().map(|arg| arg.to_string()));
            out.push(Runner {
                label,
                dialect,
                argv,
            });
        }
    };
    add("sh", Shell::Sh, "sh", &[]);
    add("dash", Shell::Sh, "dash", &[]);
    add("bash --posix", Shell::Sh, "bash", &["--posix"]);
    add("busybox ash", Shell::Sh, "busybox", &["ash"]);
    add("bash", Shell::Bash, "bash", &["--norc", "--noprofile"]);
    add("zsh", Shell::Zsh, "zsh", &["-f"]);
    add("fish", Shell::Fish, "fish", &["--no-config"]);
    out
}

fn run(runner: &Runner, script: &str, envs: &[(&str, &str)], root: &Path) -> Output {
    let mut cmd = Command::new(&runner.argv[0]);
    cmd.args(&runner.argv[1..]).arg("-c").arg(script);
    cmd.env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/home/tester")
        .env("USER", "tester");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.current_dir(root).output().unwrap()
}

fn source(runner: &Runner, file: &str) -> String {
    match runner.dialect {
        Shell::Fish => format!("source '{file}'"),
        _ => format!(". '{file}'"),
    }
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const VALUES: &str = r#"
[env]
A = "it's $USER"
B = 'a "quoted" \backslash `tick`'
C = "$5 literal $ and $-"
D = "~/x"
E = "${HOME}/y~"
F = "${UNSET_VAR}z"
G = "中文 ✓"
"#;

#[test]
fn values_match_in_every_shell() {
    let rendered = Rendered::new(&[("values", VALUES)]);
    let expected = "it's tester\na \"quoted\" \\backslash `tick`\n$5 literal $ and $-\n/home/tester/x\n/home/tester/y~\nz\n中文 ✓\n";
    let runners = runners();
    assert!(runners.len() >= 2, "need at least two shells to compare");
    for runner in &runners {
        let print = match runner.dialect {
            Shell::Fish => "printf '%s\\n' $A $B $C $D $E $F $G",
            _ => "printf '%s\\n' \"$A\" \"$B\" \"$C\" \"$D\" \"$E\" \"$F\" \"$G\"",
        };
        let script = format!(
            "{}; {print}",
            source(runner, &rendered.file(runner.dialect, "env"))
        );
        assert_eq!(
            stdout(&run(runner, &script, &[], &rendered.root)),
            expected,
            "{}",
            runner.label
        );
    }
}

#[test]
fn path_entries_are_ordered_and_never_duplicated() {
    let module = "[path]\nprepend = [\"/opt/a\", \"/opt/b\", \"/usr/bin\"]\nappend = [\"/opt/z\"]";
    let rendered = Rendered::new(&[("path", module)]);
    for runner in runners() {
        let env = rendered.file(runner.dialect, "env");
        let (reset, print) = match runner.dialect {
            Shell::Fish => ("set -e ROSAMTA_ENV", "string join : $PATH"),
            _ => ("unset ROSAMTA_ENV", "echo \"$PATH\""),
        };
        let script = format!(
            "{src}; {reset}; {src}; {print}",
            src = source(&runner, &env)
        );
        let out = stdout(&run(&runner, &script, &[], &rendered.root));
        assert_eq!(
            out.trim(),
            "/opt/a:/opt/b:/usr/bin:/bin:/opt/z",
            "{}",
            runner.label
        );
    }
}

#[test]
fn env_runs_once_per_process_tree() {
    let rendered = Rendered::new(&[(
        "count",
        "phase = \"env\"\n[raw]\nsh = \"N=$((N+1))\"\nbash = \"N=$((N+1))\"\nzsh = \"N=$((N+1))\"\nfish = \"set -g N (math $N + 1)\"",
    )]);
    for runner in runners() {
        let env = rendered.file(runner.dialect, "env");
        let print = if runner.dialect == Shell::Fish {
            "echo $N"
        } else {
            "echo \"$N\""
        };
        let init = if runner.dialect == Shell::Fish {
            "set -g N 0"
        } else {
            "N=0"
        };
        let script = format!("{init}; {src}; {src}; {print}", src = source(&runner, &env));
        assert_eq!(
            stdout(&run(&runner, &script, &[], &rendered.root)).trim(),
            "1",
            "{}",
            runner.label
        );
    }
}

#[test]
fn a_broken_snippet_names_its_module_and_others_still_load() {
    let rendered = Rendered::new(&[
        (
            "broken",
            "[raw]\nbash = \"if then fi\"\nzsh = \"echo )\"\nfish = \"if; end end\"",
        ),
        (
            "good",
            "order = 60\n[raw]\nbash = \"GOOD=1\"\nzsh = \"GOOD=1\"\nfish = \"set -g GOOD 1\"",
        ),
    ]);
    for runner in runners()
        .into_iter()
        .filter(|runner| runner.dialect != Shell::Sh)
    {
        let print = if runner.dialect == Shell::Fish {
            "echo $GOOD"
        } else {
            "echo \"$GOOD\""
        };
        let script = format!(
            "{}; {print}",
            source(&runner, &rendered.file(runner.dialect, "rc"))
        );
        let out = run(&runner, &script, &[], &rendered.root);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "1",
            "{}: {stderr}",
            runner.label
        );
        assert!(stderr.contains("broken"), "{}: {stderr}", runner.label);
    }
}

#[test]
fn aliases_live_only_in_rc() {
    let rendered = Rendered::new(&[("a", "[aliases]\nll = \"ls -l 'x y'\"")]);
    for runner in runners()
        .into_iter()
        .filter(|runner| runner.dialect != Shell::Sh)
    {
        let env = std::fs::read_to_string(rendered.file(runner.dialect, "env")).unwrap();
        assert!(!env.contains("alias"), "{}", runner.label);
        let show = if runner.dialect == Shell::Fish {
            "functions ll"
        } else {
            "alias ll"
        };
        let script = format!(
            "{}; {show}",
            source(&runner, &rendered.file(runner.dialect, "rc"))
        );
        assert!(
            stdout(&run(&runner, &script, &[], &rendered.root)).contains("ls -l"),
            "{}",
            runner.label
        );
    }
}

#[test]
fn profiling_logs_per_module_or_total() {
    let rendered = Rendered::new(&[("one", "[env]\nA = \"1\""), ("two", "[env]\nB = \"2\"")]);
    for runner in runners()
        .into_iter()
        .filter(|runner| matches!(runner.dialect, Shell::Bash | Shell::Zsh))
    {
        let log = rendered.layout.profile_log.clone();
        let _ = std::fs::remove_file(&log);
        let script = source(&runner, &rendered.file(runner.dialect, "env"));
        stdout(&run(
            &runner,
            &script,
            &[("ROSAMTA_PROFILE", "1")],
            &rendered.root,
        ));
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let has_clock = runner.dialect == Shell::Zsh
            || stdout(&run(
                &runner,
                "echo \"${EPOCHREALTIME-}\"",
                &[],
                &rendered.root,
            ))
            .trim()
            .len()
                > 1;
        if has_clock {
            assert!(
                text.contains(" one ") && text.contains(" two "),
                "{}: {text}",
                runner.label
            );
        } else {
            assert!(!text.contains(" one "), "{}: {text}", runner.label);
        }
        let perl = which("perl", "/usr/bin:/bin").is_some();
        if has_clock || perl {
            assert!(text.contains(" total "), "{}: {text}", runner.label);
        }
    }
}

#[test]
fn fish_creates_no_universal_variables() {
    let Some(runner) = runners()
        .into_iter()
        .find(|runner| runner.dialect == Shell::Fish)
    else {
        return;
    };
    let rendered = Rendered::new(&[(
        "v",
        "[env]\nEDITOR = \"nvim\"\n[path]\nprepend = [\"/opt/a\"]",
    )]);
    let config = rendered.root.join("fishconfig");
    std::fs::create_dir_all(&config).unwrap();
    let script = format!(
        "{}; echo $EDITOR",
        source(&runner, &rendered.file(Shell::Fish, "env"))
    );
    let out = run(
        &runner,
        &script,
        &[("XDG_CONFIG_HOME", config.to_str().unwrap())],
        &rendered.root,
    );
    assert_eq!(stdout(&out).trim(), "nvim");
    let vars = std::fs::read_to_string(config.join("fish/fish_variables")).unwrap_or_default();
    assert!(
        !vars.contains("EDITOR") && !vars.contains("/opt/a") && !vars.contains("ROSAMTA"),
        "{vars}"
    );
}
