//! Running one script: its interpreter from the shebang, the home directory
//! as its working directory, and its output in a log only the user reads.

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::Script;
use crate::apply::Context;
use crate::error::{Error, Result};

/// Enough for an installer's whole output, not for one stuck in a loop.
const LOG_LIMIT: u64 = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub ok: bool,
    /// What it exited with, or how it ended otherwise.
    pub status: String,
}

/// Where the last run of `source` is logged.
pub fn log_path(ctx: &Context, source: &str) -> PathBuf {
    ctx.paths
        .state()
        .join("scripts")
        .join(format!("{source}.log"))
}

/// The program to run `script` with: the shebang's interpreter and its one
/// argument, as the kernel reads them, else `sh`.
fn interpreter(script: &Path) -> Result<Vec<String>> {
    let mut head = Vec::new();
    File::open(script)
        .and_then(|file| file.take(512).read_to_end(&mut head))
        .map_err(Error::read(script))?;
    let line = head.split(|byte| *byte == b'\n').next().unwrap_or_default();
    let Some(rest) = line.strip_prefix(b"#!") else {
        return Ok(vec!["sh".into()]);
    };
    let rest = String::from_utf8_lossy(rest);
    let rest = rest.trim();
    let (program, arg) = rest.split_once([' ', '\t']).unwrap_or((rest, ""));
    if program.is_empty() {
        return Ok(vec!["sh".into()]);
    }
    let mut out = vec![program.to_string()];
    if !arg.trim().is_empty() {
        out.push(arg.trim().to_string());
    }
    Ok(out)
}

/// Run `script`. In a terminal its input is the terminal's and its output
/// shows there too; otherwise its input is empty. Either way the output,
/// stdout and stderr in the order written, replaces the last log.
pub fn run(ctx: &Context, host_id: &str, script: &Script, terminal: bool) -> Result<Outcome> {
    let repo = ctx.paths.repo();
    let source = repo.join(&script.source);
    let mut argv = interpreter(&source)?;
    argv.push(source.display().to_string());
    let log = log_path(ctx, &script.source);
    let dir = log.parent().unwrap_or(&log);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(Error::write(dir))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&log)
        .map_err(Error::write(&log))?;
    let started = crate::time::date_time(crate::time::now());
    let _ = writeln!(file, "# {started}: {}", argv.join(" "));

    let (mut reader, writer) = std::io::pipe().map_err(Error::write(&log))?;
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .current_dir(&ctx.home)
        .env("HOME", &ctx.home)
        .env("ROSAMTA_REPO", &repo)
        .env("ROSAMTA_HOST_ID", host_id)
        .stdin(if terminal {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .stdout(writer.try_clone().map_err(Error::write(&log))?)
        .stderr(writer);
    let spawned = cmd.spawn();
    // The write ends close with the command, so reading ends with the script.
    drop(cmd);
    let mut child = match spawned {
        Ok(child) => child,
        Err(err) => {
            let status = format!("cannot start {}: {err}", argv[0]);
            let _ = writeln!(file, "# {status}");
            return Ok(Outcome { ok: false, status });
        }
    };
    let mut written = 0;
    let mut buf = [0; 8192];
    let mut out = std::io::stdout();
    loop {
        let read = match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        if terminal {
            let _ = out.write_all(&buf[..read]);
            let _ = out.flush();
        }
        if written < LOG_LIMIT {
            let keep = read.min((LOG_LIMIT - written) as usize);
            let _ = file.write_all(&buf[..keep]);
            written += keep as u64;
            if written == LOG_LIMIT {
                let _ = writeln!(file, "\n# output past 1 MiB is not logged");
            }
        }
    }
    let status = child.wait().map_err(Error::write(&log))?;
    let described = match status.code() {
        Some(code) => format!("exited with {code}"),
        None => "ended by a signal".to_string(),
    };
    let _ = writeln!(file, "# {described}");
    Ok(Outcome {
        ok: status.success(),
        status: described,
    })
}
