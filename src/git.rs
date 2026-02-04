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
    if !config.is_verbose() {
        return;
    }

    for line in build_verbose_logger_lines(args, output) {
        eprintln!("{}", line);
    }
}

/// Executes a git command in the specified repository directory with timeout.
pub fn run_git(repo: &Path, config: &Config, args: &[&str]) -> anyhow::Result<String> {
    run_git_with_logger(repo, config, args, no_op_logger)
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

fn build_verbose_logger_lines(args: &[&str], output: Option<&str>) -> Vec<String> {
    use colored::Colorize;

    match output {
        None => vec![format!("  {} git {}", "→".cyan(), args.join(" "))],
        Some(out) if !out.is_empty() => out
            .lines()
            .map(|line| format!("    {}", line.dimmed()))
            .collect(),
        _ => Vec::new(),
    }
}

pub fn get_current_branch(
    repo: &Path,
    config: &Config,
    logger: GitLogger,
) -> anyhow::Result<String> {
    run_git_with_logger(repo, config, &["rev-parse", "--abbrev-ref", "HEAD"], logger)
        .context("Failed to get current branch")
}

pub fn get_current_commit(
    repo: &Path,
    config: &Config,
    logger: GitLogger,
) -> anyhow::Result<String> {
    run_git_with_logger(repo, config, &["rev-parse", "HEAD"], logger)
        .context("Failed to get current commit")
}

/// Returns true if the remote tracking ref exists.
///
/// `remote_ref` must be in `<remote>/<branch>` form (for example, `origin/feature-x`),
/// not a full `refs/remotes/...` path.
pub fn remote_ref_exists(
    repo: &Path,
    config: &Config,
    remote_ref: &str,
    logger: GitLogger,
) -> anyhow::Result<bool> {
    validate_remote_ref(remote_ref)?;
    let ref_path = format!("refs/remotes/{}", remote_ref);
    let output = run_git_output(
        repo,
        config,
        &["rev-parse", "--verify", ref_path.as_str()],
        logger,
    )?;
    Ok(output.status.success())
}

pub fn has_uncommitted_changes(
    repo: &Path,
    config: &Config,
    logger: GitLogger,
) -> anyhow::Result<bool> {
    run_git_with_logger(repo, config, &["status", "--porcelain"], logger)
        .map(|output| !output.is_empty())
        .context("Failed to check for uncommitted changes")
}

pub fn stash(repo: &Path, config: &Config, logger: GitLogger) -> anyhow::Result<bool> {
    let output =
        run_git_with_logger(repo, config, &["stash"], logger).context("Failed to stash changes")?;
    Ok(!output.contains("No local changes to save"))
}

pub fn stash_pop(repo: &Path, config: &Config, logger: GitLogger) -> anyhow::Result<()> {
    run_git_with_logger(repo, config, &["stash", "pop"], logger).context("Failed to pop stash")?;
    Ok(())
}

pub fn checkout(
    repo: &Path,
    config: &Config,
    branch: &str,
    logger: GitLogger,
) -> anyhow::Result<()> {
    validate_branch_name(branch)?;
    run_git_with_logger(repo, config, &["checkout", branch], logger)
        .with_context(|| format!("Failed to checkout branch '{}'", branch))?;
    Ok(())
}

pub fn pull(repo: &Path, config: &Config, branch: &str, logger: GitLogger) -> anyhow::Result<()> {
    validate_branch_name(branch)?;
    run_git_with_logger(
        repo,
        config,
        &["pull", "--ff-only", "origin", branch],
        logger,
    )
    .with_context(|| format!("Failed to pull '{}' from origin", branch))?;
    Ok(())
}

/// Fetches from origin and prunes deleted remote branches.
///
/// This ensures local tracking refs are up-to-date before branch analysis.
pub fn fetch_prune(repo: &Path, config: &Config, logger: GitLogger) -> anyhow::Result<()> {
    run_git_with_logger(repo, config, &["fetch", "--prune"], logger)
        .context("Failed to fetch and prune")?;
    Ok(())
}

/// Lists local branches with their upstream tracking refs.
pub fn list_branches_with_upstream(
    repo: &Path,
    config: &Config,
    logger: GitLogger,
) -> anyhow::Result<String> {
    run_git_with_logger(
        repo,
        config,
        &[
            "for-each-ref",
            "--format=%(refname:short)|%(upstream:short)",
            "refs/heads/",
        ],
        logger,
    )
    .context("Failed to get branch names with upstream info")
}

/// Deletes a local branch safely (fails if not fully merged).
pub fn delete_branch(
    repo: &Path,
    config: &Config,
    name: &str,
    logger: GitLogger,
) -> anyhow::Result<()> {
    validate_branch_name(name)?;
    run_git_with_logger(repo, config, &["branch", "-d", name], logger)
        .with_context(|| format!("Failed to delete branch '{}'", name))?;
    Ok(())
}

/// Force deletes a local branch.
pub fn delete_branch_force(
    repo: &Path,
    config: &Config,
    name: &str,
    logger: GitLogger,
) -> anyhow::Result<()> {
    validate_branch_name(name)?;
    run_git_with_logger(repo, config, &["branch", "-D", name], logger)
        .with_context(|| format!("Failed to force delete branch '{}'", name))?;
    Ok(())
}

/// Lists local branches merged into the specified target branch.
pub fn list_merged_branches(
    repo: &Path,
    config: &Config,
    target: &str,
    logger: GitLogger,
) -> anyhow::Result<String> {
    validate_branch_name(target)?;
    run_git_with_logger(repo, config, &["branch", "--merged", target], logger)
        .with_context(|| format!("Failed to list branches merged into '{}'", target))
}

/// Returns the merge-base SHA between two refs.
pub fn merge_base(
    repo: &Path,
    config: &Config,
    ref1: &str,
    ref2: &str,
    logger: GitLogger,
) -> anyhow::Result<String> {
    run_git_with_logger(repo, config, &["merge-base", ref1, ref2], logger)
        .with_context(|| format!("Failed to run merge-base for '{}' and '{}'", ref1, ref2))
}

/// Checks if all commits in a branch have been applied to the target branch.
///
/// Uses `git cherry target branch` which compares commits by patch-id (content hash).
/// Returns true if ALL commits show `-` prefix (meaning they're in target).
/// Returns false if ANY commit shows `+` prefix (meaning it's NOT in target).
///
/// Note: This works for single-commit squash merges but NOT for multi-commit
/// squash merges (the combined patch differs from individual patches).
pub fn is_branch_merged_by_cherry(
    repo: &Path,
    config: &Config,
    target: &str,
    branch: &str,
    logger: GitLogger,
) -> anyhow::Result<bool> {
    validate_branch_name(target)?;
    validate_branch_name(branch)?;
    let output = run_git_with_logger(repo, config, &["cherry", target, branch], logger)
        .with_context(|| format!("Failed to run git cherry {} {}", target, branch))?;

    // Empty output means no commits unique to branch (it's at the same point or behind)
    if output.trim().is_empty() {
        return Ok(true);
    }

    // All lines must start with '-' (commit is in target) for branch to be fully merged
    // Any '+' means there's a commit not in target
    Ok(output.lines().all(|line| line.starts_with('-')))
}

/// Gets files added by a branch since it diverged from another branch.
///
/// Returns a list of file paths that the branch introduced (not just modified).
/// This is used to check if a squash-merged branch's additions are in the target.
pub fn get_files_added_by_branch(
    repo: &Path,
    config: &Config,
    target: &str,
    branch: &str,
    logger: GitLogger,
) -> anyhow::Result<Vec<String>> {
    validate_branch_name(target)?;
    validate_branch_name(branch)?;

    // Get merge-base first
    let merge_base = run_git_with_logger(repo, config, &["merge-base", target, branch], logger)
        .with_context(|| format!("Failed to find merge-base for {} and {}", target, branch))?;
    let merge_base = merge_base.trim();

    // Get files added by branch since merge-base
    let output = run_git_with_logger(
        repo,
        config,
        &["diff", "--name-only", "--diff-filter=A", merge_base, branch],
        logger,
    )
    .with_context(|| format!("Failed to get files added by {}", branch))?;

    Ok(output
        .lines()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

/// Checks if specific files exist in a branch.
pub fn files_exist_in_branch(
    repo: &Path,
    config: &Config,
    branch: &str,
    files: &[String],
    logger: GitLogger,
) -> anyhow::Result<bool> {
    validate_branch_name(branch)?;

    for file in files {
        // Use ls-tree to check if file exists in branch
        // ls-tree returns empty output for non-existent files, error for invalid refs
        let output = run_git_with_logger(
            repo,
            config,
            &["ls-tree", "--name-only", branch, "--", file],
            logger,
        )
        .with_context(|| format!("Failed to check if {} exists in {}", file, branch))?;

        if output.trim().is_empty() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Checks if a branch's modifications are present in the target branch.
///
/// For branches that only modify existing files (no additions), this checks
/// if those modifications have been incorporated into the target. Uses
/// `git diff target branch -- <modified files>` scoped to only the files
/// the branch touched.
///
/// Returns true if the branch has no unique content (modifications are in target).
pub fn branch_changes_in_target(
    repo: &Path,
    config: &Config,
    target: &str,
    branch: &str,
    logger: GitLogger,
) -> anyhow::Result<bool> {
    validate_branch_name(target)?;
    validate_branch_name(branch)?;

    // Get merge-base
    let merge_base = run_git_with_logger(repo, config, &["merge-base", target, branch], logger)
        .with_context(|| format!("Failed to find merge-base for {} and {}", target, branch))?;
    let merge_base = merge_base.trim();

    // Get files modified by branch since merge-base (not added, just modified)
    let modified_output = run_git_with_logger(
        repo,
        config,
        &["diff", "--name-only", "--diff-filter=M", merge_base, branch],
        logger,
    )
    .with_context(|| format!("Failed to get files modified by {}", branch))?;

    let modified_files: Vec<&str> = modified_output.lines().collect();

    // If no files were modified, branch only deleted files or is empty
    // Fall back to checking if there are any differences
    if modified_files.is_empty() {
        let diff = run_git_with_logger(repo, config, &["diff", target, branch], logger)
            .with_context(|| format!("Failed to diff {} {}", target, branch))?;
        return Ok(diff.trim().is_empty());
    }

    // Check if the branch's version of modified files matches target's version
    // Use git diff with specific file paths
    let mut args = vec!["diff", "--quiet", target, branch, "--"];
    args.extend(modified_files);

    // Use run_git_output directly to distinguish between:
    // - Exit 0: no differences
    // - Exit 1: differences exist (not an error)
    // - Other: actual errors (timeout, invalid ref)
    let output = run_git_output(repo, config, &args, logger)
        .with_context(|| format!("Failed to diff {} vs {} for modified files", target, branch))?;

    if output.status.success() {
        Ok(true) // No differences
    } else if output.status.code() == Some(1) {
        Ok(false) // Files differ (expected behavior for git diff --quiet)
    } else {
        // Unexpected exit code - treat as error
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "git diff --quiet failed with unexpected exit code {:?}: {}",
            output.status.code(),
            stderr
        )
    }
}

/// Returns numstat diff output showing line changes between two branches.
///
/// Uses `git diff --numstat from_branch to_branch` which outputs:
/// `<added>\t<removed>\t<filename>` for each changed file.
///
/// This is used to determine if a branch's content is fully contained in another:
/// - If diffing FROM feature TO main shows only additions (removed=0), feature is merged
/// - If there are any removals, feature has content not in main
///
/// Binary files show `-` instead of numbers.
///
/// Note: Empty files that exist in from_branch but not to_branch show as `0\t0\tfilename`.
/// Use [`has_deleted_files`] to catch this edge case.
pub fn diff_numstat(
    repo: &Path,
    config: &Config,
    from_branch: &str,
    to_branch: &str,
    logger: GitLogger,
) -> anyhow::Result<String> {
    validate_branch_name(from_branch)?;
    validate_branch_name(to_branch)?;
    run_git_with_logger(
        repo,
        config,
        &["diff", "--numstat", from_branch, to_branch],
        logger,
    )
    .with_context(|| format!("Failed to diff --numstat {} {}", from_branch, to_branch))
}

/// Checks if there are files that exist in from_branch but not in to_branch.
///
/// Uses `git diff --diff-filter=D --name-only from_branch to_branch` which lists
/// files that would be deleted when going from `from_branch` to `to_branch`.
///
/// This catches edge cases that numstat misses, such as empty files unique to the branch.
pub fn has_deleted_files(
    repo: &Path,
    config: &Config,
    from_branch: &str,
    to_branch: &str,
    logger: GitLogger,
) -> anyhow::Result<bool> {
    validate_branch_name(from_branch)?;
    validate_branch_name(to_branch)?;
    let output = run_git_with_logger(
        repo,
        config,
        &[
            "diff",
            "--diff-filter=D",
            "--name-only",
            from_branch,
            to_branch,
        ],
        logger,
    )
    .with_context(|| {
        format!(
            "Failed to check deleted files {} {}",
            from_branch, to_branch
        )
    })?;
    Ok(!output.trim().is_empty())
}

/// Executes a git command and returns the raw output without interpreting exit status.
fn run_git_output(
    repo: &Path,
    config: &Config,
    args: &[&str],
    logger: GitLogger,
) -> anyhow::Result<std::process::Output> {
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
        Ok(output) => Ok(output),
        Err(e) => {
            // Kill the process if it's still running after timeout
            let _ = child.kill();
            Err(e)
        }
    }
}

fn validate_remote_ref(remote_ref: &str) -> anyhow::Result<()> {
    if remote_ref.is_empty() {
        anyhow::bail!("Remote ref cannot be empty");
    }
    if remote_ref.starts_with("refs/") {
        anyhow::bail!("Remote ref must be in '<remote>/<branch>' form");
    }
    if !remote_ref.contains('/') {
        anyhow::bail!("Remote ref must include a remote name, e.g. 'origin/branch'");
    }
    if remote_ref.starts_with('/') || remote_ref.ends_with('/') {
        anyhow::bail!("Remote ref must be in '<remote>/<branch>' form");
    }
    validate_branch_name(remote_ref)
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
    use std::io;
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;

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
        let arg_injections = ["-exec=malicious", "--exec=evil", "-branch", "--help"];

        for name in arg_injections {
            let result = validate_branch_name(name);
            assert!(
                result.is_err(),
                "Expected '{}' to be rejected but it was accepted",
                name
            );
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("cannot start with '-'")
            );
        }
    }

    #[test]
    fn test_validate_branch_name_accepts_unicode() {
        // Git supports unicode in branch names
        assert!(validate_branch_name("feature/新機能").is_ok());
        assert!(validate_branch_name("branch-émoji-🎉").is_ok());
    }

    #[test]
    fn test_validate_remote_ref_accepts_remote_branch() {
        assert!(validate_remote_ref("origin/feature-x").is_ok());
        assert!(validate_remote_ref("upstream/main").is_ok());
    }

    #[test]
    fn test_validate_remote_ref_rejects_empty() {
        let result = validate_remote_ref("");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_validate_remote_ref_rejects_full_ref_path() {
        let result = validate_remote_ref("refs/remotes/origin/feature-x");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("'<remote>/<branch>'")
        );
    }

    #[test]
    fn test_validate_remote_ref_rejects_missing_remote() {
        let result = validate_remote_ref("feature-x");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("remote name"));
    }

    #[test]
    fn test_validate_remote_ref_rejects_empty_branch() {
        let result = validate_remote_ref("origin/");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("'<remote>/<branch>'")
        );
    }

    #[test]
    fn test_build_verbose_logger_lines_command() {
        colored::control::set_override(false);
        let lines = build_verbose_logger_lines(&["status", "--porcelain"], None);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("git status --porcelain"));
    }

    #[test]
    fn test_build_verbose_logger_lines_output() {
        colored::control::set_override(false);
        let lines = build_verbose_logger_lines(&["status"], Some("line1\nline2"));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("line1"));
        assert!(lines[1].contains("line2"));
    }

    #[test]
    fn test_build_verbose_logger_lines_empty_output() {
        colored::control::set_override(false);
        let lines = build_verbose_logger_lines(&["status"], Some(""));
        assert!(lines.is_empty());
    }

    #[test]
    fn test_verbose_logger_noop_when_not_verbose() {
        colored::control::set_override(false);
        let config = Config {
            verbosity: crate::config::Verbosity::Normal,
        };
        verbose_logger(&config, &["status"], None);
        verbose_logger(&config, &["status"], Some("output"));
    }

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
