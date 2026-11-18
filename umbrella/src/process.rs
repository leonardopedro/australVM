//! U06 (bounded start) — run a child under a deadline, in its own process
//! group, and reap everything this process owns.
//!
//! The prototype spawned the stock front-end with `Command::output()`: no
//! deadline, and no way to stop a child that hangs. `Command::output` also
//! reads the pipes to EOF, which is fine, but a timeout has to come from
//! somewhere — and once there is one, the child must die *with its
//! descendants*, because the compiler and the Cargo build both spawn helpers.
//!
//! What this module guarantees:
//!
//! * stdout and stderr are drained concurrently (two reader threads), so a
//!   noisy child cannot fill a pipe and deadlock;
//! * the child runs in its own process group, so a timeout kills the group
//!   (`kill(-pgid, SIGKILL)`), not just the leader;
//! * every exit is reported distinctly: an exit code and a terminating signal
//!   are different outcomes, and both carry the captured output;
//! * the child is always reaped before `run` returns, timeout or not.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How a child terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal(i32),
}

/// The child's captured result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub exit: Exit,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// The deadline fired and the child's process group was killed.
    pub timed_out: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildError {
    Spawn(String),
    Io(String),
}

impl std::fmt::Display for ChildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChildError::Spawn(e) => write!(f, "cannot spawn child: {e}"),
            ChildError::Io(e) => write!(f, "child I/O failed: {e}"),
        }
    }
}

fn drain(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let result = pipe.read_to_end(&mut buf).map(|_| buf);
        let _ = tx.send(result);
    });
    rx
}

/// Run `command` with a deadline, returning its exit and both streams.
///
/// The command is configured here, not by the caller: `process_group(0)` puts
/// it (and its descendants) in a fresh group, and both pipes are captured.
pub fn run(mut command: Command, limit: Duration) -> Result<Outcome, ChildError> {
    use std::os::unix::process::CommandExt;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command
        .spawn()
        .map_err(|e| ChildError::Spawn(e.to_string()))?;
    let pid = child.id() as i32;
    let out_rx = drain(
        child
            .stdout
            .take()
            .ok_or_else(|| ChildError::Io("no stdout pipe".into()))?,
    );
    let err_rx = drain(
        child
            .stderr
            .take()
            .ok_or_else(|| ChildError::Io("no stderr pipe".into()))?,
    );

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                // Reap rather than leak, even when the wait itself failed.
                let _ = kill_group(pid);
                let _ = child.wait();
                return Err(ChildError::Io(e.to_string()));
            }
        }
        if started.elapsed() >= limit {
            timed_out = true;
            kill_group(pid)?;
            break child.wait().map_err(|e| ChildError::Io(e.to_string()))?;
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    let stdout = out_rx
        .recv()
        .map_err(|_| ChildError::Io("stdout reader vanished".into()))?
        .map_err(|e| ChildError::Io(e.to_string()))?;
    let stderr = err_rx
        .recv()
        .map_err(|_| ChildError::Io("stderr reader vanished".into()))?
        .map_err(|e| ChildError::Io(e.to_string()))?;

    use std::os::unix::process::ExitStatusExt;
    let exit = match status.code() {
        Some(code) => Exit::Code(code),
        None => Exit::Signal(status.signal().unwrap_or(0)),
    };
    Ok(Outcome {
        exit,
        stdout,
        stderr,
        timed_out,
    })
}

