//! Post-operation summary formatting for update results.

use std::time::Duration;

use colored::Colorize;

use crate::config::Config;
use crate::repo::{UpdateOutcome, UpdateResult};

use super::format_duration;

/// Prints the update summary based on verbosity setting.
pub fn print_summary(results: &[UpdateResult], duration: Duration, config: &Config) {
    if config.is_quiet() {
        print_quiet_summary(results);
    } else {
        print_normal_summary(results, duration);
    }
}

fn print_quiet_summary(results: &[UpdateResult]) {
    let (stdout_line, stderr_lines) = build_quiet_summary(results);
    println!("{}", stdout_line);
    for line in stderr_lines {
        eprintln!("{}", line);
    }
}

fn print_normal_summary(results: &[UpdateResult], duration: Duration) {
    let output = build_normal_summary(results, duration);
    print!("{}", output);
}

fn build_quiet_summary(results: &[UpdateResult]) -> (String, Vec<String>) {
    let (successes, failures): (Vec<_>, Vec<_>) = results
        .iter()
        .partition(|r| matches!(r.outcome, UpdateOutcome::Success(_)));

    let stdout_line = format!("{}/{} repositories updated", successes.len(), results.len());
    let stderr_lines = failures
        .iter()
        .filter_map(|result| match &result.outcome {
            UpdateOutcome::Failed(failure) => Some(format!(
                "error: {}: {}",
                result.path.display(),
                failure.error
            )),
            _ => None,
        })
        .collect();

    (stdout_line, stderr_lines)
}

fn build_normal_summary(results: &[UpdateResult], duration: Duration) -> String {
    let mut output = String::new();
    output.push_str(&build_section("Summary"));

    let (successes, failures): (Vec<_>, Vec<_>) = results
        .iter()
        .partition(|r| matches!(r.outcome, UpdateOutcome::Success(_)));

    output.push_str(&build_success_lines(&successes));
    output.push_str(&build_failure_lines(&failures));
    output.push_str(&format!(
        "{}: {}/{} repos in {}",
        "Total".white().bold(),
        successes.len(),
        results.len(),
        format_duration(duration)
    ));
    output.push('\n');

    output
}

fn build_section(title: &str) -> String {
    let line = "=".repeat(50).cyan().dimmed();
    let padding = (50 - title.len()) / 2;
    let centered = format!("{:>width$}", title, width = padding + title.len());
    format!("\n{}\n{}\n{}\n\n", line, centered.cyan().bold(), line)
}

fn build_success_lines(successes: &[&UpdateResult]) -> String {
    let mut output = String::new();
    if successes.is_empty() {
        return output;
    }
    output.push_str(&format!(
        "{}",
        format!("Succeeded ({}):", successes.len()).green().bold()
    ));
    output.push('\n');

    for result in successes {
        if let UpdateOutcome::Success(success) = &result.outcome {
            let stash_msg = if success.had_stash {
                " (stash restored)".yellow()
            } else {
                "".normal()
            };
            output.push_str(&format!(
                "  {} {} {} {} in {}",
                "OK".green().bold(),
                result.path.display().to_string().white(),
                success.original_head.display().cyan(),
                stash_msg,
                format_duration(result.duration).dimmed(),
            ));
            output.push('\n');
        }
    }
    output.push('\n');
    output
}

fn build_failure_lines(failures: &[&UpdateResult]) -> String {
    let mut output = String::new();
    if failures.is_empty() {
        return output;
    }

    output.push_str(&format!(
        "{}",
        format!("Failed ({}):", failures.len()).red().bold()
    ));
    output.push('\n');

    for result in failures {
        if let UpdateOutcome::Failed(failure) = &result.outcome {
            output.push_str(&format!(
                "  {} {} {} in {}",
                "FAIL".red().bold(),
                result.path.display().to_string().white(),
                format!("at {:?}: {}", failure.step, failure.error).red(),
                format_duration(result.duration).dimmed(),
            ));
            output.push('\n');
        }
    }
    output.push('\n');
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::{OriginalHead, UpdateFailure, UpdateStep, UpdateSuccess};
    use std::path::PathBuf;

    fn make_success_result(path: &str, branch: &str, had_stash: bool) -> UpdateResult {
        UpdateResult {
            path: PathBuf::from(path),
            outcome: UpdateOutcome::Success(UpdateSuccess {
                original_head: OriginalHead::Branch(branch.to_string()),
                master_branch: "master",
                had_stash,
            }),
            duration: Duration::from_millis(500),
        }
    }

    fn make_failure_result(path: &str, error: &str) -> UpdateResult {
        UpdateResult {
            path: PathBuf::from(path),
            outcome: UpdateOutcome::Failed(UpdateFailure {
                error: error.to_string(),
                step: UpdateStep::Fetching,
            }),
            duration: Duration::from_millis(100),
        }
    }

    #[test]
    fn test_build_quiet_summary_all_success() {
        let results = vec![make_success_result("/repo1", "main", false)];
        let (stdout, stderr) = build_quiet_summary(&results);
        assert_eq!(stdout, "1/1 repositories updated");
        assert!(stderr.is_empty());
    }

    #[test]
    fn test_build_quiet_summary_with_failures() {
        let results = vec![
            make_success_result("/repo1", "main", false),
            make_failure_result("/repo2", "network error"),
        ];
        let (stdout, stderr) = build_quiet_summary(&results);
        assert_eq!(stdout, "1/2 repositories updated");
        assert_eq!(stderr.len(), 1);
        assert!(stderr[0].contains("network error"));
    }

    #[test]
    fn test_build_normal_summary_contains_section() {
        let results = vec![make_success_result("/repo1", "main", false)];
        let output = build_normal_summary(&results, Duration::from_secs(1));
        assert!(output.contains("Summary"));
    }

    #[test]
    fn test_build_normal_summary_shows_stash_restored() {
        let results = vec![make_success_result("/repo1", "feature", true)];
        let output = build_normal_summary(&results, Duration::from_secs(1));
        assert!(output.contains("stash restored"));
    }

    #[test]
    fn test_build_section_format() {
        let section = build_section("Test");
        assert!(section.contains("Test"));
        assert!(section.contains("=")); // Contains separator line
    }

    #[test]
    fn test_build_success_lines_empty() {
        let output = build_success_lines(&[]);
        assert!(output.is_empty());
    }

    #[test]
    fn test_build_failure_lines_empty() {
        let output = build_failure_lines(&[]);
        assert!(output.is_empty());
    }

    #[test]
    fn test_build_failure_lines_contains_error() {
        let result = make_failure_result("/repo", "connection refused");
        let output = build_failure_lines(&[&result]);
        assert!(output.contains("connection refused"));
        assert!(output.contains("FAIL"));
    }
}
