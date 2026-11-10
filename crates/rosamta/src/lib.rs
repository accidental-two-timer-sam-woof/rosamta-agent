mod adopt;
mod commands;

use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use rosamta_core::config::Shell;
use rosamta_core::paths::Paths;
use rosamta_core::schema::Kind;
use rosamta_core::validate::{FileReport, Issue};
use rosamta_core::{Error, Result, repo, schema, validate};

#[derive(Parser)]
#[command(
    name = "rosamta",
    version,
    about = "Cross-shell, cross-host dotfile manager"
)]
struct Cli {
    /// Print one JSON document on stdout instead of text
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a repo, or clone an existing one, and register this host
    Init {
        /// Git URL of an existing repo to join
        url: Option<String>,
    },
    /// Check every file in the repo without changing any
    Validate,
    /// Print the JSON Schema of a file kind
    Schema {
        #[arg(value_enum)]
        kind: SchemaKind,
    },
    /// Render the modules for this host and switch shells to them
    Apply {
        /// Show what would change without writing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Add the rosamta block to a shell's startup files
    Hook {
        /// Shells to hook; all installed shells when omitted
        #[arg(value_parser = parse_shell)]
        shells: Vec<Shell>,
    },
    /// Remove the rosamta block from a shell's startup files
    Unhook {
        #[arg(value_parser = parse_shell)]
        shell: Shell,
    },
    /// Write the current settings into the startup files and remove every hook
    Eject {
        /// Do not ask for confirmation
        #[arg(long)]
        yes: bool,
    },
    /// Move everything outside the hook in startup files into modules
    Adopt {
        /// Do not ask for confirmation
        #[arg(long)]
        yes: bool,
    },
    /// List changes found in startup files, or accept or ignore one
    Review {
        #[command(subcommand)]
        action: Option<ReviewAction>,
    },
    /// Undo one adoption, or put startup files back the way they were before
    /// rosamta changed them
    Restore {
        /// The adoption to undo
        #[arg(required_unless_present = "original", conflicts_with = "original")]
        id: Option<u32>,
        /// Restore the content from before rosamta's first change
        #[arg(long)]
        original: bool,
        /// Do not ask for confirmation
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ReviewAction {
    /// Move a change into a module
    Accept {
        id: u32,
        /// Hosts the new module applies to
        #[arg(long, value_enum, default_value = "host")]
        scope: ScopeArg,
    },
    /// Keep a change in its startup file and stop listing it
    Ignore { id: u32 },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ScopeArg {
    /// Only this host
    Host,
    /// Every host
    All,
}

fn parse_shell(name: &str) -> std::result::Result<Shell, String> {
    name.parse()
}

#[derive(Clone, Copy, ValueEnum)]
enum SchemaKind {
    /// rosamta.toml
    Repo,
    /// A file under modules/
    Module,
    /// host.toml
    Host,
}

/// Entry point shared by the `rosamta` and `rsmt` binaries.
pub fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}: {err}", program_name());
            ExitCode::from(if err.is_usage() { 2 } else { 1 })
        }
    }
}

/// The name the user typed, so messages match `rsmt` or `rosamta`.
fn program_name() -> String {
    std::env::args_os()
        .next()
        .and_then(|arg0| {
            Path::new(&arg0)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "rosamta".into())
}

fn run(cli: &Cli) -> Result<ExitCode> {
    match &cli.command {
        Command::Init { url } => init(cli.json, url.as_deref()),
        Command::Validate => run_validate(cli.json),
        Command::Schema { kind } => {
            let kind = match kind {
                SchemaKind::Repo => Kind::Repo,
                SchemaKind::Module => Kind::Module,
                SchemaKind::Host => Kind::Host,
            };
            print_json(&schema::schema(kind));
            Ok(ExitCode::SUCCESS)
        }
        Command::Apply { dry_run } => commands::apply(cli.json, *dry_run),
        Command::Hook { shells } => commands::hook(cli.json, shells),
        Command::Unhook { shell } => commands::unhook(cli.json, *shell),
        Command::Eject { yes } => commands::eject(cli.json, *yes),
        Command::Adopt { yes } => adopt::adopt(cli.json, *yes),
        Command::Review { action: None } => adopt::review(cli.json),
        Command::Review {
            action: Some(ReviewAction::Accept { id, scope }),
        } => adopt::accept(cli.json, *id, *scope == ScopeArg::All),
        Command::Review {
            action: Some(ReviewAction::Ignore { id }),
        } => adopt::ignore(cli.json, *id),
        Command::Restore {
            id: Some(id), yes, ..
        } => adopt::restore(cli.json, *yes, *id),
        Command::Restore { yes, .. } => commands::restore(cli.json, *yes),
    }
}

fn init(json: bool, url: Option<&str>) -> Result<ExitCode> {
    let paths = Paths::from_env()?;
    let outcome = match url {
        None => repo::init_new(&paths)?,
        Some(url) => repo::join(&paths, url)?,
    };
    if json {
        print_json(&outcome);
    } else {
        let verb = if outcome.cloned { "Cloned" } else { "Created" };
        println!("{verb} repo at {}", outcome.repo.display());
        if let Some((from, to)) = outcome.migrated {
            println!("Migrated repo format {from} to {to}");
        }
        println!("Host ID: {}", outcome.host_id);
    }
    Ok(ExitCode::SUCCESS)
}

fn run_validate(json: bool) -> Result<ExitCode> {
    let repo_dir = Paths::from_env()?.repo();
    if !repo_dir.exists() {
        return Err(Error::Usage(format!(
            "no repo at {}; run `{} init` first",
            repo_dir.display(),
            program_name()
        )));
    }
    let report = validate::validate(&repo_dir)?;
    for file in &report.files {
        for issue in &file.warnings {
            eprintln!(
                "{}: warning: {}",
                location(&repo_dir, file, issue),
                issue.message
            );
        }
        if !json {
            for issue in &file.errors {
                eprintln!(
                    "{}: error: {}",
                    location(&repo_dir, file, issue),
                    issue.message
                );
            }
        }
    }
    if json {
        print_json(&serde_json::json!({ "valid": report.is_valid(), "files": report.files }));
    }
    Ok(if report.is_valid() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    })
}

fn location(repo_dir: &Path, file: &FileReport, issue: &Issue) -> String {
    let mut location = repo_dir.join(&file.path).display().to_string();
    if let Some(line) = issue.line {
        location.push_str(&format!(":{line}"));
        if let Some(column) = issue.column {
            location.push_str(&format!(":{column}"));
        }
    }
    location
}

pub(crate) fn print_json(value: &impl serde::Serialize) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("value serializes to JSON")
    );
}
