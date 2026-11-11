//! `rsmt externals update`.

use std::path::PathBuf;
use std::process::ExitCode;

use rosamta_core::apply::Context;
use rosamta_core::dotfiles::manage;
use rosamta_core::externals::{self, Action, update};
use rosamta_core::{Error, Result, adopt, time};
use serde::Serialize;

use crate::commands::{print_external, settled};
use crate::print_json;
use crate::sync::after_commit;

#[derive(Serialize)]
struct Report {
    #[serde(flatten)]
    updated: update::Updated,
    externals: Vec<externals::Step>,
}

pub fn update(json: bool, paths: &[PathBuf]) -> Result<ExitCode> {
    let started = time::now();
    let ctx = Context::from_env()?;
    let cwd = std::env::current_dir().map_err(|err| Error::Read {
        path: ".".into(),
        source: err,
    })?;
    let only = paths
        .iter()
        .map(|path| manage::home_relative(&ctx, &cwd.join(path)))
        .collect::<Result<Vec<_>>>()?;
    let lock = adopt::lock(&ctx)?;
    let updated = update::update(&ctx, &only)?;
    if !updated.locks.is_empty() {
        after_commit(&ctx, &lock, started, json);
    }
    let steps = externals::apply(&ctx)?;
    let failed = !updated.failed.is_empty()
        || steps.iter().any(|step| {
            matches!(
                step.action,
                Action::Failed | Action::Changed | Action::Conflict
            )
        });
    if json {
        print_json(&Report {
            updated,
            externals: steps,
        });
    } else {
        let short = |lock: &str| lock.chars().take(7).collect::<String>();
        let shown = |path: &PathBuf| adopt::display(&ctx.home.join(path), &ctx.home);
        for lock in &updated.locks {
            match &lock.old {
                Some(old) => println!(
                    "Locked {} to {} (was {})",
                    shown(&lock.path),
                    short(&lock.new),
                    short(old)
                ),
                None => println!("Locked {} to {}", shown(&lock.path), short(&lock.new)),
            }
        }
        for (path, reason) in &updated.failed {
            eprintln!("error: {}: {reason}", shown(path));
        }
        if updated.locks.is_empty() && updated.failed.is_empty() && settled(&steps) {
            println!("Already up to date");
        }
        for step in &steps {
            print_external(&ctx, step);
        }
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
