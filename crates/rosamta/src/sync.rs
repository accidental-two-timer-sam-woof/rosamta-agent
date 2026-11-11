//! `rsmt publish`, `rsmt pull` and `rsmt sync`.

use std::process::ExitCode;

use rosamta_core::Result;
use rosamta_core::adopt::Lock;
use rosamta_core::apply::Context;
use rosamta_core::sync::{self, Outcome, Published, Pulled};

use crate::commands::warn_skipped;
use crate::print_json;

pub fn publish(json: bool) -> Result<ExitCode> {
    let published = sync::publish(&Context::from_env()?)?;
    if json {
        print_json(&published);
    } else {
        report_published(&published);
    }
    Ok(ExitCode::SUCCESS)
}

pub fn pull(json: bool) -> Result<ExitCode> {
    let pulled = sync::pull(&Context::from_env()?)?;
    if json {
        print_json(&pulled);
    } else if pulled.commits == 0 {
        println!("Already up to date");
    } else {
        report_pulled(&pulled);
    }
    Ok(ExitCode::SUCCESS)
}

pub fn sync(json: bool) -> Result<ExitCode> {
    let published = sync::sync(&Context::from_env()?)?;
    if json {
        print_json(&published);
    } else if published.pulled.is_none() && published.pushed == 0 {
        println!("Already up to date");
    } else {
        report_published(&published);
    }
    Ok(ExitCode::SUCCESS)
}

fn report_published(published: &Published) {
    if let Some(pulled) = &published.pulled {
        report_pulled(pulled);
    }
    if published.pushed == 0 {
        if published.pulled.is_none() {
            println!("Nothing to publish");
        }
    } else {
        println!("Published {}", commits(published.pushed));
    }
}

fn report_pulled(pulled: &Pulled) {
    println!("Pulled {}", commits(pulled.commits));
    if let Some(applied) = &pulled.applied {
        warn_skipped(&applied.skipped);
        if let Some(version) = applied.version.as_deref().filter(|_| !applied.up_to_date) {
            println!("Applied {version}");
        }
    }
}

fn commits(count: u32) -> String {
    match count {
        1 => "1 commit".into(),
        n => format!("{n} commits"),
    }
}

/// After a command committed to the repo: push per the publish policy.
/// The command itself succeeded, so a failure here is only a warning.
pub(crate) fn after_commit(ctx: &Context, lock: &Lock, started: u64, json: bool) {
    match sync::publish_if_auto(ctx, lock) {
        Ok(Some(published)) if !json && published.pushed > 0 => {
            println!("Published {}", commits(published.pushed));
        }
        Ok(_) => {}
        Err(err) => return eprintln!("warning: {err}"),
    }
    // A commit that could not be made is recorded, not returned, and
    // older commits may have been pushed all the same.
    let blocked = sync::load_state(ctx.paths.state())
        .ok()
        .and_then(|state| state.publish)
        .filter(|attempt| attempt.at >= started && attempt.outcome == Outcome::Blocked);
    if let Some(reason) = blocked.and_then(|attempt| attempt.reason) {
        eprintln!("warning: {reason}");
    }
}
