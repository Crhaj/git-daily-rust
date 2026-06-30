//! Git command execution infrastructure.
//!
//! Provides low-level functions for spawning git processes with timeout support.
//!
//! # Deadlock safety
//!
//! Child stdout/stderr are drained on dedicated threads while the parent waits
//! for the process to exit. Reading the pipes only *after* the process exited
//! would deadlock whenever a git command emits more output than the OS pipe
//! buffer (~64 KB): git blocks writing, the parent never reads, and the command
//! only ever "completes" when the timeout fires.
//!
//! The reads run on freshly spawned threads rather than a bounded pool on
//! purpose: the two pipes are mutually-dependent blocking I/O (neither finishes
//! until both drain), so they must be guaranteed to run concurrently. A bounded
//! pool (rayon included) could leave one read queued behind the other and
//! reintroduce the deadlock.
//!
//! # Non-interactive execution
//!
//! Git is spawned with terminal prompts disabled (`GIT_TERMINAL_PROMPT=0`),
//! stdin redirected from `/dev/null`, and SSH forced into batch mode. Without
//! this, a repo whose remote needs credentials makes git block on a terminal
//! prompt - hanging until the timeout instead of failing fast with a clear
//! authentication error.

use std::cell::Cell;
use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context;

use crate::config::Config;
use crate::constants;

use super::GitLogger;

thread_local! {
    /// Tracks whether we've printed the first git command timing in this thread.
    ///
    /// # Why thread-local?
    ///
    /// Using thread-local storage instead of a global static avoids test pollution
    /// when tests run in parallel - each test thread gets its own independent flag.
    ///
    /// # Why Cell is safe here
    ///
    /// `Cell<bool>` provides interior mutability without synchronization overhead.
    /// This is safe because:
    /// - `thread_local!` guarantees single-threaded access (no data races possible)
    /// - `bool` is `Copy`, so `Cell::get()`/`Cell::set()` never panic
    /// - No references to the inner value escape the closure passed to `.with()`
    static FIRST_GIT_PRINTED: Cell<bool> = const { Cell::new(false) };
}

/// Executes a git command in the specified repository directory with timeout.
pub fn run_git(repo: &Path, config: &Config, args: &[&str]) -> anyhow::Result<String> {
    run_git_with_logger(repo, config, args, super::no_op_logger)
}

/// Executes a git command with a custom logging callback.
/// The logger is called once before execution (output=None) and once after (output=Some).
pub fn run_git_with_logger(
    repo: &Path,
    config: &Config,
    args: &[&str],
    logger: GitLogger,
) -> anyhow::Result<String> {
    let output = run_git_output(repo, config, args, logger)?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        logger(config, args, Some(&stdout));
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {} failed: {}", args.join(" "), stderr)
    }
}

/// Executes a git command and returns the raw output without interpreting exit status.
pub(super) fn run_git_output(
    repo: &Path,
    config: &Config,
    args: &[&str],
    logger: GitLogger,
) -> anyhow::Result<Output> {
    logger(config, args, None);
    let start = Instant::now();

    let mut child = build_git_command(repo, args)
        .spawn()
        .context("Failed to spawn git command")?;

    // Drain stdout/stderr on their own threads so a flood of output can't block
    // the child (and deadlock us) while we wait for it to exit.
    let stdout_reader = spawn_reader(child.stdout.take());
    let stderr_reader = spawn_reader(child.stderr.take());

    let status = wait_with_timeout(&mut child, constants::git_timeout());

    // Join the readers regardless of outcome. On timeout the child has already
    // been killed and reaped, so its pipes are closed and the readers finish.
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();

    trace_command_timing(config, repo, args, start.elapsed());

    // Attach the command so a timeout or wait failure names what hung.
    let status = status.with_context(|| format!("git {}", args.join(" ")))?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Builds a git command with non-interactive settings applied.
///
/// Prompts are disabled and stdin is closed so a repo needing credentials fails
/// fast instead of hanging on a terminal prompt. An existing `GIT_SSH_COMMAND`
/// is respected; otherwise SSH is forced into batch mode for the same reason.
fn build_git_command(repo: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo)
        .args(args)
        .env(
            constants::GIT_TERMINAL_PROMPT_VAR,
            constants::GIT_PROMPT_DISABLED,
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if std::env::var_os(constants::GIT_SSH_COMMAND_VAR).is_none() {
        cmd.env(
            constants::GIT_SSH_COMMAND_VAR,
            constants::GIT_SSH_BATCH_COMMAND,
        );
    }

    cmd
}

/// Spawns a thread that drains a child pipe to EOF, returning the bytes read.
///
/// Uses a small stack since the read loop is trivial. Capturing output is
/// best-effort: an absent handle or a read error yields an empty buffer, since
/// callers act on the exit status, not partial output. Panics if the OS cannot
/// create the thread, matching [`std::thread::spawn`].
fn spawn_reader<R>(handle: Option<R>) -> std::thread::JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    std::thread::Builder::new()
        .stack_size(constants::GIT_READER_STACK_SIZE)
        .spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut reader) = handle {
                let _ = reader.read_to_end(&mut buf);
            }
            buf
        })
        .expect("failed to spawn git output reader thread")
}

