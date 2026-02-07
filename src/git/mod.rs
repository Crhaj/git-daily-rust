//! Git command wrappers.
//!
//! Thin wrappers around git CLI commands with error formatting and timeout support.
//! Uses callback-based logging to avoid coupling with presentation layer.
//!
//! # Module Structure
//!
//! - [`runner`] - Low-level execution infrastructure with timeout support
//! - [`commands`] - High-level git command wrappers

mod commands;
mod runner;

// Re-export all public items from submodules
pub use commands::*;
pub use runner::{run_git, run_git_with_logger};

use crate::config::Config;

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
    use crate::config::Verbosity;

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
            verbosity: Verbosity::Normal,
            debug: false,
        };
        verbose_logger(&config, &["status"], None);
        verbose_logger(&config, &["status"], Some("output"));
    }
}
