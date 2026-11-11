//! Commands that change shell startup: apply, hook, unhook, eject, restore.

use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use rosamta_core::adopt;
use rosamta_core::apply::{self, Context};
use rosamta_core::config::Shell;
use rosamta_core::daemon::service;
use rosamta_core::dotfiles::{self, Action, Step};
use rosamta_core::externals;
use rosamta_core::hooks::FileChange;
use rosamta_core::scripts::apply as scripts;
use rosamta_core::{Error, Result};

use crate::print_json;

pub fn apply(json: bool, dry_run: bool) -> Result<ExitCode> {
    let mut ctx = Context::from_env()?;
    // Scripts run attached in a terminal; anywhere else they follow host.toml.
    let interactive = !json && std::io::stdin().is_terminal();
    if dry_run {
        let plan = apply::plan(&ctx)?;
        let externals = externals::plan(&ctx)?;
        let dotfiles = dotfiles::plan(&ctx)?;
        // Only for scripts: a dry run never unlocks secret sources.
        ctx.interactive = interactive;
        let scripts = scripts::plan(&ctx)?;
        warn_skipped(&plan.skipped);
        if json {
            let steps: Vec<&Step> = dotfiles.iter().map(|planned| &planned.step).collect();
            print_json(&DryRun {
                plan: &plan,
                externals: &externals,
                dotfiles: steps,
                scripts: &scripts,
            });
            return Ok(ExitCode::SUCCESS);
        }
        let changed: Vec<_> = dotfiles
            .iter()
            .filter(|planned| planned.step.action != Action::Unchanged)
            .collect();
        if plan.is_up_to_date() && changed.is_empty() && settled(&externals) && scripts.is_empty() {
            println!("Already up to date");
        }
        if !plan.is_up_to_date() {
            print!("{}", apply::diff(&plan, &ctx.paths.generated()));
        }
        for step in &externals {
            print_external(&ctx, step);
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
        for step in &scripts {
            print_script(step);
        }
        return Ok(ExitCode::SUCCESS);
    }
    // Asked before taking the lock, so the daemon is not held up while the
    // user types. Without a terminal, encrypted dotfiles are only skipped.
    let wrong_passphrase =
        interactive && dotfiles::needs_key(&ctx)? && !crate::dotfiles::unlock(&ctx, json)?;
    if interactive {
        crate::dotfiles::ask_vars(&ctx)?;
    }
    ctx.interactive = interactive;
    // An external or a `before` script may set up a secret source, so
    // templates then wait for it, under the lock.
    let prepared =
        (interactive && !apply::setup_pending(&ctx)).then(|| dotfiles::deploy::prepare(&ctx));
    let applied = {
        let _lock = adopt::lock(&ctx)?;
        if let Some(prepared) = prepared {
            dotfiles::deploy::keep_prepared(&mut ctx, prepared);
        }
        apply::apply(&ctx)?
    };
    warn_skipped(&applied.skipped);
    warn_chezmoi(&ctx, &applied.hooks_changed);
    let failed = wrong_passphrase || !applied.failures().is_empty();
    if json {
        print_json(&applied);
    } else {
        let changed: Vec<&Step> = applied
            .dotfiles
            .iter()
            .filter(|step| step.action != Action::Unchanged)
            .collect();
        if applied.up_to_date
            && changed.is_empty()
            && settled(&applied.externals)
            && applied.scripts.is_empty()
        {
            println!("Already up to date");
        }
        for step in &applied.externals {
            print_external(&ctx, step);
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
        for step in &applied.scripts {
            print_script(step);
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
        // Taken per answer: the daemon is not held up while the user reads.
        let _lock = adopt::lock(ctx)?;
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
        let lock = adopt::lock(ctx)?;
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
    externals: &'a [externals::Step],
    dotfiles: Vec<&'a Step>,
    scripts: &'a [scripts::Step],
}

fn print_script(step: &scripts::Step) {
    use scripts::{Action, Why};
    let source = &step.source;
    let why = match step.why {
        Some(Why::New) => "never ran on this host",
        Some(Why::Changed) => "changed since it last ran",
        Some(Why::Failed) => "failed last time",
        None => "",
    };
    match step.action {
        Action::Ran => println!("Ran {source}"),
        Action::WillRun => println!("Will run {source}: {why}"),
        Action::Queued => println!("Queued {source} for review: {why}"),
        Action::Failed => eprintln!(
            "error: script {source}: {}",
            step.reason.as_deref().unwrap_or_default()
        ),
    }
}

pub(crate) fn settled(steps: &[externals::Step]) -> bool {
    steps
        .iter()
        .all(|step| step.action == externals::Action::Unchanged)
}

pub(crate) fn print_external(ctx: &Context, step: &externals::Step) {
    use externals::Action;
    let path = adopt::display(&ctx.home.join(&step.path), &ctx.home);
    let reason = step.reason.as_deref().unwrap_or_default();
    match (step.action, reason) {
        (Action::Unchanged, _) => {}
        (Action::Downloaded, _) => println!("Downloaded {path}"),
        (Action::Updated, "") => println!("Updated {path}"),
        (Action::Updated, reason) => println!("Updated {path}: {reason}"),
        (Action::Released, _) => println!("Released {path}"),
        (Action::Changed, reason) => println!("Not updated {path}: {reason}"),
        (Action::Conflict, reason) => println!("Not downloaded {path}: {reason}"),
        (Action::Skipped, reason) => println!("Skipped {path}: {reason}"),
        (Action::Failed, reason) => eprintln!("error: {path}: {reason}"),
    }
}

fn print_step(ctx: &Context, step: &Step) {
    let path = adopt::display(&ctx.home.join(&step.path), &ctx.home);
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

/// An rc file chezmoi also manages loses rosamta's block at the next
/// `chezmoi apply`. Asked only about the files this apply hooked: chezmoi
/// renders their templates to answer, which can run a password manager.
fn warn_chezmoi(ctx: &Context, hooked: &[PathBuf]) {
    if hooked.is_empty() {
        return;
    }
    for path in rosamta_core::chezmoi::drops_block(ctx, hooked) {
        let path = adopt::display(&path, &ctx.home);
        eprintln!(
            "warning: chezmoi also manages {path}, so `chezmoi apply` will remove rosamta's block; run `chezmoi forget {path}`, or move everything over with `rsmt import chezmoi`"
        );
    }
}

pub(crate) fn warn_skipped(skipped: &[String]) {
    for reason in skipped {
        eprintln!("warning: {reason}");
    }
}

pub fn hook(json: bool, shells: &[Shell]) -> Result<ExitCode> {
    let ctx = Context::from_env()?;
    let _lock = adopt::lock(&ctx)?;
    let changed = apply::hook(&ctx, shells)?;
    report_paths(json, "Hooked", &changed);
    Ok(ExitCode::SUCCESS)
}

pub fn unhook(json: bool, shell: Shell) -> Result<ExitCode> {
    let ctx = Context::from_env()?;
    let _lock = adopt::lock(&ctx)?;
    let changed = apply::unhook(&ctx, shell)?;
    report_paths(json, "Unhooked", &changed);
    Ok(ExitCode::SUCCESS)
}

pub fn eject(json: bool, yes: bool) -> Result<ExitCode> {
    // Before anything else: on macOS this may start again from rosamta.app.
    crate::daemon::from_app()?;
    let ctx = Context::from_env()?;
    let _lock = adopt::lock(&ctx)?;
    let changes = apply::plan_eject(&ctx)?;
    if !confirm(json, yes, &changes)? {
        return Ok(ExitCode::SUCCESS);
    }
    let released = apply::eject(&ctx, &changes)?;
    let disabled = service::disable(&ctx.paths)?;
    let mut paths: Vec<_> = changes.into_iter().map(|change| change.path).collect();
    paths.extend(released);
    report_paths(json, "Ejected", &paths);
    if disabled && !json {
        println!("The background service is disabled");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn restore(json: bool, yes: bool) -> Result<ExitCode> {
    // Before anything else: on macOS this may start again from rosamta.app.
    crate::daemon::from_app()?;
    let ctx = Context::from_env()?;
    let _lock = adopt::lock(&ctx)?;
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
    // Back to before rosamta: the daemon would hook the shells again.
    let disabled = service::disable(&ctx.paths)?;
    report_paths(json, "Restored", &restored);
    if disabled && !json {
        println!("The background service is disabled");
    }
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