/// Emits per-command timing to stderr when tracing is enabled.
///
/// `--debug` prints the first git command in each thread; `GIT_DAILY_DEBUG=1`
/// prints every command.
fn trace_command_timing(config: &Config, repo: &Path, args: &[&str], elapsed: Duration) {
    let trace_all = std::env::var("GIT_DAILY_DEBUG").is_ok();
    let is_first = config.is_debug()
        && FIRST_GIT_PRINTED.with(|flag| {
            if flag.get() {
                false
            } else {
                flag.set(true);
                true
            }
        });

    if is_first || trace_all {
        let repo_name = repo.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
        eprintln!(
            "[debug] git {} in {} took {:?}",
            args.join(" "),
            repo_name,
            elapsed
        );
    }
}

/// Waits for a child process to exit, enforcing a timeout.
///
/// On timeout the child is killed and reaped so it does not linger as a zombie,
/// then an error is returned. Output is drained separately by reader threads, so
/// this only concerns the exit status. Generic over [`ChildProcess`] so the
/// timeout/kill logic can be unit-tested without spawning real processes.
fn wait_with_timeout<C>(child: &mut C, timeout: Duration) -> anyhow::Result<ExitStatus>
where
    C: ChildProcess,
{
    let start = Instant::now();
    let poll_interval = Duration::from_millis(constants::GIT_WAIT_POLL_MS);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if start.elapsed() > timeout {
                    // Terminate and reap so we don't leave a zombie git process.
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("timed out after {} seconds", timeout.as_secs());
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => return Err(e).context("Failed to wait for git process"),
        }
    }
}

/// Minimal child-process control surface.
///
/// Abstracted so the timeout/kill logic can be unit-tested without spawning real
/// processes.
trait ChildProcess {
    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>>;
    fn kill(&mut self) -> std::io::Result<()>;
    fn wait(&mut self) -> std::io::Result<ExitStatus>;
}

impl ChildProcess for std::process::Child {
    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        std::process::Child::try_wait(self)
    }

    fn kill(&mut self) -> std::io::Result<()> {
        std::process::Child::kill(self)
    }

    fn wait(&mut self) -> std::io::Result<ExitStatus> {
        std::process::Child::wait(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::io;
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;

    /// Builds a zero (success) exit status for the current platform.
    fn zero_status() -> ExitStatus {
        #[cfg(unix)]
        {
            ExitStatus::from_raw(0)
        }
        #[cfg(not(unix))]
        {
            ExitStatus::default()
        }
    }

    struct FakeChild {
        try_wait: Option<io::Result<Option<ExitStatus>>>,
        killed: bool,
        reaped: bool,
    }

    impl FakeChild {
        fn new(first: io::Result<Option<ExitStatus>>) -> Self {
            Self {
                try_wait: Some(first),
                killed: false,
                reaped: false,
            }
        }
    }

    impl ChildProcess for FakeChild {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            // After the seeded result is consumed, report "still running".
            self.try_wait.take().unwrap_or(Ok(None))
        }

        fn kill(&mut self) -> io::Result<()> {
            self.killed = true;
            Ok(())
        }

        fn wait(&mut self) -> io::Result<ExitStatus> {
            self.reaped = true;
            Ok(zero_status())
        }
    }

    #[test]
    fn test_wait_with_timeout_kills_and_reaps_on_timeout() {
        let mut child = FakeChild::new(Ok(None));
        let result = wait_with_timeout(&mut child, Duration::from_millis(0));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
        assert!(child.killed, "child should be killed on timeout");
        assert!(child.reaped, "child should be reaped on timeout");
    }

    #[test]
    fn test_wait_with_timeout_propagates_try_wait_error() {
        let mut child = FakeChild::new(Err(io::Error::other("boom")));
        let result = wait_with_timeout(&mut child, Duration::from_secs(1));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to wait"));
        assert!(!child.killed, "should not kill on a try_wait error");
    }

    #[test]
    fn test_wait_with_timeout_returns_status_on_exit() {
        let mut child = FakeChild::new(Ok(Some(zero_status())));
        let status =
            wait_with_timeout(&mut child, Duration::from_secs(1)).expect("expected status");
        assert!(status.success());
        assert!(!child.killed, "a cleanly exited child should not be killed");
    }

    #[test]
    fn test_build_git_command_disables_terminal_prompts() {
        let cmd = build_git_command(Path::new("/tmp"), &["status"]);
        let envs: std::collections::HashMap<_, _> = cmd.get_envs().collect();
        assert_eq!(
            envs.get(OsStr::new(constants::GIT_TERMINAL_PROMPT_VAR)),
            Some(&Some(OsStr::new(constants::GIT_PROMPT_DISABLED))),
            "git must be spawned with terminal prompts disabled"
        );
    }
}
