//! Importing a chezmoi source directory: its files, templates, scripts and
//! externals become rosamta declarations.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use serde_json::Value;

use crate::apply::Context;
use crate::error::{Error, Result};

mod attrs;
mod convert;
mod gotmpl;
mod plan;
pub mod report;
mod write;

pub use plan::Config;
pub use report::{Item, Outcome, Reason, Report};

/// How to run chezmoi: the program and global options, so tests can point
/// it at a sandbox.
#[derive(Debug, Clone)]
pub struct Chezmoi {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// chezmoi may ask for a passphrase to decrypt.
    pub terminal: bool,
}

impl Chezmoi {
    /// chezmoi on the user's PATH.
    pub fn find(ctx: &Context) -> Option<Self> {
        let args = if ctx.interactive {
            Vec::new()
        } else {
            vec!["--no-tty".to_string()]
        };
        Some(Self {
            program: crate::shells::which("chezmoi", &ctx.path_var)?,
            args,
            terminal: ctx.interactive,
        })
    }

    fn output(&self, args: &[&str]) -> Result<Vec<u8>> {
        self.run(args, None)
    }

    fn run(&self, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if input.is_some() {
            cmd.stdin(Stdio::piped());
        } else if !self.terminal {
            cmd.stdin(Stdio::null());
        }
        let cannot = |err: std::io::Error| Error::Usage(format!("cannot run chezmoi: {err}"));
        // Without a terminal nothing can answer chezmoi, so it gets a deadline.
        if input.is_none() && !self.terminal {
            let out = crate::process::output_with_timeout(&mut cmd, TIMEOUT)
                .map_err(cannot)?
                .ok_or_else(|| {
                    Error::Usage(format!("chezmoi {} did not finish", args.join(" ")))
                })?;
            return checked(args, out);
        }
        let mut child = cmd.spawn().map_err(cannot)?;
        let stdin = child.stdin.take();
        // Written alongside reading, so a large output cannot block it.
        let out = std::thread::scope(|scope| {
            if let (Some(mut stdin), Some(input)) = (stdin, input) {
                scope.spawn(move || stdin.write_all(input));
            }
            child.wait_with_output()
        })
        .map_err(cannot)?;
        checked(args, out)
    }

    fn json(&self, args: &[&str]) -> Result<Value> {
        serde_json::from_slice(&self.output(args)?)
            .map_err(|err| Error::Usage(format!("chezmoi {}: {err}", args.join(" "))))
    }

    pub fn source_path(&self) -> Result<PathBuf> {
        let out = self.output(&["source-path"])?;
        Ok(PathBuf::from(String::from_utf8_lossy(&out).trim()))
    }

    fn decrypt(&self, file: &Path) -> Result<Vec<u8>> {
        self.output(&["decrypt", &file.display().to_string()])
    }

    /// `text` rendered as chezmoi renders a template.
    fn render(&self, text: &[u8]) -> Result<Vec<u8>> {
        self.run(&["execute-template"], Some(text))
    }
}

/// How long chezmoi may take without a terminal.
const TIMEOUT: Duration = Duration::from_secs(60);

fn checked(args: &[&str], out: Output) -> Result<Vec<u8>> {
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(Error::Usage(format!(
            "chezmoi {} failed: {}",
            args.first().copied().unwrap_or_default(),
            stderr.trim()
        )));
    }
    Ok(out.stdout)
}

pub struct Options {
    /// The source directory; else chezmoi's, else `~/.local/share/chezmoi`.
    pub source: Option<PathBuf>,
    pub dry_run: bool,
    pub chezmoi: Option<Chezmoi>,
}

/// Where the source is, following `.chezmoiroot`.
pub fn source_dir(ctx: &Context, options: &Options) -> Result<PathBuf> {
    let dir = match (&options.source, &options.chezmoi) {
        (Some(dir), _) => dir.clone(),
        (None, Some(chezmoi)) => chezmoi.source_path()?,
        (None, None) => ctx.home.join(".local/share/chezmoi"),
    };
    if !dir.is_dir() {
        return Err(Error::Usage(format!(
            "{} is not a chezmoi source directory",
            dir.display()
        )));
    }
    let Ok(sub) = std::fs::read_to_string(dir.join(".chezmoiroot")) else {
        return Ok(dir);
    };
    // As chezmoi: a relative path inside the source.
    let sub = Path::new(sub.trim());
    let inside = sub
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)));
    let root = dir.join(sub);
    if !inside || !root.is_dir() || root.is_symlink() {
        return Err(Error::Usage(format!(
            "{}: .chezmoiroot must name a directory inside the source",
            dir.display()
        )));
    }
    Ok(root)
}

/// Whether importing `source` would need the repo's key: some files are
/// encrypted. The caller unlocks or creates it first, in a terminal.
pub fn needs_key(source: &Path) -> Result<bool> {
    let mut stack = vec![source.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(Error::read(&dir))? {
            let entry = entry.map_err(Error::read(&dir))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            // Symlinks are not followed, as the import does not follow them.
            if entry.file_type().map_err(Error::read(&dir))?.is_dir() {
                stack.push(entry.path());
            } else if name.contains("encrypted_") {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Fails when the repo has changes not published yet, which importing
/// would commit or undo; checked before asking for a passphrase.
pub fn writable(ctx: &Context) -> Result<()> {
    write::writable(&ctx.paths.repo())
}

/// Fails unless `rsmt init` has set up the repo and this host.
pub fn ready(ctx: &Context) -> Result<()> {
    if !ctx.paths.repo().join(".git").exists() || !ctx.paths.host_config().is_file() {
        return Err(Error::Usage(
            "this host is not set up yet; run `rsmt init` first".into(),
        ));
    }
    Ok(())
}

/// Import the source into the repo, or with `dry_run` only report what
/// would happen. The caller holds the adopt lock, and unlocks the repo key
/// first when the source has encrypted files.
pub fn import(ctx: &Context, options: &Options) -> Result<Report> {
    ready(ctx)?;
    let root = source_dir(ctx, options)?;
    // chezmoi reads data and renders from the same source.
    let chezmoi = options.chezmoi.clone().map(|mut chezmoi| {
        if let Some(dir) = &options.source {
            chezmoi
                .args
                .extend(["--source".to_string(), dir.display().to_string()]);
        }
        chezmoi
    });
    let config = match &chezmoi {
        Some(chezmoi) => Config::read(chezmoi)?,
        None => Config {
            umask: 0o022,
            ..Default::default()
        },
    };
    let host = crate::adopt::host_config(ctx)?;
    // A dry run shows what importing does once the key is unlocked.
    let can_encrypt = options.dry_run
        || (crate::dotfiles::crypt::repo_has_key(ctx) && !crate::dotfiles::crypt::locked(ctx)?);
    let plan = plan::plan(&plan::Inputs {
        ctx,
        host: &host,
        root: &root,
        config: &config,
        chezmoi: chezmoi.as_ref(),
        can_encrypt,
    })?;
    if options.dry_run {
        return Ok(plan.report);
    }
    write::write(ctx, &root, plan)
}
