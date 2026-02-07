//! Git command execution infrastructure.
//!
//! Provides low-level functions for spawning git processes with timeout support.

use std::cell::Cell;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

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
) -> anyhow::Result<std::process::Output> {
    logger(config, args, None);

    let start = Instant::now();
    let debug = config.is_debug();
    // Print first command timing with --debug, all commands with GIT_DAILY_DEBUG=1
    let trace_all = std::env::var("GIT_DAILY_DEBUG").is_ok();
    let is_first = debug
        && FIRST_GIT_PRINTED.with(|flag| {
            if flag.get() {
                false
            } else {
                flag.set(true);
                true
            }
        });

    let mut child = Command::new("git")
        .current_dir(repo)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn git command")?;

    let result = wait_with_timeout(&mut child, constants::git_timeout());

    // Print timing for first command (--debug) or all commands (GIT_DAILY_DEBUG=1)
    if is_first || trace_all {
        let elapsed = start.elapsed();
        let repo_name = repo.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
        eprintln!(
            "[debug] git {} in {} took {:?}",
            args.join(" "),
            repo_name,
            elapsed
        );
    }

    match result {
        Ok(output) => Ok(output),
        Err(e) => {
            // Kill the process if it's still running after timeout
            let _ = child.kill();
            Err(e)
        }
    }
}

/// Waits for a child process with a timeout.
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
) -> anyhow::Result<std::process::Output> {
    wait_with_timeout_inner(child, timeout)
}

fn wait_with_timeout_inner<C>(
    child: &mut C,
    timeout: std::time::Duration,
) -> anyhow::Result<std::process::Output>
where
    C: WaitableChild,
{
    use std::time::Instant;

    let start = Instant::now();
    let poll_interval = std::time::Duration::from_millis(100);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Process has exited, collect output
                let stdout = child.read_stdout()?;
                let stderr = child.read_stderr()?;

                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                // Process still running
                if start.elapsed() > timeout {
                    anyhow::bail!("git command timed out after {} seconds", timeout.as_secs());
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => return Err(e).context("Failed to wait for git process"),
        }
    }
}

trait WaitableChild {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
    fn read_stdout(&mut self) -> anyhow::Result<Vec<u8>>;
    fn read_stderr(&mut self) -> anyhow::Result<Vec<u8>>;
}

impl WaitableChild for std::process::Child {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        std::process::Child::try_wait(self)
    }

    fn read_stdout(&mut self) -> anyhow::Result<Vec<u8>> {
        let mut buf = Vec::new();
        if let Some(mut stdout) = self.stdout.take() {
            std::io::Read::read_to_end(&mut stdout, &mut buf)
                .context("Failed to read stdout from git process")?;
        }
        Ok(buf)
    }

    fn read_stderr(&mut self) -> anyhow::Result<Vec<u8>> {
        let mut buf = Vec::new();
        if let Some(mut stderr) = self.stderr.take() {
            std::io::Read::read_to_end(&mut stderr, &mut buf)
                .context("Failed to read stderr from git process")?;
        }
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;

    struct FakeChild {
        try_wait: Option<io::Result<Option<std::process::ExitStatus>>>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    }

    impl WaitableChild for FakeChild {
        fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
            self.try_wait.take().unwrap_or(Ok(None))
        }

        fn read_stdout(&mut self) -> anyhow::Result<Vec<u8>> {
            Ok(std::mem::take(&mut self.stdout))
        }

        fn read_stderr(&mut self) -> anyhow::Result<Vec<u8>> {
            Ok(std::mem::take(&mut self.stderr))
        }
    }

    #[test]
    fn test_wait_with_timeout_times_out() {
        let mut child = FakeChild {
            try_wait: Some(Ok(None)),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        let result = wait_with_timeout_inner(&mut child, std::time::Duration::from_millis(0));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[test]
    fn test_wait_with_timeout_propagates_try_wait_error() {
        let mut child = FakeChild {
            try_wait: Some(Err(io::Error::other("boom"))),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        let result = wait_with_timeout_inner(&mut child, std::time::Duration::from_secs(1));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to wait"));
    }

    #[test]
    fn test_wait_with_timeout_reads_output() {
        let status = {
            #[cfg(unix)]
            {
                std::process::ExitStatus::from_raw(0)
            }
            #[cfg(not(unix))]
            {
                std::process::ExitStatus::default()
            }
        };

        let mut child = FakeChild {
            try_wait: Some(Ok(Some(status))),
            stdout: b"ok\n".to_vec(),
            stderr: b"warn\n".to_vec(),
        };
        let output = wait_with_timeout_inner(&mut child, std::time::Duration::from_secs(1))
            .expect("expected output");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"ok\n");
        assert_eq!(output.stderr, b"warn\n");
    }
}
