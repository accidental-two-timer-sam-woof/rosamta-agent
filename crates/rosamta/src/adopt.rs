//! `rsmt adopt`, `rsmt review` and `rsmt restore <id>`.

use std::io::IsTerminal;
use std::process::ExitCode;

use rosamta_core::Result;
use rosamta_core::adopt::{self, Adoption, Candidate, Kind, QueueItem, Scope};
use rosamta_core::apply::Context;
use rosamta_core::dotfiles::review::{self, Reason};
use rosamta_core::hooks::FileChange;
use rosamta_core::scripts::Run;
use rosamta_core::scripts::apply::Why;
use rosamta_core::{scripts, time};

use crate::commands::confirm;
use crate::print_json;
use crate::sync::after_commit;

pub fn adopt(json: bool, yes: bool) -> Result<ExitCode> {
    let started = time::now();
    let ctx = Context::from_env()?;
    let lock = adopt::lock(&ctx)?;
    let candidates = adopt::scan(&ctx)?;
    if candidates.is_empty() {
        if json {
            print_json(&Vec::<Adoption>::new());
        } else {
            println!("Nothing to adopt");
        }
        return Ok(ExitCode::SUCCESS);
    }
    let plan = adopt::plan(&ctx, &candidates, Scope::Host)?;
    if !json && !yes {
        for candidate in &candidates {
            println!("{}", describe(&ctx, candidate));
        }
    }
    let mut changes = plan.files.clone();
    changes.extend(plan.modules.iter().map(|(path, text)| FileChange {
        path: path.clone(),
        before: None,
        after: Some(text.clone()),
    }));
    changes.extend(plan.edits.iter().cloned());
    if !confirm(json, yes, &changes)? {
        return Ok(ExitCode::SUCCESS);
    }
    let done = adopt::execute(&ctx, plan)?;
    report(&ctx, json, &done);
    after_commit(&ctx, &lock, started, json);
    Ok(ExitCode::SUCCESS)
}

/// `~/.zshrc:12-14 → modules/adopted/bun.toml (this host only)`.
fn describe(ctx: &Context, candidate: &Candidate) -> String {
    let lines = format!(
        "{}:{}-{}",
        adopt::display(&candidate.file, &ctx.home),
        candidate.start,
        candidate.end
    );
    let module = candidate
        .module
        .as_deref()
        .map(|module| adopt::display(module, &ctx.home))
        .unwrap_or_default();
    match candidate.kind {
        Kind::Comment => format!("{lines} → removed, comments only"),
        Kind::Duplicate => format!("{lines} → removed, already in {module}"),
        Kind::Include => format!("{lines} → {module} (adds this host)"),
        _ if candidate.local => format!("{lines} → {module} (this host only)"),
        _ => format!("{lines} → {module}"),
    }
}

fn report(ctx: &Context, json: bool, done: &[Adoption]) {
    if json {
        print_json(&done);
        return;
    }
    for adoption in done {
        let file = adopt::display(&adoption.file, &ctx.home);
        match (&adoption.module, adoption.id) {
            (Some(module), id) if id > 0 => println!(
                "Adopted {id}: {file} → {}",
                adopt::display(module, &ctx.home)
            ),
            _ => println!("Removed {file}:{}", adoption.line),
        }
    }
}

pub fn review(json: bool) -> Result<ExitCode> {
    let started = time::now();
    let ctx = Context::from_env()?;
    let lock = adopt::lock(&ctx)?;
    let reviewed = adopt::refresh(&ctx)?;
    if !reviewed.adopted.is_empty() {
        after_commit(&ctx, &lock, started, json);
    }
    if json {
        print_json(&reviewed.items);
        return Ok(ExitCode::SUCCESS);
    }
    report(&ctx, false, &reviewed.adopted);
    if reviewed.items.is_empty() {
        println!("Nothing to review");
    }
    for item in &reviewed.items {
        println!("{}", item_line(&ctx, item));
    }
    Ok(ExitCode::SUCCESS)
}

