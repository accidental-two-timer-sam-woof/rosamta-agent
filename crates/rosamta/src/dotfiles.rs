//! `rsmt add`, `rsmt forget`, and asking for the passphrase of encrypted
//! dotfiles.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rosamta_core::apply::Context;
use rosamta_core::dotfiles::{crypt, manage};
use rosamta_core::{Error, Result, adopt, time};

use crate::print_json;
use crate::sync::after_commit;

const TRIES: usize = 3;

pub fn add(json: bool, paths: &[PathBuf], copy: bool, encrypt: bool) -> Result<ExitCode> {
    let started = time::now();
    let ctx = Context::from_env()?;
    let cwd = current_dir()?;
    let paths: Vec<PathBuf> = paths.iter().map(|path| cwd.join(path)).collect();
    let lock = adopt::lock(&ctx)?;
    if encrypt {
        ensure_key(&ctx, json)?;
    }
    let added = manage::add(&ctx, &paths, copy, encrypt)?;
    report(&ctx, json, "Added", &added);
    after_commit(&ctx, &lock, started, json);
    Ok(ExitCode::SUCCESS)
}

pub fn forget(json: bool, path: &Path) -> Result<ExitCode> {
    let started = time::now();
    let ctx = Context::from_env()?;
    let cwd = current_dir()?;
    let lock = adopt::lock(&ctx)?;
    let forgotten = manage::forget(&ctx, &cwd.join(path))?;
    report(&ctx, json, "Stopped managing", &[forgotten]);
    after_commit(&ctx, &lock, started, json);
    Ok(ExitCode::SUCCESS)
}

fn current_dir() -> Result<PathBuf> {
    std::env::current_dir().map_err(|err| Error::Read {
        path: ".".into(),
        source: err,
    })
}

fn report(ctx: &Context, json: bool, verb: &str, paths: &[PathBuf]) {
    let paths: Vec<PathBuf> = paths.iter().map(|rel| ctx.home.join(rel)).collect();
    if json {
        print_json(&paths);
        return;
    }
    for path in paths {
        println!("{verb} {}", adopt::display(&path, &ctx.home));
    }
}

/// Make the key on first use, or unlock the repo's key on this host.
fn ensure_key(ctx: &Context, json: bool) -> Result<()> {
    let interactive = !json && std::io::stdin().is_terminal();
    if crypt::repo_has_key(ctx) {
        if !crypt::locked(ctx)? || unlock(ctx, json)? {
            return Ok(());
        }
        return Err(Error::Usage(if interactive {
            "wrong passphrase".into()
        } else {
            "enter the passphrase for encrypted dotfiles in a terminal".into()
        }));
    }
    if !interactive {
        return Err(Error::Usage(
            "set the passphrase for encrypted dotfiles in a terminal".into(),
        ));
    }
    let first = ask("New passphrase for encrypted dotfiles: ")?;
    if first.is_empty() {
        return Err(Error::Usage("the passphrase is empty".into()));
    }
    if ask("Repeat the passphrase: ")? != first {
        return Err(Error::Usage("the passphrases do not match".into()));
    }
    crypt::create(ctx, &first)
}

/// Ask for the passphrase in a terminal, up to three times. `false` when
/// there is no terminal or every try was wrong.
pub(crate) fn unlock(ctx: &Context, json: bool) -> Result<bool> {
    if json || !std::io::stdin().is_terminal() {
        return Ok(false);
    }
    for _ in 0..TRIES {
        if crypt::unlock(ctx, &ask("Passphrase for encrypted dotfiles: ")?)? {
            return Ok(true);
        }
        eprintln!("Wrong passphrase");
    }
    Ok(false)
}

/// Ask for each `vars.*` a template needs and save it in `host.toml`; an
/// empty answer is saved too, so it is not asked again.
pub(crate) fn ask_vars(ctx: &Context) -> Result<()> {
    for name in rosamta_core::dotfiles::missing_vars(ctx)? {
        print!("vars.{name}: ");
        std::io::stdout().flush().map_err(|err| Error::Write {
            path: "stdout".into(),
            source: err,
        })?;
        let mut answer = String::new();
        let read = std::io::stdin()
            .lock()
            .read_line(&mut answer)
            .map_err(|err| Error::Read {
                path: "stdin".into(),
                source: err,
            })?;
        // End of input is no answer: the template waits instead.
        if read == 0 {
            println!();
            return Ok(());
        }
        let answer = answer.strip_suffix('\n').unwrap_or(&answer);
        rosamta_core::host::set_var(
            &ctx.paths,
            &name,
            answer.strip_suffix('\r').unwrap_or(answer),
        )?;
    }
    Ok(())
}

fn ask(prompt: &str) -> Result<String> {
    rpassword::prompt_password(prompt).map_err(|err| Error::Read {
        path: "/dev/tty".into(),
        source: err,
    })
}
