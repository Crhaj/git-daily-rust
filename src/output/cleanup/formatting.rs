//! Branch formatting for cleanup display.
//!
//! Provides functions for formatting branch information with aligned columns
//! and semantic coloring.

use colored::Colorize;

use crate::cleanup::{BranchInfo, MergeStatus, TrackingStatus};

/// Column widths for aligned branch list display.
///
/// Use [`BranchListWidths::from_branches`] to calculate optimal widths
/// based on the actual branch names in your list.
#[derive(Debug, Clone, Copy)]
pub struct BranchListWidths {
    pub(super) name: usize,
    pub(super) status: usize,
    pub(super) remote: usize,
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

    pub(super) fn total_width(&self) -> usize {
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

    // Pad plain text FIRST, then colorize to ensure correct alignment
    let status_text = branch_status_text(branch);
    let status_padded = format!("{:<width$}", status_text, width = widths.status);
    let status = colorize_branch_status(branch, &status_padded);

    let remote = format_tracking_status(&branch.tracking_status);
    let warning = format_branch_warning(branch);

    format!(
        "{} {:<name_width$}  {}  {}{}",
        prefix,
        branch.name,
        status,
        remote,
        warning,
        name_width = widths.name,
    )
}

/// Formats a branch for display in a multi-select prompt.
///
/// Creates a compact, colored representation suitable for dialoguer's MultiSelect,
/// showing status information inline with the branch name so users can make
/// informed decisions without referring to a separate list.
///
/// **Important:** Padding is applied to plain text *before* colorization to ensure
/// correct column alignment (ANSI codes don't affect visible width calculations).
///
/// Format: `branch-name          status    remote: state`
///
/// # Example output
///
/// - `feature/login          merged    remote: gone`
/// - `hotfix/v1              unclear   remote: exists  (status unknown)`
/// - `my-branch              (current) remote: gone`
#[must_use]
pub fn format_branch_selection_item(branch: &BranchInfo, widths: &BranchListWidths) -> String {
    // Pad plain text FIRST, then colorize to ensure correct alignment
    // (ANSI escape codes would throw off width calculations)
    let status_text = branch_status_text(branch);
    let status_padded = format!("{:<width$}", status_text, width = widths.status);
    let status = colorize_branch_status(branch, &status_padded);

    let remote_text = format!("remote: {}", tracking_status_text(&branch.tracking_status));
    let remote = colorize_tracking_status(&branch.tracking_status, &remote_text);

    let warning = format_branch_warning_compact(branch);

    format!(
        "{:<name_width$}  {}  {}{}",
        branch.name,
        status,
        remote,
        warning,
        name_width = widths.name,
    )
}

/// Compact warning text for selection items.
pub(super) fn format_branch_warning_compact(branch: &BranchInfo) -> String {
    if branch.is_current {
        return String::new();
    }

    let status = &branch.merge_status;
    if status.is_uncertain() {
        format!("  {}", "(status unknown)".yellow())
    } else if status.requires_caution() {
        format!("  {}", "(unmerged)".yellow())
    } else {
        String::new()
    }
}

/// Returns plain text for branch status (no ANSI colors).
pub(super) fn branch_status_text(branch: &BranchInfo) -> &'static str {
    if branch.is_current {
        "(current branch)"
    } else {
        merge_status_text(&branch.merge_status)
    }
}

/// Returns plain text for merge status.
pub(super) fn merge_status_text(status: &MergeStatus) -> &'static str {
    match status {
        MergeStatus::Merged => "merged",
        MergeStatus::SquashMerged => "squash-merged",
        MergeStatus::Unmerged => "unmerged",
        MergeStatus::Unclear => "unclear",
    }
}

/// Returns plain text for tracking status.
pub(super) fn tracking_status_text(status: &TrackingStatus) -> &'static str {
    match status {
        TrackingStatus::RemoteExists(_) => "exists",
        TrackingStatus::RemoteGone => "gone",
        TrackingStatus::NoUpstream => "local",
        TrackingStatus::Unknown => "unknown",
    }
}

/// Applies semantic color to branch status text.
pub(super) fn colorize_branch_status(branch: &BranchInfo, text: &str) -> String {
    if branch.is_current {
        text.cyan().to_string()
    } else {
        colorize_merge_status(&branch.merge_status, text)
    }
}

/// Applies semantic color to merge status text.
pub(super) fn colorize_merge_status(status: &MergeStatus, text: &str) -> String {
    if status.is_safely_deletable() {
        text.green().to_string()
    } else if status.requires_caution() {
        text.yellow().to_string()
    } else {
        text.magenta().to_string()
    }
}

/// Applies semantic color to tracking status text.
pub(super) fn colorize_tracking_status(status: &TrackingStatus, text: &str) -> String {
    if status.is_active() {
        text.to_string()
    } else {
        text.dimmed().to_string()
    }
}

