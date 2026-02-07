//! Branch cleanup printing - terminal output for cleanup operations.
//!
//! Provides functions for printing branch lists, deletion results, and summaries.

use colored::Colorize;

use crate::cleanup::{BranchInfo, CleanupResult, DeletionOutcome, DeletionResult};
use crate::config::Config;

use super::formatting::{format_branch_line, BranchListWidths};

/// Width of the summary box divider line.
const SUMMARY_BOX_WIDTH: usize = 60;

/// Prints the header for the branch list.
pub fn print_branch_list_header(widths: &BranchListWidths, config: &Config) {
    if config.is_quiet() {
        return;
    }
    let header = build_branch_list_header(widths);
    eprintln!("{}", header);
}

/// Prints the complete branch list with header.
pub fn print_branch_list(branches: &[BranchInfo], config: &Config) {
    if config.is_quiet() {
        return;
    }

    let widths = BranchListWidths::from_branches(branches);
    print_branch_list_header(&widths, config);

    for branch in branches {
        eprintln!("{}", format_branch_line(branch, &widths));
    }
}

/// Prints a single deletion result (real-time feedback during deletion).
pub fn print_deletion_result(result: &DeletionResult, config: &Config) {
    if config.is_quiet() {
        return;
    }
    let line = build_deletion_result_line(result);
    eprintln!("{}", line);
}

/// Prints the final cleanup summary.
pub fn print_cleanup_summary(result: &CleanupResult, remaining: &[BranchInfo], config: &Config) {
    if config.is_quiet() {
        print_quiet_cleanup_summary(result);
    } else {
        print_normal_cleanup_summary(result, remaining);
    }
}

/// Prints the "Analyzing branches..." message.
pub fn print_analyzing_branches(config: &Config) {
    if config.is_quiet() {
        return;
    }
    eprintln!("{}", "Analyzing branches...".dimmed());
}

/// Prints "No branches available for cleanup" message.
pub fn print_no_branches_to_clean(config: &Config) {
    if config.is_quiet() {
        return;
    }
    eprintln!("{}", "No branches available for cleanup.".yellow().bold());
    eprintln!(
        "{}",
        "All branches are either current or protected.".dimmed()
    );
}

/// Prints "Deleting branches..." header.
pub fn print_deleting_header(config: &Config) {
    if config.is_quiet() {
        return;
    }
    eprintln!("\n{}", "Deleting branches...".cyan());
}

pub(super) fn build_branch_list_header(widths: &BranchListWidths) -> String {
    let header_line = format!(
        "  {:<name_width$}  {:<status_width$}  {}",
        "BRANCH",
        "STATUS",
        "REMOTE",
        name_width = widths.name,
        status_width = widths.status,
    );

    let separator = "─".repeat(widths.total_width());

    format!("{}\n  {}", header_line.white().bold(), separator.dimmed())
}

pub(super) fn build_deletion_result_line(result: &DeletionResult) -> String {
    match &result.outcome {
        DeletionOutcome::Deleted => {
            format!("  {} {}", "✓".green(), result.branch)
        }
        DeletionOutcome::ForceDeleted => {
            format!("  {} {} {}", "✓".green(), result.branch, "(force)".dimmed())
        }
        DeletionOutcome::Skipped { reason } => {
            format!("  {} {}: {}", "○".dimmed(), result.branch, reason.dimmed())
        }
        DeletionOutcome::Failed { error } => {
            format!("  {} {}: {}", "✗".red(), result.branch, error.red())
        }
    }
}

fn print_quiet_cleanup_summary(result: &CleanupResult) {
    // Single pass: count deletions and collect failed errors
    let mut deleted_count = 0;
    let mut failed_errors: Vec<(&str, &str)> = Vec::new();

    for d in &result.deletions {
        match &d.outcome {
            DeletionOutcome::Deleted | DeletionOutcome::ForceDeleted => {
                deleted_count += 1;
            }
            DeletionOutcome::Failed { error } => {
                failed_errors.push((&d.branch, error));
            }
            DeletionOutcome::Skipped { .. } => {}
        }
    }

    println!(
        "{}/{} branches deleted",
        deleted_count,
        result.deletions.len()
    );

    for (branch, error) in &failed_errors {
        eprintln!("error: {}: {}", branch, error);
    }

    if !failed_errors.is_empty() {
        eprintln!("{} failed", failed_errors.len());
    }
}

