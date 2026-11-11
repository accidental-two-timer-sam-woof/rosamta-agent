//! `rsmt remote github`, `rsmt init --github` and the git credential helper.

use std::io::Read;
use std::process::{ExitCode, Stdio};

use rosamta_core::apply::Context;
use rosamta_core::github::{self, GitHub, Stored, TokenStore, Transport};
use rosamta_core::paths::Paths;
use rosamta_core::{Error, Result, adopt, sync, time};

use crate::print_json;

pub fn login(json: bool) -> Result<ExitCode> {
    let paths = Paths::from_env()?;
    let github = GitHub::default();
    let code = github.request_code()?;
    eprintln!(
        "Open {} and enter {}",
        code.verification_uri, code.user_code
    );
    open_browser(&code.verification_uri);
    let token = github.wait_for_token(&code)?;
    let login = github.login(&token)?;
    if let Stored::File(path) = TokenStore::new(&paths).save(&token)? {
        eprintln!(
            "warning: no keychain available; the token is in {}, readable only by you",
            path.display()
        );
    }
    if json {
        print_json(&serde_json::json!({ "login": login }));
    } else {
        println!("Signed in to GitHub as {login}");
    }
    Ok(ExitCode::SUCCESS)
}

fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    // The URL is printed too, so a host without a browser still works.
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub fn logout(json: bool) -> Result<ExitCode> {
    let paths = Paths::from_env()?;
    let token = TokenStore::new(&paths).delete()?;
    let helper = github::remove_helper(&paths.repo())?;
    if json {
        print_json(&serde_json::json!({ "signed_out": token || helper }));
    } else if token || helper {
        println!("Signed out of GitHub");
    } else {
        println!("Not signed in to GitHub");
    }
    Ok(ExitCode::SUCCESS)
}

/// `git credential` protocol: answer `get` with the stored token.
pub fn credential(operation: &str) -> Result<ExitCode> {
    let mut request = String::new();
    std::io::stdin()
        .read_to_string(&mut request)
        .map_err(|err| Error::Read {
            path: "stdin".into(),
            source: err,
        })?;
    if operation == "get" {
        let token = TokenStore::new(&Paths::from_env()?).load()?;
        print!("{}", github::credential_reply(&request, token.as_deref()));
    }
    Ok(ExitCode::SUCCESS)
}

/// `rsmt init --github <name>` creates a private repo; `<owner>/<name>` joins one.
pub fn init(json: bool, target: &str) -> Result<ExitCode> {
    let started = time::now();
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    let existing = target.split_once('/');
    if !existing.map_or(valid(target), |(owner, name)| valid(owner) && valid(name)) {
        return Err(Error::Usage(format!(
            "`{target}` is not a GitHub repo name or <owner>/<name>"
        )));
    }
    let paths = Paths::from_env()?;
    let token = TokenStore::new(&paths).load()?;
    let signed_out = || {
        Error::Usage(format!(
            "not signed in to GitHub; run `{} remote github login` first",
            crate::program_name()
        ))
    };
    if existing.is_none() && token.is_none() {
        return Err(signed_out());
    }
    let transport = if github::ssh_works() {
        Transport::Ssh
    } else {
        let exe = std::env::current_exe().map_err(|err| Error::Read {
            path: "the rosamta executable".into(),
            source: err,
        })?;
        Transport::Https {
            helper: exe.display().to_string(),
        }
    };
    if token.is_none() && transport != Transport::Ssh {
        return Err(signed_out());
    }
    let outcome = match (existing, &token) {
        (Some(_), _) => github::join(&paths, target, &transport)?,
        (None, Some(token)) => {
            github::init_new(&paths, &GitHub::default(), token, target, &transport)?
        }
        (None, None) => return Err(signed_out()),
    };
    crate::report_init(json, &outcome);
    let ctx = Context::from_env()?;
    let lock = adopt::lock(&ctx)?;
    if outcome.cloned {
        crate::sync::after_commit(&ctx, &lock, started, json);
    } else {
        drop(lock);
        // The new GitHub repo is empty until the first push.
        if let Err(err) = sync::publish(&ctx) {
            eprintln!("warning: {err}");
        }
    }
    Ok(ExitCode::SUCCESS)
}
