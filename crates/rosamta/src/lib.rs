mod adopt;
mod commands;
mod daemon;
mod github;
mod sync;

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
        /// Create a private GitHub repo with this name, or join <owner>/<name>
        #[arg(long, value_name = "NAME", conflicts_with = "url")]
        github: Option<String>,
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
    /// Commit changes in the repo and push them to the remote
    Publish,
    /// Bring in other hosts' changes and apply them
    Pull,
    /// Pull, then publish
    Sync,
    /// Run the background service, or start it at login
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Connect an account that hosts the repo
    Remote {
        #[command(subcommand)]
        remote: RemoteCommand,
    },
    /// Hand git the stored GitHub token (git credential helper)
    #[command(name = "git-credential", hide = true)]
    GitCredential { operation: String },
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
enum DaemonAction {
    /// Run the background service in this process
    Run,
    /// Start the background service now and at every login
    Enable,
    /// Stop the background service and no longer start it at login
    Disable,
    /// Show whether the background service runs and what it last did
    Status,
}

#[derive(Subcommand)]
enum RemoteCommand {
    /// GitHub
    Github {
        #[command(subcommand)]
        action: GithubAction,
    },
}

#[derive(Subcommand)]
enum GithubAction {
    /// Sign in with a code entered on github.com
    Login,
    /// Delete the stored token and the credential helper rosamta set up
    Logout,
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
        Command::Init {
            github: Some(target),
            ..
        } => github::init(cli.json, target),
        Command::Init { url, .. } => init(cli.json, url.as_deref()),
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
        Command::Publish => sync::publish(cli.json),
        Command::Pull => sync::pull(cli.json),
        Command::Sync => sync::sync(cli.json),
        Command::Remote {
            remote: RemoteCommand::Github { action },
        } => match action {
            GithubAction::Login => github::login(cli.json),
            GithubAction::Logout => github::logout(cli.json),
        },
        Command::GitCredential { operation } => github::credential(operation),
        Command::Daemon { action } => match action {
            DaemonAction::Run => daemon::run(),
            DaemonAction::Enable => daemon::enable(cli.json),
            DaemonAction::Disable => daemon::disable(cli.json),
            DaemonAction::Status => daemon::status(cli.json),
        },
    }
}

fn init(json: bool, url: Option<&str>) -> Result<ExitCode> {
    let started = rosamta_core::time::now();
    let paths = Paths::from_env()?;
    let outcome = match url {
        None => repo::init_new(&paths)?,
        Some(url) => repo::join(&paths, url)?,
    };
    report_init(json, &outcome);
    if outcome.cloned {
        // Joining committed this host to the repo's host list.
        let ctx = rosamta_core::apply::Context::from_env()?;
        let lock = rosamta_core::adopt::lock(&ctx)?;
        sync::after_commit(&ctx, &lock, started, json);
    }
    Ok(ExitCode::SUCCESS)
}

fn report_init(json: bool, outcome: &repo::InitOutcome) {
    if json {
        print_json(outcome);
    } else {
        let verb = if outcome.cloned { "Cloned" } else { "Created" };
        println!("{verb} repo at {}", outcome.repo.display());
        if let Some((from, to)) = outcome.migrated {
            println!("Migrated repo format {from} to {to}");
        }
        println!("Host ID: {}", outcome.host_id);
    }
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