/// `3  ~/.zshrc:12-14  2026-11-10 14:30 UTC  modules/adopted/bun.toml`,
/// `4  ~/.gitconfig  2026-11-10 14:31 UTC  differs from the repo`, or what a
/// template waits for.
fn item_line(ctx: &Context, item: &QueueItem) -> String {
    if let Some(script) = &item.script {
        let run = match script.run {
            Run::Once => "once",
            Run::Onchange => "onchange",
        };
        let why = match script.why {
            Why::New => "never ran on this host",
            Why::Changed => "changed since it last ran",
            Why::Failed => "failed last time",
        };
        return format!(
            "{}  {} ({run})  {}  {why}",
            item.id,
            script.source,
            time::date_time(item.detected_at)
        );
    }
    if let Some(dotfile) = &item.dotfile {
        let what = match (&dotfile.waiting, dotfile.reason) {
            (Some(waiting), _) => waiting.describe(),
            (None, Reason::Changed) => "changed on this host".into(),
            (None, _) => "differs from the repo".into(),
        };
        return format!(
            "{}  {}  {}  {what}",
            item.id,
            adopt::display(&item.file, &ctx.home),
            time::date_time(item.detected_at)
        );
    }
    let target = match &item.module {
        Some(module) if item.local => {
            format!("{} (this host only)", adopt::display(module, &ctx.home))
        }
        // Only a module another host adopted exists before accepting.
        Some(module) if ctx.paths.repo().join(module).exists() => {
            format!("{} (adds this host)", adopt::display(module, &ctx.home))
        }
        Some(module) => adopt::display(module, &ctx.home),
        None => "comments only".into(),
    };
    format!(
        "{}  {}:{}-{}  {}  {target}",
        item.id,
        adopt::display(&item.file, &ctx.home),
        item.start,
        item.end,
        time::date_time(item.detected_at)
    )
}

pub fn accept(json: bool, id: u32, all_hosts: bool) -> Result<ExitCode> {
    let started = time::now();
    let mut ctx = Context::from_env()?;
    let lock = adopt::lock(&ctx)?;
    if let Some(script) = scripts::review::item_of(&ctx, id)? {
        // A script run from the queue gets the terminal as `rsmt apply` does.
        ctx.interactive = !json && std::io::stdin().is_terminal();
        let outcome = scripts::review::accept(&ctx, id, &script)?;
        if json {
            print_json(&serde_json::json!({
                "source": script.source,
                "ok": outcome.ok,
                "status": outcome.status,
            }));
        } else if outcome.ok {
            println!("Ran {}", script.source);
        } else {
            eprintln!("error: script {}: {}", script.source, outcome.status);
        }
        return Ok(if outcome.ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    let scope = if all_hosts { Scope::All } else { Scope::Host };
    let dotfile = adopt::queue(&ctx)?
        .into_iter()
        .find(|item| item.id == id && item.dotfile.is_some());
    let done = adopt::accept(&ctx, id, scope)?;
    match dotfile {
        Some(item) if json => print_json(&[&item.file]),
        Some(item) => println!("Kept {} in the repo", adopt::display(&item.file, &ctx.home)),
        None => report(&ctx, json, &done),
    }
    after_commit(&ctx, &lock, started, json);
    Ok(ExitCode::SUCCESS)
}

pub fn ignore(json: bool, id: u32) -> Result<ExitCode> {
    let ctx = Context::from_env()?;
    let _lock = adopt::lock(&ctx)?;
    let item = adopt::ignore(&ctx, id)?;
    if json {
        print_json(&item);
    } else {
        println!("Ignored {id}");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn discard(json: bool, id: u32) -> Result<ExitCode> {
    let mut ctx = Context::from_env()?;
    // Rendering the repo's version may unlock a tool or run a secret command.
    ctx.interactive = !json && std::io::stdin().is_terminal();
    let _lock = adopt::lock(&ctx)?;
    let path = ctx.home.join(review::discard(&ctx, id)?);
    if json {
        print_json(&[&path]);
    } else {
        println!(
            "Replaced {} with the repo's version",
            adopt::display(&path, &ctx.home)
        );
    }
    Ok(ExitCode::SUCCESS)
}

pub fn restore(json: bool, yes: bool, id: u32) -> Result<ExitCode> {
    let started = time::now();
    let ctx = Context::from_env()?;
    let lock = adopt::lock(&ctx)?;
    let undo = adopt::plan_undo(&ctx, id)?;
    let mut changes = vec![undo.file.clone()];
    if let Some((path, text)) = &undo.module {
        changes.push(FileChange {
            path: path.clone(),
            before: Some(text.clone()),
            after: undo.module_after.clone(),
        });
    }
    if !confirm(json, yes, &changes)? {
        return Ok(ExitCode::SUCCESS);
    }
    let file = undo.file.path.clone();
    adopt::undo(&ctx, undo)?;
    if json {
        print_json(&[&file]);
    } else {
        println!("Restored {}", adopt::display(&file, &ctx.home));
    }
    after_commit(&ctx, &lock, started, json);
    Ok(ExitCode::SUCCESS)
}
