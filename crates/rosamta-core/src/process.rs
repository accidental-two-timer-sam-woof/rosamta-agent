//! Run a child process with a deadline, so a stuck shell cannot hang rosamta.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long output may keep arriving after the child exits: what it wrote
/// is in the pipe by then, and a background job may hold the pipe for good.
const DRAIN: Duration = Duration::from_millis(200);

/// `Ok(None)` means the child did not exit before the deadline and was
/// killed. Output something it started writes after it exits may be cut.
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
    let (out, out_done) = drain(child.stdout.take().expect("piped"));
    let (err, err_done) = drain(child.stderr.take().expect("piped"));
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            // A background job of the child can hold the pipes past its exit.
            let until = deadline.min(Instant::now() + DRAIN);
            while !(out_done.is_finished() && err_done.is_finished()) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            let take = |buf: &Arc<Mutex<Vec<u8>>>| std::mem::take(&mut *buf.lock().unwrap());
            return Ok(Some(Output {
                status,
                stdout: take(&out),
                stderr: take(&err),
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

/// Read `pipe` to its end on a thread, into a buffer readable at any time.
fn drain(
    mut pipe: impl Read + Send + 'static,
) -> (Arc<Mutex<Vec<u8>>>, std::thread::JoinHandle<()>) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buf);
    let done = std::thread::spawn(move || {
        let mut chunk = [0; 8192];
        while let Ok(read @ 1..) = pipe.read(&mut chunk) {
            sink.lock().unwrap().extend_from_slice(&chunk[..read]);
        }
    });
    (buf, done)
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
        // The child is done; its background job does not hold up the result.
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 30 & echo started"]);
        let output = output_with_timeout(&mut cmd, Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert_eq!(output.stdout, b"started\n");
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
