//! Run a child process with a deadline, so a stuck shell cannot hang rosamta.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// `Ok(None)` means the deadline passed: the child was killed, or something
/// it started still held its output open.
pub(crate) fn output_with_timeout(
    cmd: &mut Command,
    timeout: Duration,
) -> std::io::Result<Option<Output>> {
    // A session of its own: one process group to kill, and no terminal. An
    // interactive shell in a background group of rsmt's terminal stops or
    // runs nothing.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Drain both pipes on threads so a chatty child cannot block on a full pipe.
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            // A background job of the child can hold the pipes past its exit.
            while !(out.is_finished() && err.is_finished()) {
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            return Ok(Some(Output {
                status,
                stdout: out.join().unwrap_or_default(),
                stderr: err.join().unwrap_or_default(),
            }));
        }
        if Instant::now() >= deadline {
            // Before the child is reaped, so its group id cannot be reused yet.
            unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) };
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grandchild_holding_the_pipes_cannot_outlast_the_deadline() {
        let start = Instant::now();
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 30; :"]);
        assert!(
            output_with_timeout(&mut cmd, Duration::from_millis(300))
                .unwrap()
                .is_none()
        );
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 30 & echo started"]);
        assert!(
            output_with_timeout(&mut cmd, Duration::from_millis(300))
                .unwrap()
                .is_none()
        );
        assert!(start.elapsed() < Duration::from_secs(5));

        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "echo out; echo err >&2"]);
        let output = output_with_timeout(&mut cmd, Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(output.stdout, b"out\n");
        assert_eq!(output.stderr, b"err\n");
    }
}