fn kill_group(pid: i32) -> Result<(), ChildError> {
    // Negative pid = the whole process group the child leads. `kill` returns
    // ESRCH when the group is already gone, which is not an error here.
    let rc = unsafe { libc::kill(-pid, libc::SIGKILL) };
    if rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(ChildError::Io(format!(
            "cannot kill process group {pid}: {}",
            std::io::Error::last_os_error()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Write straight to a file descriptor: libtest's output capture sits in
    /// `print!`, so a helper that used it would be buffered until its test
    /// ended — and these helpers are killed before they end.
    fn write_fd(fd: i32, bytes: &[u8]) {
        let mut written = 0;
        while written < bytes.len() {
            let rc = unsafe {
                libc::write(
                    fd,
                    bytes[written..].as_ptr() as *const libc::c_void,
                    bytes.len() - written,
                )
            };
            if rc <= 0 {
                return;
            }
            written += rc as usize;
        }
    }

    /// Re-exec this test binary as a helper. `AVM_CHILD_MODE` selects behavior;
    /// without it the helper runs one selected test and exits.
    #[test]
    fn child_helper() {
        let Ok(mode) = std::env::var("AVM_CHILD_MODE") else {
            return;
        };
        match mode.as_str() {
            "noisy" => {
                let chunk = "x".repeat(4096);
                for _ in 0..256 {
                    write_fd(1, chunk.as_bytes());
                    write_fd(2, chunk.as_bytes());
                }
            }
            "hang" => std::thread::sleep(Duration::from_secs(60)),
            "grandchild" => {
                let gc = Command::new("sleep")
                    .arg("60")
                    .spawn()
                    .expect("spawn grandchild");
                write_fd(1, format!("grandchild={}\n", gc.id()).as_bytes());
                std::thread::sleep(Duration::from_secs(60));
            }
            "exit3" => std::process::exit(3),
            "abort" => std::process::abort(),
            other => panic!("unknown helper mode {other}"),
        }
    }

    fn helper(mode: &str) -> Command {
        let mut cmd = Command::new(std::env::current_exe().expect("current exe"));
        cmd.arg("--exact")
            .arg("process::tests::child_helper")
            .arg("--nocapture")
            .env("AVM_CHILD_MODE", mode)
            // Keep the helper's own harness quiet.
            .env("RUST_TEST_THREADS", "1");
        cmd
    }

    #[test]
    fn a_noisy_child_cannot_deadlock_the_pipes() {
        let outcome = run(helper("noisy"), Duration::from_secs(30)).expect("run");
        assert_eq!(outcome.exit, Exit::Code(0));
        assert!(!outcome.timed_out);
        // The helper's own test harness writes its banner first; everything it
        // wrote itself is on top of that, and nothing may be truncated.
        assert!(outcome.stdout.len() >= 4096 * 256, "stdout truncated");
        assert!(outcome.stderr.len() >= 4096 * 256, "stderr truncated");
    }

    #[test]
    fn a_hanging_child_is_killed_at_the_deadline_and_reaped() {
        let started = Instant::now();
        let outcome = run(helper("hang"), Duration::from_millis(300)).expect("run");
        let elapsed = started.elapsed();
        assert!(outcome.timed_out, "the deadline must fire");
        assert_eq!(outcome.exit, Exit::Signal(libc::SIGKILL));
        assert!(
            elapsed < Duration::from_secs(10),
            "the deadline is parent-controlled, took {elapsed:?}"
        );
    }

    #[test]
    fn the_whole_process_group_dies_with_the_leader() {
        let outcome = run(helper("grandchild"), Duration::from_millis(500)).expect("run");
        assert!(outcome.timed_out);
        let text = String::from_utf8_lossy(&outcome.stdout);
        // The helper's harness writes the test name without a newline before
        // the test runs, so the marker is not at a line start.
        let pid: i32 = text
            .split("grandchild=")
            .nth(1)
            .unwrap_or_else(|| panic!("helper must print its grandchild pid; stdout: {text:?}"))
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap_or("")
            .parse()
            .unwrap_or_else(|_| panic!("unreadable grandchild pid; stdout: {text:?}"));
        // The grandchild was killed with the group, so its /proc entry is gone
        // (or at worst a zombie owned by init, not a live sleeper).
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let state = stat
            .rsplit(')')
            .next()
            .and_then(|s| s.trim().chars().next());
        assert!(
            state.is_none() || state == Some('Z'),
            "grandchild {pid} is still running: {stat}"
        );
        let _ = std::fs::read_to_string(format!("/proc/{pid}/stat"));
    }

    #[test]
    fn exit_codes_and_signals_are_distinct_outcomes() {
        let code = run(helper("exit3"), Duration::from_secs(30)).expect("run");
        assert_eq!(code.exit, Exit::Code(3));
        assert!(!code.timed_out);
        let signalled = run(helper("abort"), Duration::from_secs(30)).expect("run");
        assert_eq!(signalled.exit, Exit::Signal(libc::SIGABRT));
        assert_ne!(code.exit, signalled.exit);
    }

    #[test]
    fn a_missing_program_is_a_spawn_error() {
        let mut cmd = Command::new("/nonexistent/definitely-not-a-program");
        cmd.arg("x");
        let err = run(cmd, Duration::from_secs(1)).unwrap_err();
        assert!(matches!(err, ChildError::Spawn(_)), "{err}");
    }
}
