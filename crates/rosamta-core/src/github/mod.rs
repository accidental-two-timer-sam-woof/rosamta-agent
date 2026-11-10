//! Linking GitHub (ADR 0010): device flow sign-in, the token, the repo and
//! how git reaches it.

mod token;

use std::path::Path;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::git;
use crate::paths::Paths;
use crate::repo::{self, InitOutcome};

pub use token::{Stored, TokenStore};

/// rosamta's OAuth App. Device flow needs only the client ID, which is public.
pub const CLIENT_ID: &str = "Ov23liLGTz876ezcWYf7";
const SCOPE: &str = "repo";
/// The credential helper rosamta adds to a repo that reaches GitHub over HTTPS.
const HELPER_KEY: &str = "credential.https://github.com.helper";

pub struct GitHub {
    web: String,
    api: String,
    client_id: String,
    agent: ureq::Agent,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    interval: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RepoInfo {
    pub full_name: String,
    pub ssh_url: String,
    pub clone_url: String,
    pub private: bool,
}

impl Default for GitHub {
    fn default() -> Self {
        Self::with_urls("https://github.com", "https://api.github.com", CLIENT_ID)
    }
}

impl GitHub {
    pub fn with_urls(web: &str, api: &str, client_id: &str) -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(concat!("rosamta/", env!("CARGO_PKG_VERSION")))
            .timeout_global(Some(Duration::from_secs(30)))
            .tls_config(tls)
            .build()
            .new_agent();
        Self {
            web: web.trim_end_matches('/').into(),
            api: api.trim_end_matches('/').into(),
            client_id: client_id.into(),
            agent,
        }
    }

    pub fn request_code(&self) -> Result<DeviceCode> {
        if self.client_id.is_empty() {
            return Err(Error::GitHub("this build has no GitHub client ID".into()));
        }
        let url = format!("{}/login/device/code", self.web);
        let form = [("client_id", self.client_id.as_str()), ("scope", SCOPE)];
        let mut reply = self
            .agent
            .post(&url)
            .header("Accept", "application/json")
            .send_form(form)
            .map_err(unreachable)?;
        if !reply.status().is_success() {
            return Err(Error::GitHub(format!(
                "GitHub refused the sign-in request ({})",
                reply.status()
            )));
        }
        reply.body_mut().read_json().map_err(unreadable)
    }

    /// Poll until the user authorizes the code, declines it, or it expires.
    pub fn wait_for_token(&self, code: &DeviceCode) -> Result<String> {
        let url = format!("{}/login/oauth/access_token", self.web);
        let deadline = Instant::now() + Duration::from_secs(code.expires_in);
        let mut interval = code.interval;
        loop {
            std::thread::sleep(Duration::from_secs(interval));
            if Instant::now() >= deadline {
                return Err(expired());
            }
            let form = [
                ("client_id", self.client_id.as_str()),
                ("device_code", code.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ];
            let reply: TokenReply = self
                .agent
                .post(&url)
                .header("Accept", "application/json")
                .send_form(form)
                .map_err(unreachable)?
                .body_mut()
                .read_json()
                .map_err(unreadable)?;
            if let Some(token) = reply.access_token {
                return Ok(token);
            }
            match reply.error.as_deref() {
                Some("authorization_pending") => {}
                // GitHub sends the longer interval to use from now on.
                Some("slow_down") => interval = reply.interval.unwrap_or(interval + 5),
                Some("expired_token") => return Err(expired()),
                Some("access_denied") => {
                    return Err(Error::GitHub("sign-in was declined on GitHub".into()));
                }
                other => {
                    let detail = reply.error_description.as_deref().or(other);
                    return Err(Error::GitHub(format!(
                        "GitHub sign-in failed: {}",
                        detail.unwrap_or("no reason given")
                    )));
                }
            }
        }
    }

    /// The account the token belongs to.
    pub fn login(&self, token: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct User {
            login: String,
        }
        let user: User = self.api_call(token, "/user", None)?;
        Ok(user.login)
    }

    /// A new private repo under the signed-in account.
    pub fn create_repo(&self, token: &str, name: &str) -> Result<RepoInfo> {
        let body = serde_json::json!({ "name": name, "private": true });
        self.api_call(token, "/user/repos", Some(body))
    }

    /// POST `body` when there is one, otherwise GET.
    fn api_call<T: serde::de::DeserializeOwned>(
        &self,
        token: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T> {
        let url = format!("{}{path}", self.api);
        let auth = format!("Bearer {token}");
        let accept = "application/vnd.github+json";
        let reply = match body {
            Some(body) => self
                .agent
                .post(&url)
                .header("Accept", accept)
                .header("Authorization", &auth)
                .send_json(body),
            None => self
                .agent
                .get(&url)
                .header("Accept", accept)
                .header("Authorization", &auth)
                .call(),
        };
        let mut reply = reply.map_err(unreachable)?;
        if reply.status().is_success() {
            return reply.body_mut().read_json().map_err(unreadable);
        }
        let status = reply.status();
        let detail: serde_json::Value = reply.body_mut().read_json().unwrap_or_default();
        let message = detail["errors"][0]["message"]
            .as_str()
            .or(detail["message"].as_str())
            .unwrap_or("no reason given");
        Err(Error::GitHub(format!("GitHub: {message} ({status})")))
    }
}

fn expired() -> Error {
    Error::GitHub("the code expired before it was entered".into())
}

fn unreachable(err: ureq::Error) -> Error {
    Error::GitHub(format!("cannot reach GitHub: {err}"))
}

fn unreadable(err: ureq::Error) -> Error {
    Error::GitHub(format!("unexpected reply from GitHub: {err}"))
}

/// Whether the user's SSH setup authenticates to GitHub without prompting.
pub fn ssh_works() -> bool {
    let mut cmd = std::process::Command::new("ssh");
    cmd.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "git@github.com",
    ]);
    crate::process::output_with_timeout(&mut cmd, Duration::from_secs(20))
        .ok()
        .flatten()
        .is_some_and(|output| {
            String::from_utf8_lossy(&output.stderr).contains("successfully authenticated")
        })
}

