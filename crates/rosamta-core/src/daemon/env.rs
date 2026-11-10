//! The environment of the user's login shell. launchd and systemd start the
//! daemon without reading any rc file, so `SSH_AUTH_SOCK` or a PATH set
//! there would be missing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::process::output_with_timeout;

const START: &str = "__ROSAMTA_ENV_START__";
const END: &str = "__ROSAMTA_ENV_END__";
const TIMEOUT: Duration = Duration::from_secs(10);

/// Variables that describe the capturing shell itself, or rosamta's own
/// markers: inheriting `ROSAMTA_ENV` would make every shell the daemon
/// starts skip its env stage.
fn keep(name: &str) -> bool {
    !matches!(name, "PWD" | "OLDPWD" | "SHLVL" | "_") && !name.starts_with("ROSAMTA_")
}

/// The user's shell: `$SHELL`, else the passwd entry.
pub fn user_shell() -> Option<PathBuf> {
    if let Some(shell) = std::env::var_os("SHELL").filter(|shell| !shell.is_empty()) {
        return Some(shell.into());
    }
    // SAFETY: getpwuid returns a pointer into static storage or null; it is
    // read before any other call could overwrite it.
    unsafe {
        let entry = libc::getpwuid(libc::getuid());
        if entry.is_null() || (*entry).pw_shell.is_null() {
            return None;
        }
        let shell = std::ffi::CStr::from_ptr((*entry).pw_shell);
        Some(PathBuf::from(shell.to_string_lossy().into_owned()))
    }
}

/// Run `shell` as an interactive login shell and read its environment.
pub fn login_shell_env(shell: &Path) -> Result<Vec<(String, String)>, String> {
    capture(Command::new(shell), TIMEOUT)
}

fn capture(mut cmd: Command, timeout: Duration) -> Result<Vec<(String, String)>, String> {
    // Markers fence off whatever rc files print, such as an instant prompt.
    cmd.args([
        "-l",
        "-i",
        "-c",
        &format!("echo {START}; env -0; echo {END}"),
    ]);
    let output = output_with_timeout(&mut cmd, timeout)
        .map_err(|err| format!("cannot start the login shell: {err}"))?
        .ok_or_else(|| format!("the login shell did not finish in {}s", timeout.as_secs()))?;
    parse(&output.stdout).ok_or_else(|| "the login shell printed no environment".into())
}

fn parse(stdout: &[u8]) -> Option<Vec<(String, String)>> {
    let text = String::from_utf8_lossy(stdout);
    let start = text.find(&format!("{START}\n"))? + START.len() + 1;
    let end = start + text[start..].rfind(&format!("{END}\n"))?;
    let vars: Vec<(String, String)> = text[start..end]
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .filter(|(name, _)| !name.is_empty() && keep(name))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    (!vars.is_empty()).then_some(vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ignores_noise_and_markers() {
        let out = b"\x1b[?25l instant prompt\n__ROSAMTA_ENV_START__\nA=1\0B=x=y\0SHLVL=2\0ROSAMTA_ENV=v1\0EMPTY=\0__ROSAMTA_ENV_END__\nbye\n";
        assert_eq!(
            parse(out).unwrap(),
            [
                ("A".to_string(), "1".to_string()),
                ("B".into(), "x=y".into()),
                ("EMPTY".into(), String::new())
            ]
        );
        assert_eq!(parse(b"no markers"), None);
    }

    /// A zsh setup like powerlevel10k's: the instant prompt writes to the
    /// terminal from `.zshrc`, and the SSH agent comes from `.zprofile`.
    #[test]
    fn zsh_with_instant_prompt() {
        let Some(zsh) = crate::shells::which("zsh", &std::env::var("PATH").unwrap_or_default())
        else {
            return;
        };
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join(".zprofile"),
            "export SSH_AUTH_SOCK=\"$HOME/agent.sock\"\n",
        )
        .unwrap();
        std::fs::write(
            home.path().join(".zshrc"),
            "print -n '\\e[?25l\\e[2K p10k instant prompt '\nprint junk\n",
        )
        .unwrap();
        let mut cmd = Command::new(zsh);
        cmd.env_clear()
            .env("HOME", home.path())
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("TERM", "dumb");
        let vars = capture(cmd, TIMEOUT).unwrap();
        let sock = vars.iter().find(|(name, _)| name == "SSH_AUTH_SOCK");
        let expected = home.path().join("agent.sock").display().to_string();
        assert_eq!(
            sock.map(|(_, value)| value.as_str()),
            Some(expected.as_str())
        );
        assert!(!vars.iter().any(|(name, _)| name == "SHLVL"));
    }

    #[test]
    fn a_hanging_shell_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let shell = dir.path().join("slow");
        std::fs::write(&shell, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&shell, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let started = std::time::Instant::now();
        let err = capture(Command::new(&shell), Duration::from_secs(1)).unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
