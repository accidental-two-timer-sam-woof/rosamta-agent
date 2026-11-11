//! Importing a chezmoi source directory: its files, templates, scripts and
//! externals become rosamta declarations.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
        Some(Self {
            program: crate::shells::which("chezmoi", &ctx.path_var)?,
            args: Vec::new(),
            terminal: ctx.interactive,
        })
    }

    fn output(&self, args: &[&str]) -> Result<Vec<u8>> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args).args(args).stderr(Stdio::piped());
        if !self.terminal {
            cmd.stdin(Stdio::null());
        }
        let out = cmd
            .output()
            .map_err(|err| Error::Usage(format!("cannot run chezmoi: {err}")))?;
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

    fn cat(&self, target: &Path) -> Result<Vec<u8>> {
        self.output(&["cat", &target.display().to_string()])
    }
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
    let root = dir.join(".chezmoiroot");
    Ok(match std::fs::read_to_string(&root) {
        Ok(sub) => dir.join(sub.trim()),
        Err(_) => dir,
    })
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
            if entry.path().is_dir() {
                stack.push(entry.path());
            } else if name.contains("encrypted_") {
                return Ok(true);
            }
        }
    }
    Ok(false)
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