/// How git reaches a GitHub repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    Ssh,
    /// With rosamta's credential helper, run as `<helper> git-credential`.
    Https {
        helper: String,
    },
}

impl Transport {
    fn url(&self, full_name: &str) -> String {
        match self {
            Transport::Ssh => format!("git@github.com:{full_name}.git"),
            Transport::Https { .. } => format!("https://github.com/{full_name}.git"),
        }
    }

    /// `git -c` settings that make the helper the only one for GitHub.
    fn config(&self) -> Vec<String> {
        match self {
            Transport::Ssh => Vec::new(),
            Transport::Https { helper } => vec![
                format!("{HELPER_KEY}="),
                format!("{HELPER_KEY}={}", helper_command(helper)),
            ],
        }
    }
}

/// `!'/path/to/rosamta' git-credential`, quoted for the shell git runs it with.
fn helper_command(program: &str) -> String {
    format!("!'{}' git-credential", program.replace('\'', r"'\''"))
}

/// Add rosamta's credential helper to this repo only. The empty value first
/// keeps helpers from the user's global config out of GitHub requests.
pub fn configure_https(repo_dir: &Path, helper: &str) -> Result<()> {
    remove_helper(repo_dir)?;
    git::run(repo_dir, &["config", "--local", "--add", HELPER_KEY, ""])?;
    let command = helper_command(helper);
    git::run(
        repo_dir,
        &["config", "--local", "--add", HELPER_KEY, &command],
    )?;
    Ok(())
}

/// Undo [`configure_https`]; returns whether there was anything to remove.
pub fn remove_helper(repo_dir: &Path) -> Result<bool> {
    if !repo_dir.join(".git").exists()
        || git::run(repo_dir, &["config", "--local", "--get-all", HELPER_KEY]).is_err()
    {
        return Ok(false);
    }
    git::run(repo_dir, &["config", "--local", "--unset-all", HELPER_KEY])?;
    Ok(true)
}

/// `rsmt init --github <name>`: a new private repo under the signed-in
/// account, set as the remote of a new local repo. Pushing is left to the caller.
pub fn init_new(
    paths: &Paths,
    github: &GitHub,
    token: &str,
    name: &str,
    transport: &Transport,
) -> Result<InitOutcome> {
    // Local steps fail first: once the GitHub repo exists, a retry stops at
    // "name already exists".
    let outcome = repo::init_new(paths)?;
    let created = match github.create_repo(token, name) {
        Ok(created) => created,
        Err(err) => {
            // Only what `repo::init_new` just made, so a retry starts clean.
            let _ = std::fs::remove_dir_all(&outcome.repo);
            return Err(err);
        }
    };
    let url = transport.url(&created.full_name);
    git::run(&outcome.repo, &["remote", "add", "origin", &url])?;
    if let Transport::Https { helper } = transport {
        configure_https(&outcome.repo, helper)?;
    }
    Ok(outcome)
}

/// `rsmt init --github <owner>/<name>`: clone an existing repo.
pub fn join(paths: &Paths, full_name: &str, transport: &Transport) -> Result<InitOutcome> {
    let config = transport.config();
    let config: Vec<&str> = config.iter().map(String::as_str).collect();
    let outcome = repo::join_with_config(paths, &transport.url(full_name), &config)?;
    if let Transport::Https { helper } = transport {
        configure_https(&outcome.repo, helper)?;
    }
    Ok(outcome)
}

/// The token for a credential helper request from git, as `key=value` lines;
/// empty when the request is not for GitHub over HTTPS or there is no token.
pub fn credential_reply(request: &str, token: Option<&str>) -> String {
    let field = |key: &str| {
        request
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
    };
    match token {
        Some(token)
            if field("protocol") == Some("https") && field("host") == Some("github.com") =>
        {
            format!("username=x-access-token\npassword={token}\n")
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests;
