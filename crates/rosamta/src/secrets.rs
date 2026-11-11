//! `rsmt secrets`: credentials of secret sources.

use std::process::ExitCode;

use rosamta_core::paths::Paths;
use rosamta_core::token::{Stored, TokenStore};
use rosamta_core::{Error, Result};

use crate::print_json;

pub fn bws_login(json: bool) -> Result<ExitCode> {
    let paths = Paths::from_env()?;
    let token = rpassword::prompt_password("Access token: ").map_err(|err| Error::Read {
        path: "/dev/tty".into(),
        source: err,
    })?;
    let token = token.trim();
    if token.is_empty() {
        return Err(Error::Usage("no access token entered".into()));
    }
    let stored = TokenStore::bws(&paths).save(token)?;
    if let Stored::File(path) = &stored {
        eprintln!(
            "warning: no keychain available; the token is in {}, readable only by you",
            path.display()
        );
    }
    if json {
        let place = match stored {
            Stored::Keychain => "keychain",
            Stored::File(_) => "file",
        };
        print_json(&serde_json::json!({ "stored": place }));
    } else {
        println!("Saved the Bitwarden Secrets Manager access token");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn bws_logout(json: bool) -> Result<ExitCode> {
    let deleted = TokenStore::bws(&Paths::from_env()?).delete()?;
    if json {
        print_json(&serde_json::json!({ "deleted": deleted }));
    } else if deleted {
        println!("Deleted the Bitwarden Secrets Manager access token");
    } else {
        println!("No Bitwarden Secrets Manager access token was stored");
    }
    Ok(ExitCode::SUCCESS)
}