fn print_normal_cleanup_summary(result: &CleanupResult, remaining: &[BranchInfo]) {
    let output = build_cleanup_summary(result, remaining);
    eprint!("{}", output);
}

pub(super) fn build_cleanup_summary(result: &CleanupResult, remaining: &[BranchInfo]) -> String {
    let mut output = String::new();

    // Header - center plain text first, then apply color to avoid ANSI code width issues
    let line = "═".repeat(SUMMARY_BOX_WIDTH);
    let centered_title = format!("{:^width$}", "Cleanup Complete", width = SUMMARY_BOX_WIDTH);
    output.push_str(&format!("\n{}\n", line.cyan()));
    output.push_str(&format!("{}\n", centered_title.cyan().bold()));
    output.push_str(&format!("{}\n\n", line.cyan()));

    // Note if we switched branches
    if let Some(from_branch) = &result.switched_from {
        output.push_str(&format!(
            "{}: Switched from '{}' to '{}'\n\n",
            "Note".cyan(),
            from_branch,
            result.main_branch
        ));
    }

    // Partition results
    let (deleted, rest): (Vec<_>, Vec<_>) = result.deletions.iter().partition(|d| {
        matches!(
            d.outcome,
            DeletionOutcome::Deleted | DeletionOutcome::ForceDeleted
        )
    });
    let (skipped, failed): (Vec<_>, Vec<_>) = rest
        .into_iter()
        .partition(|d| matches!(d.outcome, DeletionOutcome::Skipped { .. }));

    // Deleted section
    if !deleted.is_empty() {
        output.push_str(&format!(
            "{}\n",
            format!("Deleted ({}):", deleted.len()).green().bold()
        ));
        for d in &deleted {
            let suffix = if matches!(d.outcome, DeletionOutcome::ForceDeleted) {
                " (force)"
            } else {
                ""
            };
            output.push_str(&format!(
                "  {} {}{}\n",
                "✓".green(),
                d.branch,
                suffix.dimmed()
            ));
        }
        output.push('\n');
    }

    // Skipped section
    if !skipped.is_empty() {
        output.push_str(&format!(
            "{}\n",
            format!("Skipped ({}):", skipped.len()).yellow().bold()
        ));
        for d in &skipped {
            if let DeletionOutcome::Skipped { reason } = &d.outcome {
                output.push_str(&format!(
                    "  {} {}: {}\n",
                    "○".dimmed(),
                    d.branch,
                    reason.dimmed()
                ));
            }
        }
        output.push('\n');
    }

    // Failed section
    if !failed.is_empty() {
        output.push_str(&format!(
            "{}\n",
            format!("Failed ({}):", failed.len()).red().bold()
        ));
        for d in &failed {
            if let DeletionOutcome::Failed { error } = &d.outcome {
                output.push_str(&format!("  {} {}\n", "✗".red(), d.branch));
                output.push_str(&format!("    {}: {}\n", "Error".red(), error));
                output.push_str(&format!(
                    "    {}: Use 'git branch -D {}' to force delete\n",
                    "Hint".dimmed(),
                    d.branch
                ));
            }
        }
        output.push('\n');
    }

    // Remaining branches
    if !remaining.is_empty() {
        output.push_str(&format!(
            "{}: {}\n",
            "Remaining branches".white().bold(),
            remaining.len()
        ));
        for branch in remaining {
            let suffix = if branch.is_current {
                " (current)".cyan().to_string()
            } else {
                String::new()
            };
            output.push_str(&format!("  - {}{}\n", branch.name, suffix));
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::{MergeStatus, TrackingStatus};

    fn make_branch(name: &str, is_current: bool, status: MergeStatus) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
            is_current,
            merge_status: status,
            tracking_status: TrackingStatus::RemoteGone,
        }
    }

    fn make_deletion(branch: &str, outcome: DeletionOutcome) -> DeletionResult {
        DeletionResult {
            branch: branch.to_string(),
            outcome,
        }
    }

    #[test]
    fn test_build_branch_list_header_format() {
        let widths = BranchListWidths::from_branches(&[]);
        let header = build_branch_list_header(&widths);
        assert!(header.contains("BRANCH"));
        assert!(header.contains("STATUS"));
        assert!(header.contains("REMOTE"));
        assert!(header.contains("─")); // Separator
    }

    #[test]
    fn test_build_deletion_result_line_deleted() {
        let result = make_deletion("feature", DeletionOutcome::Deleted);
        let line = build_deletion_result_line(&result);
        assert!(line.contains("✓"));
        assert!(line.contains("feature"));
    }

    #[test]
    fn test_build_deletion_result_line_force_deleted() {
        let result = make_deletion("feature", DeletionOutcome::ForceDeleted);
        let line = build_deletion_result_line(&result);
        assert!(line.contains("✓"));
        assert!(line.contains("force"));
    }

    #[test]
    fn test_build_deletion_result_line_skipped() {
        let result = make_deletion(
            "feature",
            DeletionOutcome::Skipped {
                reason: "current branch".to_string(),
            },
        );
        let line = build_deletion_result_line(&result);
        assert!(line.contains("○"));
        assert!(line.contains("current branch"));
    }

    #[test]
    fn test_build_deletion_result_line_failed() {
        let result = make_deletion(
            "feature",
            DeletionOutcome::Failed {
                error: "not merged".to_string(),
            },
        );
        let line = build_deletion_result_line(&result);
        assert!(line.contains("✗"));
        assert!(line.contains("not merged"));
    }

    #[test]
    fn test_build_cleanup_summary_includes_header() {
        let result = CleanupResult {
            main_branch: "master".to_string(),
            deletions: vec![],
            switched_from: None,
        };
        let output = build_cleanup_summary(&result, &[]);
        assert!(output.contains("Cleanup Complete"));
        assert!(output.contains("═")); // Box drawing char
    }

    #[test]
    fn test_build_cleanup_summary_deleted_section() {
        let result = CleanupResult {
            main_branch: "master".to_string(),
            deletions: vec![make_deletion("feature", DeletionOutcome::Deleted)],
            switched_from: None,
        };
        let output = build_cleanup_summary(&result, &[]);
        assert!(output.contains("Deleted (1)"));
        assert!(output.contains("feature"));
    }

    #[test]
    fn test_build_cleanup_summary_skipped_section() {
        let result = CleanupResult {
            main_branch: "master".to_string(),
            deletions: vec![make_deletion(
                "current",
                DeletionOutcome::Skipped {
                    reason: "current branch".to_string(),
                },
            )],
            switched_from: None,
        };
        let output = build_cleanup_summary(&result, &[]);
        assert!(output.contains("Skipped (1)"));
    }

    #[test]
    fn test_build_cleanup_summary_failed_section() {
        let result = CleanupResult {
            main_branch: "master".to_string(),
            deletions: vec![make_deletion(
                "feature",
                DeletionOutcome::Failed {
                    error: "not merged".to_string(),
                },
            )],
            switched_from: None,
        };
        let output = build_cleanup_summary(&result, &[]);
        assert!(output.contains("Failed (1)"));
        assert!(output.contains("Hint")); // Provides recovery hint
    }

    #[test]
    fn test_build_cleanup_summary_remaining_branches() {
        let result = CleanupResult {
            main_branch: "master".to_string(),
            deletions: vec![],
            switched_from: None,
        };
        let remaining = vec![make_branch("develop", false, MergeStatus::Unmerged)];
        let output = build_cleanup_summary(&result, &remaining);
        assert!(output.contains("Remaining branches"));
        assert!(output.contains("develop"));
    }

    #[test]
    fn test_build_cleanup_summary_remaining_current_marked() {
        let result = CleanupResult {
            main_branch: "master".to_string(),
            deletions: vec![],
            switched_from: None,
        };
        let remaining = vec![make_branch("develop", true, MergeStatus::Unmerged)];
        let output = build_cleanup_summary(&result, &remaining);
        assert!(output.contains("(current)"));
    }

    #[test]
    fn test_build_cleanup_summary_shows_switched_from() {
        let result = CleanupResult {
            main_branch: "master".to_string(),
            deletions: vec![make_deletion("feature/old", DeletionOutcome::Deleted)],
            switched_from: Some("feature/old".to_string()),
        };
        let output = build_cleanup_summary(&result, &[]);
        assert!(output.contains("Switched from"));
        assert!(output.contains("feature/old"));
        assert!(output.contains("master"));
    }
}
