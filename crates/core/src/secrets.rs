//! Where templates get secrets: Bitwarden through rbw, bw or bws, and the
//! commands the repo declares in `secrets.toml`. Values stay in memory for
//! one apply and are never written anywhere.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::apply::Context;
use crate::config::{BitwardenTool, HostConfig};
use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::process::output_with_timeout;
use crate::shells::which;
use crate::token::TokenStore;
use crate::validate;

pub const MANIFEST: &str = "secrets.toml";

/// A Bitwarden CLI reading the local vault answers in seconds; bw is a
/// Node program and slower to start.
const TIMEOUT: Duration = Duration::from_secs(60);

const ORDER: [BitwardenTool; 3] = [BitwardenTool::Rbw, BitwardenTool::Bw, BitwardenTool::Bws];

/// `secrets.toml`: the commands templates may run with `secret()`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretCommands {
    #[serde(default)]
    pub commands: BTreeMap<String, SecretCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretCommand {
    /// The program and its arguments, run without a shell; `{arg}` in any
    /// of them becomes the template's argument.
    pub run: Vec<String>,
}

/// The repo's commands; none when the file does not exist.
pub fn load(repo_dir: &Path) -> Result<SecretCommands> {
    let path = repo_dir.join(MANIFEST);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(err) => return Err(Error::Read { path, source: err }),
    };
    validate::parse(&text).map_err(|issue| {
        Error::Usage(format!(
            "{}: {}; run `rsmt validate` for details",
            path.display(),
            issue.message
        ))
    })
}

/// Problems `rsmt validate` reports, one message each.
pub fn check(commands: &SecretCommands) -> Vec<String> {
    commands
        .commands
        .iter()
        .filter_map(|(name, command)| {
            let problem = match command.run.first().map(String::as_str) {
                None | Some("") => "run must start with a program",
                Some(program) if program.contains("{arg}") => "the program cannot come from {arg}",
                // Relative to wherever rsmt runs, which may be any checkout.
                Some(program) if program.contains('/') && !program.starts_with('/') => {
                    "the program must be a name on PATH or an absolute path"
                }
                Some(_) => return None,
            };
            Some(format!("command `{name}`: {problem}"))
        })
        .collect()
}

/// Why a secret could not be read without a terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waiting {
    /// The tool is locked; only `rsmt apply` in a terminal unlocks it.
    Locked(BitwardenTool),
    /// Secret commands run only in a terminal.
    Command(String),
}

/// A secret that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Waiting(Waiting),
    Error(String),
}

/// What one apply has fetched, so each secret is read once.
#[derive(Default)]
struct State {
    /// The Bitwarden tool in use, once chosen; `None` inside when none is.
    tool: Option<Option<(BitwardenTool, PathBuf)>>,
    /// `bw unlock --raw` output, for this apply only.
    bw_session: Option<String>,
    items: BTreeMap<String, Value>,
    bws_secrets: Option<Vec<Value>>,
    /// The stored bws token, once read.
    bws_token: Option<Option<String>>,
    outputs: BTreeMap<(String, String), String>,
    waiting: Vec<Waiting>,
    /// Whether the tool in use could be read, once checked.
    unlocked: Option<std::result::Result<(), Failure>>,
    /// Secrets handed to templates, cached or not.
    used: usize,
}

/// The secret sources of one apply.
pub struct Session {
    interactive: bool,
    order: Vec<BitwardenTool>,
    commands: BTreeMap<String, SecretCommand>,
    path_var: String,
    bws_token: BwsToken,
    state: Mutex<State>,
}

/// Read only when bws is about to be used, so a run without templates never
/// opens the keychain.
enum BwsToken {
    Given(Option<String>),
    Stored(Paths),
}

impl Session {
    /// `interactive` is `rsmt apply` in a terminal, the only place that
    /// unlocks a tool or runs a secret command.
    pub fn new(ctx: &Context, host: &HostConfig, interactive: bool) -> Result<Self> {
        let mut session = Self::with(
            interactive,
            &host.secrets.bitwarden,
            load(&ctx.paths.repo())?,
            ctx.path_var.clone(),
            None,
        );
        session.bws_token = BwsToken::Stored(ctx.paths.clone());
        Ok(session)
    }

    pub(crate) fn with(
        interactive: bool,
        order: &[BitwardenTool],
        commands: SecretCommands,
        path_var: String,
        bws_token: Option<String>,
    ) -> Self {
        Self {
            interactive,
            order: if order.is_empty() {
                ORDER.to_vec()
            } else {
                order.to_vec()
            },
            commands: commands.commands,
            path_var,
            bws_token: BwsToken::Given(bws_token),
            state: Mutex::default(),
        }
    }

