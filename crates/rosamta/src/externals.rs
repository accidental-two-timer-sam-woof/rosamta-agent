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
    let shown = |path: &PathBuf| adopt::display(&ctx.home.join(path), &ctx.home);
    // Printed before downloading, which can take a while or fail.
    if !json {
        let short = |lock: &str| lock.chars().take(7).collect::<String>();
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
        for failure in &updated.failed {
            eprintln!("error: {}: {}", shown(&failure.path), failure.reason);
        }
    }
    let steps = externals::apply(&ctx)?;
    let failed =
        !updated.failed.is_empty() || steps.iter().any(|step| step.action == Action::Failed);
    // New locks reach other hosts only once they downloaded here.
    if !updated.locks.is_empty() && !failed {
        after_commit(&ctx, &lock, started, json);
    }
    if json {
        print_json(&Report {
            updated,
            externals: steps,
        });
    } else {
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
