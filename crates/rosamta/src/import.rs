//! `rsmt import chezmoi`.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rosamta_core::apply::Context;
use rosamta_core::chezmoi::{self, Chezmoi, Item, Options, Outcome, Reason, Report};
use rosamta_core::{Error, Result, adopt, time};

use crate::dotfiles::ensure_key;
use crate::print_json;
use crate::sync::after_commit;

pub fn chezmoi(json: bool, source: Option<&Path>, dry_run: bool) -> Result<ExitCode> {
    let started = time::now();
    let mut ctx = Context::from_env()?;
    // chezmoi may ask for the passphrase of its own key.
    ctx.interactive = !json && std::io::stdin().is_terminal();
    chezmoi::ready(&ctx)?;
    let source = match source {
        Some(path) => Some(current_dir()?.join(path)),
        None => None,
    };
    let options = Options {
        source,
        dry_run,
        chezmoi: Chezmoi::find(&ctx),
    };
    if dry_run {
        let report = chezmoi::import(&ctx, &options)?;
        print(&ctx, json, &report);
        return Ok(ExitCode::SUCCESS);
    }
    let lock = adopt::lock(&ctx)?;
    chezmoi::writable(&ctx)?;
    // Without a terminal, encrypted files are left to convert by hand.
    if ctx.interactive && chezmoi::needs_key(&chezmoi::source_dir(&ctx, &options)?)? {
        ensure_key(&ctx, json)?;
    }
    let report = chezmoi::import(&ctx, &options)?;
    print(&ctx, json, &report);
    after_commit(&ctx, &lock, started, json);
    Ok(ExitCode::SUCCESS)
}

fn current_dir() -> Result<PathBuf> {
    std::env::current_dir().map_err(|err| Error::Read {
        path: ".".into(),
        source: err,
    })
}

fn print(ctx: &Context, json: bool, report: &Report) {
    if json {
        print_json(report);
        return;
    }
    for item in &report.items {
        println!("{}", line(ctx, item));
    }
    println!(
        "Imported {}, skipped {}, not imported {}",
        report.count(Outcome::Imported),
        report.count(Outcome::Skipped),
        report.count(Outcome::Manual)
    );
}

fn line(ctx: &Context, item: &Item) -> String {
    let target = item.target.as_deref().unwrap_or_default();
    let home = || adopt::display(&ctx.home.join(target), &ctx.home);
    let mut text = match (item.outcome, item.reason) {
        (Outcome::Imported, Reason::Script) => format!("Imported script {target}"),
        (Outcome::Imported, Reason::External) => format!("Imported external {}", home()),
        (Outcome::Imported, _) => format!("Imported {}", home()),
        (Outcome::Skipped, reason) => format!("Skipped {}: {}", item.source, why(reason)),
        (Outcome::Manual, reason) => format!("Not imported {}: {}", item.source, why(reason)),
        (Outcome::Note, reason) => format!("Note: {}: {}", item.source, why(reason)),
    };
    if let Some(detail) = &item.detail {
        text.push_str(&format!(" ({detail})"));
    }
    text
}

fn why(reason: Reason) -> &'static str {
    match reason {
        Reason::Dotfile | Reason::Script | Reason::External => "imported",
        Reason::Ignored => "excluded by .chezmoiignore",
        Reason::Empty => "empty",
        Reason::Exists => "already in the repo",
        Reason::StartupFile => "a shell startup file; rosamta keeps its content in modules",
        Reason::Template => "unsupported template",
        Reason::Secret => "looks like a secret and is not encrypted",
        Reason::Encrypted => "encrypted, and cannot be decrypted or encrypted here",
        Reason::Scope => "its platforms do not fit one `when`",
        Reason::Differs => "renders differently from chezmoi",
        Reason::Modify => "modify_ file",
        Reason::Remove => "remove_ entry",
        Reason::Symlink => "symlink_ entry",
        Reason::CreateTemplate => "create_ template",
        Reason::ExactDir => "exact_ directory",
        Reason::DirPermissions => "directory permissions",
        Reason::EmptyDir => "empty directory",
        Reason::ExternalType => "unsupported external type",
        Reason::ExternalField => "unsupported external fields",
        Reason::ExternalTemplate => "externals file is a template",
        Reason::Nested => "inside another dotfile or external",
        Reason::Format => "unsupported file format",
        Reason::Special => "chezmoi-only file",
        Reason::Link => "a symlink in the source directory",
        Reason::GitIgnored => "ignored by git",
        Reason::Hostname => "hostnames became host IDs",
        Reason::EveryApply => "ran on every chezmoi apply; now runs when it changes",
        Reason::ScriptDir => "now runs in the home directory",
        Reason::RefreshPeriod => "stays at its locked version until `rsmt externals update`",
        Reason::ArchiveConflict => {
            "chezmoi's copy is in place, so the first apply lists a conflict"
        }
        Reason::Unlocked => "not locked until `rsmt externals update`",
        Reason::MayBeEmpty => "empty on some hosts, where rosamta still writes the file",
    }
}
