//! Encrypted dotfiles (ADR 0012). One age X25519 key encrypts them all. The
//! repo holds it as `key.age`, encrypted with the user's passphrase; each
//! host keeps it unlocked in `$XDG_CONFIG_HOME/rosamta/age.key`.

use std::str::FromStr;

use age::secrecy::{ExposeSecret, SecretString};
use age::x25519;

use crate::apply::Context;
use crate::error::{Error, Result};

pub const REPO_KEY: &str = "key.age";

/// This host's key; `None` until it is created or unlocked here.
fn identity(ctx: &Context) -> Result<Option<x25519::Identity>> {
    let path = ctx.paths.age_key();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Error::Read { path, source: err }),
    };
    x25519::Identity::from_str(text.trim())
        .map(Some)
        .map_err(|err| Error::Usage(format!("{}: {err}", path.display())))
}

pub fn repo_has_key(ctx: &Context) -> bool {
    ctx.paths.repo().join(REPO_KEY).is_file()
}

/// Whether encrypted dotfiles wait for the passphrase on this host.
pub fn locked(ctx: &Context) -> Result<bool> {
    Ok(repo_has_key(ctx) && identity(ctx)?.is_none())
}

/// Make the key, protect it with `passphrase` in the repo, and keep it
/// unlocked on this host.
pub fn create(ctx: &Context, passphrase: &str) -> Result<()> {
    if repo_has_key(ctx) {
        return Err(Error::Usage("the repo already has a key".into()));
    }
    let key = x25519::Identity::generate();
    let secret = key.to_string();
    let recipient = age::scrypt::Recipient::new(SecretString::from(passphrase.to_string()));
    let sealed = age::encrypt_and_armor(&recipient, secret.expose_secret().as_bytes())
        .map_err(|err| Error::Usage(format!("cannot encrypt the key: {err}")))?;
    crate::edit::write_private(
        &ctx.paths.age_key(),
        &format!("{}\n", secret.expose_secret()),
    )?;
    crate::edit::write_atomic(&ctx.paths.repo().join(REPO_KEY), &sealed)
}

/// Unlock the repo's key with `passphrase` and keep it on this host.
/// `false` when the passphrase is wrong.
pub fn unlock(ctx: &Context, passphrase: &str) -> Result<bool> {
    let path = ctx.paths.repo().join(REPO_KEY);
    let sealed = std::fs::read(&path).map_err(Error::read(&path))?;
    let identity = age::scrypt::Identity::new(SecretString::from(passphrase.to_string()));
    let secret = match age::decrypt(&identity, &sealed) {
        Ok(secret) => secret,
        Err(age::DecryptError::DecryptionFailed | age::DecryptError::NoMatchingKeys) => {
            return Ok(false);
        }
        Err(err) => {
            return Err(Error::Usage(format!("{}: {err}", path.display())));
        }
    };
    let text = String::from_utf8(secret)
        .map_err(|_| Error::Usage(format!("{} does not hold an age key", path.display())))?;
    x25519::Identity::from_str(text.trim())
        .map_err(|err| Error::Usage(format!("{}: {err}", path.display())))?;
    crate::edit::write_private(&ctx.paths.age_key(), &format!("{}\n", text.trim()))?;
    Ok(true)
}

/// Encrypt for the repo, as armored text.
pub fn encrypt(ctx: &Context, plaintext: &[u8]) -> Result<Vec<u8>> {
    let key = identity(ctx)?.ok_or_else(|| {
        Error::Usage("the key for encrypted dotfiles is not unlocked on this host".into())
    })?;
    age::encrypt_and_armor(&key.to_public(), plaintext)
        .map(String::into_bytes)
        .map_err(|err| Error::Usage(format!("cannot encrypt: {err}")))
}

/// Decrypt what the repo holds; `None` when this host has no key yet.
pub fn decrypt(ctx: &Context, ciphertext: &[u8]) -> Result<Option<Vec<u8>>> {
    let Some(key) = identity(ctx)? else {
        return Ok(None);
    };
    age::decrypt(&key, ciphertext)
        .map(Some)
        .map_err(|err| Error::Usage(format!("cannot decrypt: {err}")))
}
