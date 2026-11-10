//! Commands that change shell startup: apply, hook, unhook, eject, restore.

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

use rosamta_core::adopt;
use rosamta_core::apply::{self, Context};
use rosamta_core::config::Shell;
use rosamta_core::dotfiles::{self, Action, Step};
use rosamta_core::hooks::FileChange;
use rosamta_core::{Error, Result};

use crate::print_json;

pub fn apply(json: bool, dry_run: bool) -> Result<ExitCode> {
    let ctx = Context::from_env()?;
    if dry_run {
        let plan = apply::plan(&ctx)?;
        let dotfiles = dotfiles::plan(&ctx)?;
        warn_skipped(&plan.skipped);
        if json {
            let steps: Vec<&Step> = dotfiles.iter().map(|planned| &planned.step).collect();
            print_json(&DryRun {
                plan: &plan,
                dotfiles: steps,
            });
            return Ok(ExitCode::SUCCESS);
        }
        let changed: Vec<_> = dotfiles
            .iter()
            .filter(|planned| planned.step.action != Action::Unchanged)
            .collect();
        if plan.is_up_to_date() && changed.is_empty() {
            println!("Already up to date");
        }
        if !plan.is_up_to_date() {
            print!("{}", apply::diff(&plan, &ctx.paths.generated()));
        }
        for planned in changed {
            print_step(&ctx, &planned.step);
            if let Some((before, after)) = &planned.preview {
                let name = ctx.home.join(&planned.step.path).display().to_string();
                print!(
                    "{}",
                    apply::unified(&name, before.as_deref(), after.as_deref())
                );
            }
        }
        return Ok(ExitCode::SUCCESS);
    }
    let applied = {
        let _lock = adopt::lock(&ctx)?;
        apply::apply(&ctx)?
    };
    warn_skipped(&applied.skipped);
    let failed = applied
        .dotfiles
        .iter()
        .any(|step| step.action == Action::Failed);
    if json {
        print_json(&applied);
    } else {
        let changed: Vec<&Step> = applied
            .dotfiles
            .iter()
            .filter(|step| step.action != Action::Unchanged)
            .collect();
        if applied.up_to_date && changed.is_empty() {
            println!("Already up to date");
        }
        if !applied.up_to_date {
            let shells: Vec<&str> = applied.shells.iter().map(|shell| shell.name()).collect();
            println!(
                "Applied {} for {}",
                applied.version.as_deref().unwrap_or_default(),
                shells.join(", ")
            );
        }
        for path in &applied.hooks_changed {
            println!("Hooked {}", path.display());
        }
        for step in changed {
            print_step(&ctx, step);
        }
        if std::io::stdin().is_terminal() {
            settle_conflicts(&ctx, &applied.dotfiles)?;
        }
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// Ask about each conflict: the repo's version, this host's (kept for this
/// host alone), or later from `rsmt review`.
fn settle_conflicts(ctx: &Context, steps: &[Step]) -> Result<()> {
    let started = rosamta_core::time::now();
    let lock = adopt::lock(ctx)?;
    let mut kept = false;
    for step in steps.iter().filter(|step| step.action == Action::Conflict) {
        let item = adopt::queue(ctx)?.into_iter().find(|item| {
            item.dotfile
                .as_ref()
                .is_some_and(|dotfile| dotfile.path == step.path)
        });
        let Some((id, dotfile)) = item.and_then(|item| Some((item.id, item.dotfile?))) else {
            continue;
        };
        print!("{}", dotfiles::review::diff(ctx, &dotfile)?);
        let path = adopt::display(&ctx.home.join(&step.path), &ctx.home);
        let answer = ask(&format!(
            "Keep which version of {path}? [r]epo, [l]ocal, [s]kip: "
        ))?;
        match answer.as_str() {
            "r" | "repo" => {
                dotfiles::review::discard(ctx, id)?;
                println!("Replaced {path} with the repo's version");
            }
            "l" | "local" => {
                adopt::accept(ctx, id, adopt::Scope::Host)?;
                println!("Kept {path} in the repo for this host");
                kept = true;
            }
            _ => println!("Skipped {path}; run `rsmt review` to decide later"),
        }
    }
    if kept {
        crate::sync::after_commit(ctx, &lock, started, false);
    }
    Ok(())
}

fn ask(question: &str) -> Result<String> {
    print!("{question}");
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
    Ok(answer.trim().to_ascii_lowercase())
}

#[derive(serde::Serialize)]
struct DryRun<'a> {
    #[serde(flatten)]
    plan: &'a apply::Plan,
    dotfiles: Vec<&'a Step>,
}

fn print_step(ctx: &Context, step: &Step) {
    let path = ctx.home.join(&step.path);
    let path = path.display();
    let reason = step.reason.as_deref().unwrap_or_default();
    match step.action {
        Action::Unchanged => {}
        Action::Linked => println!("Linked {path}"),
        Action::Copied => println!("Copied {path}"),
        Action::Updated => println!("Updated {path}"),
        Action::Saved => println!("Saved {path} to the repo"),
        Action::Released => println!("Released {path}"),
        Action::Conflict => println!("Not deployed {path}: differs from the repo"),
        Action::Changed => println!("Not deployed {path}: changed on this host"),
        Action::Skipped => println!("Skipped {path}: {reason}"),
        Action::Failed => eprintln!("error: {path}: {reason}"),
    }
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