    fn bws_token(&self) -> Option<String> {
        let paths = match &self.bws_token {
            BwsToken::Given(token) => return token.clone(),
            BwsToken::Stored(paths) => paths,
        };
        if let Some(token) = &self.state().bws_token {
            return token.clone();
        }
        let token = match std::env::var("BWS_ACCESS_TOKEN") {
            Ok(token) if !token.is_empty() => Some(token),
            _ => TokenStore::bws(paths).load().ok().flatten(),
        };
        self.state().bws_token = Some(token.clone());
        token
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// How many secrets templates have been given so far; a render that
    /// raised it holds a secret.
    pub fn used(&self) -> usize {
        self.state().used
    }

    fn hand(&self, value: String) -> String {
        self.state().used += 1;
        value
    }

    /// What renders since the last call waited for, emptying the list.
    pub fn take_waiting(&self) -> Vec<Waiting> {
        std::mem::take(&mut self.state().waiting)
    }

    fn wait(&self, waiting: Waiting) -> Failure {
        self.state().waiting.push(waiting.clone());
        Failure::Waiting(waiting)
    }

    /// `bitwarden(item, field)`: the password when `field` is omitted.
    pub fn bitwarden(
        &self,
        item: &str,
        field: Option<&str>,
    ) -> std::result::Result<String, Failure> {
        let Some((tool, program)) = self.tool() else {
            let tried: Vec<&str> = self.order.iter().map(|tool| tool.name()).collect();
            return Err(Failure::Error(format!(
                "no Bitwarden tool is installed and signed in (tried {})",
                tried.join(", ")
            )));
        };
        if tool == BitwardenTool::Bws {
            return self
                .bws(&program, item, field)
                .map(|value| self.hand(value));
        }
        let cached = self.state().items.get(item).cloned();
        let entry = match cached {
            Some(entry) => entry,
            None => {
                let unlocked = self.state().unlocked.clone();
                let unlocked = match unlocked {
                    Some(unlocked) => unlocked,
                    None => {
                        let unlocked = self.unlock(tool, &program);
                        self.state().unlocked = Some(unlocked.clone());
                        unlocked
                    }
                };
                if let Err(failure) = unlocked {
                    // Each render takes what it waited for.
                    return Err(match failure {
                        Failure::Waiting(waiting) => self.wait(waiting),
                        error => error,
                    });
                }
                let entry = self.fetch(tool, &program, item)?;
                self.state().items.insert(item.to_string(), entry.clone());
                entry
            }
        };
        field_of(tool, &entry, field)
            .map(|value| self.hand(value))
            .ok_or_else(|| {
                Failure::Error(format!(
                    "Bitwarden item `{item}` has no {}",
                    field.unwrap_or("password")
                ))
            })
    }

    /// The first tool in the order that is installed and signed in.
    fn tool(&self) -> Option<(BitwardenTool, PathBuf)> {
        if let Some(tool) = &self.state().tool {
            return tool.clone();
        }
        let chosen = self.order.iter().find_map(|&tool| {
            let program = which(tool.name(), &self.path_var)?;
            self.signed_in(tool, &program).then_some((tool, program))
        });
        self.state().tool = Some(chosen.clone());
        chosen
    }

    fn signed_in(&self, tool: BitwardenTool, program: &Path) -> bool {
        match tool {
            BitwardenTool::Rbw => run_json(program, &["config", "show"], &[]).is_ok_and(|config| {
                config["email"]
                    .as_str()
                    .is_some_and(|email| !email.is_empty())
            }),
            BitwardenTool::Bw => self
                .bw_status(program)
                .is_some_and(|status| status != "unauthenticated"),
            BitwardenTool::Bws => self.bws_token().is_some(),
        }
    }

    fn bw_status(&self, program: &Path) -> Option<String> {
        let env = self.bw_env();
        let status = run_json(program, &["status"], &env).ok()?;
        status["status"].as_str().map(str::to_string)
    }

    fn bw_env(&self) -> Vec<(&'static str, String)> {
        self.state()
            .bw_session
            .clone()
            .map(|session| vec![("BW_SESSION", session)])
            .unwrap_or_default()
    }

    /// Unlock a locked tool in a terminal, where the tool asks for the
    /// master password itself; elsewhere, wait.
    fn unlock(&self, tool: BitwardenTool, program: &Path) -> std::result::Result<(), Failure> {
        let locked = match tool {
            BitwardenTool::Rbw => {
                !run(program, &["unlocked"], &[]).is_ok_and(|out| out.status.success())
            }
            BitwardenTool::Bw => self.bw_status(program).as_deref() == Some("locked"),
            BitwardenTool::Bws => false,
        };
        if !locked {
            return Ok(());
        }
        if !self.interactive {
            return Err(Failure::Waiting(Waiting::Locked(tool)));
        }
        let failed = |err: String| Failure::Error(format!("cannot unlock {}: {err}", tool.name()));
        let mut cmd = Command::new(program);
        cmd.stdin(Stdio::inherit()).stderr(Stdio::inherit());
        for name in BW_OUTPUT_VARS {
            cmd.env_remove(name);
        }
        match tool {
            BitwardenTool::Rbw => {
                let status = cmd
                    .arg("unlock")
                    .status()
                    .map_err(|err| failed(err.to_string()))?;
                if !status.success() {
                    return Err(failed(format!("rbw unlock exited with {status}")));
                }
            }
            _ => {
                let out = cmd
                    .args(["unlock", "--raw"])
                    .stdout(Stdio::piped())
                    .output()
                    .map_err(|err| failed(err.to_string()))?;
                let session = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !out.status.success() || session.is_empty() {
                    return Err(failed(format!("bw unlock exited with {}", out.status)));
                }
                self.state().bw_session = Some(session);
            }
        }
        Ok(())
    }

    fn fetch(
        &self,
        tool: BitwardenTool,
        program: &Path,
        item: &str,
    ) -> std::result::Result<Value, Failure> {
        let failed =
            |err: String| Failure::Error(format!("{} cannot read `{item}`: {err}", tool.name()));
        // Both tools fall back to partial matches; only the item named counts.
        let named = |entry: &Value| entry["name"] == item || entry["id"] == item;
        if tool == BitwardenTool::Rbw {
            let entry = run_json(program, &["get", "--raw", "--", item], &[]).map_err(failed)?;
            return match named(&entry) {
                true => Ok(entry),
                false => Err(failed(format!("no item is named `{item}`"))),
            };
        }
        let search = format!("--search={item}");
        let list =
            run_json(program, &["list", "items", &search], &self.bw_env()).map_err(failed)?;
        let mut found = list
            .as_array()
            .into_iter()
            .flatten()
            .filter(|entry| named(entry));
        match (found.next(), found.next()) {
            (Some(entry), None) => Ok(entry.clone()),
            (None, _) => Err(failed(format!("no item is named `{item}`"))),
            (Some(_), Some(_)) => Err(failed(format!("several items are named `{item}`"))),
        }
    }

    /// bws has no items, only secrets with a key, a value and a note; one
    /// list per apply finds them by key.
    fn bws(
        &self,
        program: &Path,
        key: &str,
        field: Option<&str>,
    ) -> std::result::Result<String, Failure> {
        let name = match field {
            None => "value",
            Some("note") => "note",
            Some(other) => {
                return Err(Failure::Error(format!(
                    "bws secrets have only a value and a note, not `{other}`"
                )));
            }
        };
        if self.state().bws_secrets.is_none() {
            let token = self.bws_token().unwrap_or_default();
            let list = run_json(
                program,
                &["secret", "list", "--output", "json", "--color", "no"],
                &[("BWS_ACCESS_TOKEN", token)],
            )
            .map_err(|err| Failure::Error(format!("bws cannot list secrets: {err}")))?;
            self.state().bws_secrets = Some(list.as_array().cloned().unwrap_or_default());
        }
        let state = self.state();
        let mut found = state
            .bws_secrets
            .iter()
            .flatten()
            .filter(|secret| secret["key"] == key);
        match (found.next(), found.next()) {
            (Some(secret), None) => Ok(secret[name].as_str().unwrap_or_default().to_string()),
            (None, _) => Err(Failure::Error(format!(
                "bws has no secret with key `{key}`"
            ))),
            (Some(_), Some(_)) => Err(Failure::Error(format!(
                "bws has several secrets with key `{key}`"
            ))),
        }
    }

    /// `secret(name, arg)`: the output of a command `secrets.toml` declares,
    /// run only in a terminal.
    pub fn secret(&self, name: &str, arg: &str) -> std::result::Result<String, Failure> {
        let Some(command) = self.commands.get(name) else {
            return Err(Failure::Error(format!(
                "secret command `{name}` is not declared in {MANIFEST}"
            )));
        };
        let key = (name.to_string(), arg.to_string());
        let cached = self.state().outputs.get(&key).cloned();
        if let Some(output) = cached {
            return Ok(self.hand(output));
        }
        if !self.interactive {
            return Err(self.wait(Waiting::Command(name.to_string())));
        }
        let run: Vec<String> = command
            .run
            .iter()
            .map(|part| part.replace("{arg}", arg))
            .collect();
        let failed = |why: String| Failure::Error(format!("secret command `{name}` failed: {why}"));
        let (program, args) = run
            .split_first()
            .ok_or_else(|| failed("run is empty".into()))?;
        let out = run_in_terminal(program, args).map_err(|err| failed(err.to_string()))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let last = stderr.lines().rev().find(|line| !line.trim().is_empty());
            return Err(failed(last.unwrap_or("no error output").trim().to_string()));
        }
        let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
        if output.ends_with('\n') {
            output.pop();
            if output.ends_with('\r') {
                output.pop();
            }
        }
        self.state().outputs.insert(key, output.clone());
        Ok(self.hand(output))
    }
}