/// Formats tracking status with colors (convenience wrapper).
pub(super) fn format_tracking_status(status: &TrackingStatus) -> String {
    colorize_tracking_status(status, tracking_status_text(status))
}

pub(super) fn format_branch_warning(branch: &BranchInfo) -> String {
    if branch.is_current {
        return String::new();
    }

    let status = &branch.merge_status;
    if status.is_uncertain() {
        format!("   {}", "status unknown".yellow())
    } else if status.requires_caution() {
        format!("   {}", "unmerged".yellow())
    } else {
        String::new()
    }
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

    /// Strips ANSI escape codes from a string, returning only visible characters.
    fn strip_ansi(s: &str) -> String {
        s.chars()
            .fold((String::new(), false), |(mut acc, in_escape), c| {
                if c == '\x1b' {
                    (acc, true)
                } else if in_escape {
                    (acc, c != 'm')
                } else {
                    acc.push(c);
                    (acc, false)
                }
            })
            .0
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
    fn test_format_branch_line_columns_align_with_colors() {
        colored::control::set_override(true);

        let branches = vec![
            make_branch("branch-a", false, MergeStatus::Merged),
            make_branch("branch-a", true, MergeStatus::Merged),
            make_branch("branch-a", false, MergeStatus::Unmerged),
        ];
        let widths = BranchListWidths::from_branches(&branches);

        let positions: Vec<_> = branches
            .iter()
            .map(|b| strip_ansi(&format_branch_line(b, &widths)).find("gone"))
            .collect();

        assert!(
            positions.iter().all(|p| *p == positions[0]),
            "Remote status columns should align: {:?}",
            positions
        );
    }

    #[test]
    fn test_format_branch_selection_item_includes_status() {
        let branch = make_branch("feature/test", false, MergeStatus::Merged);
        let widths = BranchListWidths::from_branches(std::slice::from_ref(&branch));
        let item = format_branch_selection_item(&branch, &widths);
        assert!(item.contains("feature/test"));
        assert!(item.contains("remote:"));
    }

    #[test]
    fn test_format_branch_selection_item_current_branch() {
        let branch = make_branch("my-branch", true, MergeStatus::Merged);
        let widths = BranchListWidths::from_branches(std::slice::from_ref(&branch));
        let item = format_branch_selection_item(&branch, &widths);
        assert!(item.contains("my-branch"));
        assert!(item.contains("current")); // Shows "(current)" status
    }

    #[test]
    fn test_format_branch_selection_item_unclear_shows_warning() {
        let branch = make_branch("unclear-branch", false, MergeStatus::Unclear);
        let widths = BranchListWidths::from_branches(std::slice::from_ref(&branch));
        let item = format_branch_selection_item(&branch, &widths);
        assert!(item.contains("status unknown"));
    }

    #[test]
    fn test_format_branch_selection_item_unmerged_shows_warning() {
        let branch = make_branch("unmerged-branch", false, MergeStatus::Unmerged);
        let widths = BranchListWidths::from_branches(std::slice::from_ref(&branch));
        let item = format_branch_selection_item(&branch, &widths);
        assert!(item.contains("(unmerged)"));
    }

    #[test]
    fn test_format_branch_selection_item_columns_align_with_colors() {
        colored::control::set_override(true);

        let branches = vec![
            make_branch("short", false, MergeStatus::Merged),
            make_branch("short", true, MergeStatus::Merged),
            make_branch("short", false, MergeStatus::Unclear),
        ];
        let widths = BranchListWidths::from_branches(&branches);

        let positions: Vec<_> = branches
            .iter()
            .map(|b| strip_ansi(&format_branch_selection_item(b, &widths)).find("remote:"))
            .collect();

        assert!(
            positions.iter().all(|p| *p == positions[0]),
            "Columns should align: {:?}",
            positions
        );
    }

    #[test]
    fn test_colorize_merge_status_merged_is_green() {
        colored::control::set_override(true);
        let result = colorize_merge_status(&MergeStatus::Merged, "merged");
        assert!(result.contains("\x1b[32m"), "Expected green ANSI code");
    }

    #[test]
    fn test_colorize_merge_status_unmerged_is_yellow() {
        colored::control::set_override(true);
        let result = colorize_merge_status(&MergeStatus::Unmerged, "unmerged");
        assert!(result.contains("\x1b[33m"), "Expected yellow ANSI code");
    }

    #[test]
    fn test_colorize_merge_status_unclear_is_magenta() {
        colored::control::set_override(true);
        let result = colorize_merge_status(&MergeStatus::Unclear, "unclear");
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
        assert!(warning.contains("status unknown"));
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
}
