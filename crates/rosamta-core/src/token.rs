//! Where the GitHub token lives: the system keychain (macOS Keychain, Linux
//! Secret Service), or a file only the user can read when there is none.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::paths::Paths;

const SERVICE: &str = "rosamta";
const ACCOUNT: &str = "github.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    Keychain,
    /// No keychain was available; the file has mode 0600.
    File(PathBuf),
}

pub struct TokenStore {
    file: PathBuf,
    keychain: bool,
}

impl TokenStore {
    /// `ROSAMTA_NO_KEYCHAIN` keeps the token in the file, as on a host
    /// without a keychain; tests use it to stay out of the real one.
    pub fn new(paths: &Paths) -> Self {
        Self {
            file: paths.github_token(),
            keychain: std::env::var_os("ROSAMTA_NO_KEYCHAIN").is_none(),
        }
    }

    fn entry(&self) -> Option<keyring::Entry> {
        self.keychain
            .then(|| keyring::Entry::new(SERVICE, ACCOUNT).ok())
            .flatten()
    }

    pub fn save(&self, token: &str) -> Result<Stored> {
        if let Some(entry) = self.entry()
            && entry.set_password(token).is_ok()
        {
            remove(&self.file)?;
            return Ok(Stored::Keychain);
        }
        crate::edit::write_private(&self.file, &format!("{token}\n"))?;
        Ok(Stored::File(self.file.clone()))
    }

    pub fn load(&self) -> Result<Option<String>> {
        if let Some(token) = self.entry().and_then(|entry| entry.get_password().ok()) {
            return Ok(Some(token));
        }
        match std::fs::read_to_string(&self.file) {
            Ok(text) => Ok(Some(text.trim().to_string()).filter(|token| !token.is_empty())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(Error::Read {
                path: self.file.clone(),
                source: err,
            }),
        }
    }

    /// Returns whether there was a token to delete.
    pub fn delete(&self) -> Result<bool> {
        let in_keychain = self
            .entry()
            .is_some_and(|entry| entry.delete_credential().is_ok());
        Ok(remove(&self.file)? || in_keychain)
    }
}

fn remove(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(Error::Write {
            path: path.to_path_buf(),
            source: err,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn file_store_is_private_and_deletable() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore {
            file: dir.path().join("rosamta/github-token"),
            keychain: false,
        };
        assert_eq!(store.load().unwrap(), None);
        assert_eq!(
            store.save("gho_x").unwrap(),
            Stored::File(store.file.clone())
        );
        let mode = std::fs::metadata(&store.file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(store.load().unwrap().as_deref(), Some("gho_x"));
        assert!(store.delete().unwrap());
        assert_eq!(store.load().unwrap(), None);
        assert!(!store.delete().unwrap());
    }
}

#[cfg(test)]
mod keychain {
    /// Writes, reads and deletes a throwaway item in the real keychain.
    /// Needs the real HOME: macOS finds the default keychain through it.
    #[test]
    #[ignore = "touches the system keychain"]
    fn round_trip() {
        let entry = keyring::Entry::new("rosamta-probe", "probe").unwrap();
        entry.set_password("probe-value").unwrap();
        assert_eq!(entry.get_password().unwrap(), "probe-value");
        entry.delete_credential().unwrap();
    }
}