impl BitwardenTool {
    pub fn name(self) -> &'static str {
        match self {
            Self::Rbw => "rbw",
            Self::Bw => "bw",
            Self::Bws => "bws",
        }
    }
}

/// One field of an item as rbw (`get --raw`) or bw (`get item`) prints it.
fn field_of(tool: BitwardenTool, entry: &Value, field: Option<&str>) -> Option<String> {
    let login = match tool {
        BitwardenTool::Rbw => &entry["data"],
        _ => &entry["login"],
    };
    let value = match field.unwrap_or("password") {
        name @ ("password" | "username" | "totp") => &login[name],
        "notes" => &entry["notes"],
        custom => entry["fields"]
            .as_array()?
            .iter()
            .find(|candidate| candidate["name"] == custom)
            .map(|found| &found["value"])?,
    };
    value.as_str().map(str::to_string)
}

/// The terminal stays the command's, for a password or Touch ID prompt:
/// stdin is shared and stderr is shown as it comes, and kept for the error.
fn run_in_terminal(program: &str, args: &[String]) -> std::io::Result<std::process::Output> {
    use std::io::{Read, Write};
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stderr = child.stderr.take().expect("piped");
    let echo = std::thread::spawn(move || {
        let (mut kept, mut buf) = (Vec::new(), [0u8; 4096]);
        while let Ok(read @ 1..) = stderr.read(&mut buf) {
            let _ = std::io::stderr().write_all(&buf[..read]);
            kept.extend_from_slice(&buf[..read]);
        }
        kept
    });
    let mut out = child.wait_with_output()?;
    out.stderr = echo.join().unwrap_or_default();
    Ok(out)
}

