//! `rsmt adopt`, `rsmt review` and `rsmt restore <id>`.

use std::process::ExitCode;

use rosamta_core::Result;
use rosamta_core::adopt::{self, Adoption, Candidate, Kind, QueueItem, Scope};
use rosamta_core::apply::Context;
use rosamta_core::hooks::FileChange;
use rosamta_core::time;

use crate::commands::confirm;
use crate::print_json;

pub fn adopt(json: bool, yes: bool) -> Result<ExitCode> {
    let ctx = Context::from_env()?;
    let _lock = adopt::lock(&ctx)?;
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
    if !confirm(json, yes, &changes)? {
        return Ok(ExitCode::SUCCESS);
    }
    let done = adopt::execute(&ctx, plan)?;
    report(&ctx, json, &done);
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
    let ctx = Context::from_env()?;
    let _lock = adopt::lock(&ctx)?;
    let reviewed = adopt::refresh(&ctx)?;
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

/// `3  ~/.zshrc:12-14  2026-11-10 14:30 UTC  modules/adopted/bun.toml`.
fn item_line(ctx: &Context, item: &QueueItem) -> String {
    let target = match &item.module {
        Some(module) if item.local => {
            format!("{} (this host only)", adopt::display(module, &ctx.home))
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
    let ctx = Context::from_env()?;
    let _lock = adopt::lock(&ctx)?;
    let scope = if all_hosts { Scope::All } else { Scope::Host };
    let done = adopt::accept(&ctx, id, scope)?;
    report(&ctx, json, &done);
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

pub fn restore(json: bool, yes: bool, id: u32) -> Result<ExitCode> {
    let ctx = Context::from_env()?;
    let _lock = adopt::lock(&ctx)?;
    let undo = adopt::plan_undo(&ctx, id)?;
    let mut changes = vec![undo.file.clone()];
    if let Some((path, text)) = &undo.module {
        changes.push(FileChange {
            path: path.clone(),
            before: Some(text.clone()),
            after: None,
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
    Ok(ExitCode::SUCCESS)
}
