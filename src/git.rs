//! Git command wrappers.
//!
//! Thin wrappers around git CLI commands with error formatting and timeout support.
//! Uses callback-based logging to avoid coupling with presentation layer.

use crate::config::Config;
use crate::constants;
use anyhow::Context;
use std::path::Path;
use std::process::{Command, Stdio};

/// Callback for logging git commands and their output.
/// Used to decouple git operations from presentation concerns.
pub type GitLogger = fn(&Config, &[&str], Option<&str>);

/// Default logger that does nothing. Used when no logging is needed.
pub fn no_op_logger(_config: &Config, _args: &[&str], _output: Option<&str>) {}

/// Git command logger for verbose mode.
/// Called with output=None before command execution, output=Some after.
pub fn verbose_logger(config: &Config, args: &[&str], output: Option<&str>) {
    use colored::Colorize;

    if !config.is_verbose() {
        return;
    }

    match output {
        None => {
            // Before execution: print the command
            eprintln!("  {} git {}", "→".cyan(), args.join(" "));
        }
        Some(out) if !out.is_empty() => {
            // After execution: print the output
            for line in out.lines() {
                eprintln!("    {}", line.dimmed());
            }
        }
        _ => {}
    }
}

/// Executes a git command in the specified repository directory with timeout.
pub fn run_git(repo: &Path, config: &Config, args: &[&str]) -> anyhow::Result<String> {
    run_git_with_logger(repo, config, args, no_op_logger)
}

/// Executes a git command with custom logging callback.
/// The logger is called once before execution (output=None) and once after (output=Some).
pub fn run_git_with_logger(
    repo: &Path,
    config: &Config,
    args: &[&str],
    logger: GitLogger,
) -> anyhow::Result<String> {
    logger(config, args, None);

    let mut child = Command::new("git")
        .current_dir(repo)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn git command")?;

    let result = wait_with_timeout(&mut child, constants::git_timeout());

    match result {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                logger(config, args, Some(&stdout));
                Ok(stdout)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("git {} failed: {}", args.join(" "), stderr)
            }
        }
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
    use std::time::Instant;

    let start = Instant::now();
    let poll_interval = std::time::Duration::from_millis(100);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Process has exited, collect output
                let stdout = child
                    .stdout
                    .take()
                    .map(|mut s| {
                        let mut buf = Vec::new();
                        std::io::Read::read_to_end(&mut s, &mut buf)
                            .context("Failed to read stdout from git process")
                            .map(|_| buf)
                    })
                    .transpose()?
                    .unwrap_or_default();

                let stderr = child
                    .stderr
                    .take()
                    .map(|mut s| {
                        let mut buf = Vec::new();
                        std::io::Read::read_to_end(&mut s, &mut buf)
                            .context("Failed to read stderr from git process")
                            .map(|_| buf)
                    })
                    .transpose()?
                    .unwrap_or_default();

                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                // Process still running
                if start.elapsed() > timeout {
                    anyhow::bail!(
                        "git command timed out after {} seconds",
                        timeout.as_secs()
                    );
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => return Err(e).context("Failed to wait for git process"),
        }
    }
}

pub fn get_current_branch(repo: &Path, config: &Config, logger: GitLogger) -> anyhow::Result<String> {
    run_git_with_logger(repo, config, &["rev-parse", "--abbrev-ref", "HEAD"], logger)
        .context("Failed to get current branch")
}

pub fn get_current_commit(repo: &Path, config: &Config, logger: GitLogger) -> anyhow::Result<String> {
    run_git_with_logger(repo, config, &["rev-parse", "HEAD"], logger)
        .context("Failed to get current commit")
}

pub fn has_uncommitted_changes(repo: &Path, config: &Config, logger: GitLogger) -> anyhow::Result<bool> {
    run_git_with_logger(repo, config, &["status", "--porcelain"], logger)
        .map(|output| !output.is_empty())
        .context("Failed to check for uncommitted changes")
}