/// bw variables that reshape its output or exit code.
const BW_OUTPUT_VARS: [&str; 3] = ["BW_RESPONSE", "BW_QUIET", "BW_CLEANEXIT"];

fn run(
    program: &Path,
    args: &[&str],
    env: &[(&str, String)],
) -> std::result::Result<std::process::Output, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    // bw only: plain JSON and a real exit code, and no prompt when locked.
    for name in BW_OUTPUT_VARS {
        cmd.env_remove(name);
    }
    cmd.env("BW_NOINTERACTION", "true");
    for (name, value) in env {
        cmd.env(name, value);
    }
    output_with_timeout(&mut cmd, TIMEOUT)
        .map_err(|err| err.to_string())?
        .ok_or_else(|| format!("no answer in {}s", TIMEOUT.as_secs()))
}

fn run_json(
    program: &Path,
    args: &[&str],
    env: &[(&str, String)],
) -> std::result::Result<Value, String> {
    let out = run(program, args, env)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let last = stderr.lines().rev().find(|line| !line.trim().is_empty());
        return Err(last.unwrap_or("failed").trim().to_string());
    }
    serde_json::from_slice(&out.stdout).map_err(|err| format!("unreadable output: {err}"))
}

/// The Bitwarden desktop app's SSH agent socket for how it was installed;
/// `None` without the desktop app.
pub fn bitwarden_ssh_socket(home: &Path) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    socket(
        home,
        &path_var,
        Path::new("/Applications"),
        Path::new("/opt/Bitwarden"),
    )
}

fn socket(home: &Path, path_var: &str, applications: &Path, opt: &Path) -> Option<PathBuf> {
    let store = home.join("Library/Containers/com.bitwarden.desktop");
    if store.is_dir() {
        return Some(store.join("Data/.bitwarden-ssh-agent.sock"));
    }
    let snap = home.join("snap/bitwarden");
    if snap.is_dir() {
        return Some(snap.join("current/.bitwarden-ssh-agent.sock"));
    }
    let flatpak = home.join(".var/app/com.bitwarden.desktop");
    if flatpak.is_dir() {
        return Some(flatpak.join("data/.bitwarden-ssh-agent.sock"));
    }
    let installed = applications.join("Bitwarden.app").is_dir()
        || home.join("Applications/Bitwarden.app").is_dir()
        || opt.is_dir()
        || which("bitwarden", path_var).is_some();
    installed.then(|| home.join(".bitwarden-ssh-agent.sock"))
}

#[cfg(test)]
mod tests;
