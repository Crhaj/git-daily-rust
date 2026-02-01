//! Branch cleanup output - listing branches and reporting deletion results.

use colored::Colorize;

use crate::cleanup::{
    BranchInfo, CleanupResult, DeletionOutcome, DeletionResult, MergeStatus, TrackingStatus,
};
use crate::config::Config;

/// Width of the summary box divider line.
const SUMMARY_BOX_WIDTH: usize = 60;

/// Column widths for aligned branch list display.
///
/// Use [`BranchListWidths::from_branches`] to calculate optimal widths
/// based on the actual branch names in your list.
#[derive(Debug, Clone, Copy)]
pub struct BranchListWidths {
    name: usize,
    status: usize,
    remote: usize,
}

impl BranchListWidths {
    const MIN_NAME_WIDTH: usize = 6;
    const MAX_NAME_WIDTH: usize = 60;
    const STATUS_WIDTH: usize = 16; // "(current branch)" is 16 chars
    const REMOTE_WIDTH: usize = 7; // "unknown" is 7 chars

    /// Calculates optimal column widths based on branch data.
    ///
    /// Name width is clamped between `MIN_NAME_WIDTH` (6) and `MAX_NAME_WIDTH` (60)
    /// to ensure readable output even with very long branch names.
    #[must_use]
    pub fn from_branches(branches: &[BranchInfo]) -> Self {
        let max_name = branches
            .iter()
            .map(|b| b.name.len())
            .max()
            .unwrap_or(Self::MIN_NAME_WIDTH)
            .clamp(Self::MIN_NAME_WIDTH, Self::MAX_NAME_WIDTH);

        Self {
            name: max_name,
            status: Self::STATUS_WIDTH,
            remote: Self::REMOTE_WIDTH,
        }
    }

    fn total_width(&self) -> usize {
        self.name + self.status + self.remote + 4 // 4 for spacing between columns
    }
}

/// Formats a single branch line for display.
///
/// The output includes the branch name, merge status, and tracking status
/// with appropriate coloring based on safety.
///
/// # Example
///
/// ```
/// use git_daily_rust::cleanup::{BranchInfo, MergeStatus, TrackingStatus};
/// use git_daily_rust::output::{BranchListWidths, format_branch_line};
///
/// let branch = BranchInfo {
///     name: "feature/login".to_string(),
///     is_current: false,
///     merge_status: MergeStatus::Merged,
///     tracking_status: TrackingStatus::RemoteGone,
/// };
/// let widths = BranchListWidths::from_branches(&[branch.clone()]);
/// let line = format_branch_line(&branch, &widths);
/// assert!(line.contains("feature/login"));
/// ```
#[must_use]
pub fn format_branch_line(branch: &BranchInfo, widths: &BranchListWidths) -> String {
    let prefix = if branch.is_current { "*" } else { " " };
    let name = &branch.name;
    let status = format_branch_status(branch);
    let remote = format_tracking_status(&branch.tracking_status);
    let warning = format_branch_warning(branch);

    format!(
        "{} {:<name_width$}  {:<status_width$}  {}{}",
        prefix,
        name,
        status,
        remote,
        warning,
        name_width = widths.name,
        status_width = widths.status,
    )
}

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

fn format_branch_status(branch: &BranchInfo) -> String {
    if branch.is_current {
        "(current branch)".cyan().to_string()
    } else {
        format_merge_status(&branch.merge_status)
    }
}

/// Formats merge status with semantic color coding.
///
/// Uses semantic methods to avoid duplicating variant knowledge.
fn format_merge_status(status: &MergeStatus) -> String {
    let label = status.to_string();
    if status.is_safely_deletable() {
        label.green().to_string()
    } else if status.requires_caution() {
        label.yellow().to_string()
    } else {
        // Uncertain status (unclear)
        label.magenta().to_string()
    }
}

/// Formats tracking status with semantic color coding.
///
/// Active remotes are normal, inactive are dimmed.
fn format_tracking_status(status: &TrackingStatus) -> String {
    let label = status.to_string();
    if status.is_active() {
        label
    } else {
        label.dimmed().to_string()
    }
}

fn format_branch_warning(branch: &BranchInfo) -> String {
    if branch.is_current {
        return String::new();
    }

    let status = &branch.merge_status;
    if status.is_uncertain() {
        format!("   {}", "may be squash-merged".yellow())
    } else if status.requires_caution() {
        format!("   {}", "unmerged".yellow())
    } else {
        String::new()
    }
}

