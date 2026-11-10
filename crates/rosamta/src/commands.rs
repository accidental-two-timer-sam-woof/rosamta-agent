//! Commands that change shell startup: apply, hook, unhook, eject, restore.

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

use rosamta_core::apply::{self, Context};
use rosamta_core::config::Shell;
use rosamta_core::hooks::FileChange;
use rosamta_core::{Error, Result};

use crate::print_json;

pub fn apply(json: bool, dry_run: bool) -> Result<ExitCode> {
    let ctx = Context::from_env()?;
    if dry_run {
        let plan = apply::plan(&ctx)?;
        warn_skipped(&plan.skipped);
        if json {
            print_json(&plan);
        } else if plan.is_up_to_date() {
            println!("Already up to date");
        } else {
            print!("{}", apply::diff(&plan, &ctx.paths.generated()));
        }
        return Ok(ExitCode::SUCCESS);
    }
    let applied = apply::apply(&ctx)?;
    warn_skipped(&applied.skipped);
    if json {
        print_json(&applied);
    } else if applied.up_to_date {
        println!("Already up to date");
    } else {
        let shells: Vec<&str> = applied.shells.iter().map(|shell| shell.name()).collect();
        println!(
            "Applied {} for {}",
            applied.version.as_deref().unwrap_or_default(),
            shells.join(", ")
        );
        for path in &applied.hooks_changed {
            println!("Hooked {}", path.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn warn_skipped(skipped: &[String]) {
    for reason in skipped {
        eprintln!("warning: {reason}");
    }
}

pub fn hook(json: bool, shells: &[Shell]) -> Result<ExitCode> {
    let ctx = Context::from_env()?;
    let shells = if shells.is_empty() {
        Shell::ALL.to_vec()
    } else {
        shells.to_vec()
    };
    let changed = apply::hook(&ctx, &shells)?;
    report_paths(json, "Hooked", &changed);
    Ok(ExitCode::SUCCESS)
}

pub fn unhook(json: bool, shell: Shell) -> Result<ExitCode> {
    let ctx = Context::from_env()?;
    let changed = apply::unhook(&ctx, shell)?;
    report_paths(json, "Unhooked", &changed);
    Ok(ExitCode::SUCCESS)
}

pub fn eject(json: bool, yes: bool) -> Result<ExitCode> {
    let ctx = Context::from_env()?;
    let changes = apply::plan_eject(&ctx)?;
    if !confirm(json, yes, &changes)? {
        return Ok(ExitCode::SUCCESS);
    }
    apply::eject(&ctx, &changes)?;
    let paths: Vec<_> = changes.into_iter().map(|change| change.path).collect();
    report_paths(json, "Ejected", &paths);
    Ok(ExitCode::SUCCESS)
}

pub fn restore(json: bool, yes: bool) -> Result<ExitCode> {
    let ctx = Context::from_env()?;
    let changes = apply::plan_restore(&ctx)?;
    if changes.is_empty() {
        if json {
            print_json(&Vec::<String>::new());
        } else {
            println!("Nothing to restore");
        }
        return Ok(ExitCode::SUCCESS);
    }
    if !confirm(json, yes, &changes)? {
        return Ok(ExitCode::SUCCESS);
    }
    let restored = apply::restore_original(&ctx)?;
    report_paths(json, "Restored", &restored);
    Ok(ExitCode::SUCCESS)
}

/// Show the diff and ask, unless `--yes`. Without a terminal, `--yes` is required.
pub(crate) fn confirm(json: bool, yes: bool, changes: &[FileChange]) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if json || !std::io::stdin().is_terminal() {
        return Err(Error::Usage(
            "add --yes to confirm when not running in a terminal".into(),
        ));
    }
    for change in changes {
        let name = change.path.display().to_string();
        print!(
            "{}",
            apply::unified(&name, change.before.as_deref(), change.after.as_deref())
        );
    }
    print!("Write these changes? [y/N] ");
    std::io::stdout().flush().map_err(|err| Error::Write {
        path: "stdout".into(),
        source: err,
    })?;
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|err| Error::Read {
            path: "stdin".into(),
            source: err,
        })?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

fn report_paths(json: bool, verb: &str, paths: &[std::path::PathBuf]) {
    if json {
        print_json(&paths);
    } else {
        for path in paths {
            println!("{verb} {}", path.display());
        }
    }
}