pub fn fetch_prune(repo: &Path, config: &Config, logger: GitLogger) -> anyhow::Result<()> {
    run_git_with_logger(repo, config, &["fetch", "--prune"], logger)
        .context("Failed to fetch from remote")?;
    Ok(())
}

pub fn stash(repo: &Path, config: &Config, logger: GitLogger) -> anyhow::Result<bool> {
    let output = run_git_with_logger(repo, config, &["stash"], logger)
        .context("Failed to stash changes")?;
    Ok(!output.contains("No local changes to save"))
}

pub fn stash_pop(repo: &Path, config: &Config, logger: GitLogger) -> anyhow::Result<()> {
    run_git_with_logger(repo, config, &["stash", "pop"], logger)
        .context("Failed to pop stash")?;
    Ok(())
}

pub fn checkout(repo: &Path, config: &Config, branch: &str, logger: GitLogger) -> anyhow::Result<()> {
    validate_branch_name(branch)?;
    run_git_with_logger(repo, config, &["checkout", branch], logger)
        .with_context(|| format!("Failed to checkout branch '{}'", branch))?;
    Ok(())
}

pub fn pull(repo: &Path, config: &Config, branch: &str, logger: GitLogger) -> anyhow::Result<()> {
    validate_branch_name(branch)?;
    run_git_with_logger(repo, config, &["pull", "--ff-only", "origin", branch], logger)
        .with_context(|| format!("Failed to pull '{}' from origin", branch))?;
    Ok(())
}

/// Validates branch name to prevent command and argument injection.
fn validate_branch_name(branch: &str) -> anyhow::Result<()> {
    if branch.is_empty() {
        anyhow::bail!("Branch name cannot be empty");
    }

    // Defense-in-depth: block shell metacharacters even though Command doesn't use a shell
    const DANGEROUS_CHARS: &[char] = &['\0', '\n', ';', '|', '&', '$', '`', '(', ')', '{', '}'];
    if let Some(c) = branch.chars().find(|c| DANGEROUS_CHARS.contains(c)) {
        anyhow::bail!("Invalid character '{}' in branch name: {:?}", c, branch);
    }

    // Prevent argument injection (e.g., "--exec=malicious")
    if branch.starts_with('-') {
        anyhow::bail!("Branch name cannot start with '-': {:?}", branch);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_branch_name_accepts_valid_names() {
        assert!(validate_branch_name("main").is_ok());
        assert!(validate_branch_name("master").is_ok());
        assert!(validate_branch_name("feature/new-thing").is_ok());
        assert!(validate_branch_name("feat_123").is_ok());
        assert!(validate_branch_name("bugfix-42").is_ok());
        assert!(validate_branch_name("release/v1.2.3").is_ok());
    }

    #[test]
    fn test_validate_branch_name_rejects_empty() {
        let result = validate_branch_name("");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_validate_branch_name_rejects_shell_metacharacters() {
        let dangerous = [
            "branch;rm -rf /",
            "branch|cat /etc/passwd",
            "branch&echo pwned",
            "branch$USER",
            "branch`whoami`",
            "branch(subshell)",
            "branch{expansion}",
            "branch\nrm -rf /",
            "branch\0null",
        ];

        for name in dangerous {
            let result = validate_branch_name(name);
            assert!(
                result.is_err(),
                "Expected '{}' to be rejected but it was accepted",
                name.escape_debug()
            );
        }
    }

    #[test]
    fn test_validate_branch_name_rejects_argument_injection() {
        let arg_injections = [
            "-exec=malicious",
            "--exec=evil",
            "-branch",
            "--help",
        ];

        for name in arg_injections {
            let result = validate_branch_name(name);
            assert!(
                result.is_err(),
                "Expected '{}' to be rejected but it was accepted",
                name
            );
            assert!(result.unwrap_err().to_string().contains("cannot start with '-'"));
        }
    }

    #[test]
    fn test_validate_branch_name_accepts_unicode() {
        // Git supports unicode in branch names
        assert!(validate_branch_name("feature/新機能").is_ok());
        assert!(validate_branch_name("branch-émoji-🎉").is_ok());
    }
}