fn build_branch_list_header(widths: &BranchListWidths) -> String {
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

fn build_deletion_result_line(result: &DeletionResult) -> String {
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

fn build_cleanup_summary(result: &CleanupResult, remaining: &[BranchInfo]) -> String {
    let mut output = String::new();

    // Header - center plain text first, then apply color to avoid ANSI code width issues
    let line = "═".repeat(SUMMARY_BOX_WIDTH);
    let centered_title = format!("{:^width$}", "Cleanup Complete", width = SUMMARY_BOX_WIDTH);
    output.push_str(&format!("\n{}\n", line.cyan()));
    output.push_str(&format!("{}\n", centered_title.cyan().bold()));
    output.push_str(&format!("{}\n\n", line.cyan()));

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
    fn test_branch_list_widths_from_empty() {
        let widths = BranchListWidths::from_branches(&[]);
        assert_eq!(widths.name, BranchListWidths::MIN_NAME_WIDTH);
    }

    #[test]
    fn test_branch_list_widths_clamps_long_names() {
        let long_name = "a".repeat(100);
        let branch = make_branch(&long_name, false, MergeStatus::Merged);
        let widths = BranchListWidths::from_branches(&[branch]);
        assert_eq!(widths.name, BranchListWidths::MAX_NAME_WIDTH);
    }

    #[test]
    fn test_format_branch_line_current_branch() {
        let branch = make_branch("feature/test", true, MergeStatus::Merged);
        let widths = BranchListWidths::from_branches(std::slice::from_ref(&branch));
        let line = format_branch_line(&branch, &widths);
        assert!(line.starts_with("*"));
        assert!(line.contains("feature/test"));
    }

    #[test]
    fn test_format_branch_line_not_current() {
        let branch = make_branch("feature/test", false, MergeStatus::Merged);
        let widths = BranchListWidths::from_branches(std::slice::from_ref(&branch));
        let line = format_branch_line(&branch, &widths);
        assert!(line.starts_with(" "));
    }

    #[test]
    fn test_format_merge_status_merged_is_green() {
        colored::control::set_override(true);
        let result = format_merge_status(&MergeStatus::Merged);
        assert!(result.contains("\x1b[32m"), "Expected green ANSI code");
    }

    #[test]
    fn test_format_merge_status_unmerged_is_yellow() {
        colored::control::set_override(true);
        let result = format_merge_status(&MergeStatus::Unmerged);
        assert!(result.contains("\x1b[33m"), "Expected yellow ANSI code");
    }

    #[test]
    fn test_format_merge_status_unclear_is_magenta() {
        colored::control::set_override(true);
        let result = format_merge_status(&MergeStatus::Unclear);
        assert!(result.contains("\x1b[35m"), "Expected magenta ANSI code");
    }

    #[test]
    fn test_format_tracking_status_emits_dim_for_inactive() {
        colored::control::set_override(true);
        let result = format_tracking_status(&TrackingStatus::RemoteGone);
        assert!(result.contains("\x1b[2m"), "Expected dim ANSI code");

        let result = format_tracking_status(&TrackingStatus::NoUpstream);
        assert!(result.contains("\x1b[2m"), "Expected dim ANSI code");
    }

    #[test]
    fn test_format_tracking_status_no_style_for_active() {
        colored::control::set_override(true);
        let result = format_tracking_status(&TrackingStatus::RemoteExists("origin/foo".into()));
        // Active status should not have ANSI codes
        assert!(
            !result.contains("\x1b["),
            "Active status should not have ANSI styling"
        );
    }

    #[test]
    fn test_format_branch_warning_unmerged() {
        let branch = make_branch("feature", false, MergeStatus::Unmerged);
        let warning = format_branch_warning(&branch);
        assert!(warning.contains("unmerged"));
    }

    #[test]
    fn test_format_branch_warning_unclear() {
        let branch = make_branch("feature", false, MergeStatus::Unclear);
        let warning = format_branch_warning(&branch);
        assert!(warning.contains("squash-merged"));
    }

    #[test]
    fn test_format_branch_warning_merged_no_warning() {
        let branch = make_branch("feature", false, MergeStatus::Merged);
        let warning = format_branch_warning(&branch);
        assert!(warning.is_empty());
    }

    #[test]
    fn test_format_branch_warning_current_no_warning() {
        let branch = make_branch("feature", true, MergeStatus::Unmerged);
        let warning = format_branch_warning(&branch);
        assert!(warning.is_empty());
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
}
