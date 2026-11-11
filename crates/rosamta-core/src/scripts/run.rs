//! Running one script: its interpreter from the shebang, the home directory
//! as its working directory, and its output in a log only the user reads.

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

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
    let log = log_path(ctx, &super::apply::key(script));
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
    // A log left from before keeps its mode otherwise.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(Error::write(&log))?;
    let started = crate::time::date_time(crate::time::now());
    let _ = writeln!(file, "# {started}: {}", argv.join(" "));

    let (reader, writer) = std::io::pipe().map_err(Error::write(&log))?;
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .current_dir(&ctx.home)
        .env("HOME", &ctx.home)
        .env("ROSAMTA_REPO", &repo)
        .env("ROSAMTA_HOST_ID", host_id)
        .stdout(writer.try_clone().map_err(Error::write(&log))?)
        .stderr(writer);
    if terminal {
        cmd.stdin(Stdio::inherit());
    } else {
        // No controlling terminal: sudo and the like fail at once instead of
        // waiting on a prompt nobody sees. One process group to time out.
        cmd.stdin(Stdio::null());
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
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
    let file = Arc::new(Mutex::new(file));
    let closed = Arc::new(AtomicBool::new(false));
    let (done, finished) = mpsc::channel();
    let pump = Pump {
        file: Arc::clone(&file),
        closed: Arc::clone(&closed),
        terminal,
    };
    std::thread::spawn(move || {
        pump.drain(reader);
        let _ = done.send(());
    });
    let status = wait(&mut child, terminal).map_err(Error::write(&log))?;
    // A service the script started keeps the pipe open: its output is
    // read and dropped from here on, so neither side waits on the other.
    let _ = finished.recv_timeout(Duration::from_secs(2));
    closed.store(true, Ordering::SeqCst);
    let described = match (status, status.and_then(|status| status.code())) {
        (None, _) => format!("stopped after {} minutes", TIMEOUT.as_secs() / 60),
        (Some(_), Some(code)) => format!("exited with {code}"),
        (Some(status), None) => format!("ended by signal {}", status.signal().unwrap_or_default()),
    };
    if let Ok(mut file) = file.lock() {
        let _ = writeln!(file, "# {described}");
    }
    Ok(Outcome {
        ok: status.is_some_and(|status| status.success()),
        status: described,
    })
}

/// Outside a terminal nobody can answer or stop a script; one that runs
/// this long is stopped.
const TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// The script's exit, or `None` when it ran out of time and was stopped.
fn wait(child: &mut Child, terminal: bool) -> std::io::Result<Option<ExitStatus>> {
    if terminal {
        return child.wait().map(Some);
    }
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if started.elapsed() > TIMEOUT {
            // Its own session, so the group is the script and what it started.
            unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
            child.wait()?;
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Copies the script's output to the log, and to the terminal when there
/// is one, until the pipe closes or the run is over.
struct Pump {
    file: Arc<Mutex<File>>,
    closed: Arc<AtomicBool>,
    terminal: bool,
}

impl Pump {
    fn drain(&self, mut reader: std::io::PipeReader) {
        let mut written = 0;
        let mut buf = [0; 8192];
        let mut out = std::io::stdout();
        loop {
            let read = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(read) => read,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            if self.closed.load(Ordering::SeqCst) {
                continue;
            }
            if self.terminal {
                let _ = out.write_all(&buf[..read]);
                let _ = out.flush();
            }
            if written >= LOG_LIMIT {
                continue;
            }
            let keep = read.min((LOG_LIMIT - written) as usize);
            if let Ok(mut file) = self.file.lock() {
                let _ = file.write_all(&buf[..keep]);
                written += keep as u64;
                if keep < read {
                    let _ = writeln!(file, "\n# output past 1 MiB is not logged");
                }
            }
        }
    }
}
