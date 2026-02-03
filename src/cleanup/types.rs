//! Core types for branch cleanup operations.
//!
//! This module defines the data structures used throughout the cleanup feature:
//! - Branch information and status enums
//! - Deletion results and outcomes
//! - Interactive flow results

/// Information about a local branch's status relative to the main branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    /// The branch name (e.g., "feature/login").
    pub name: String,
    /// Whether this is the currently checked-out branch.
    pub is_current: bool,
    /// Whether the branch has been merged into main.
    pub merge_status: MergeStatus,
    /// Whether the remote tracking branch still exists.
    pub tracking_status: TrackingStatus,
}

/// The merge status of a branch relative to the main branch.
///
/// Detected using a two-step approach:
/// 1. `git diff --diff-filter=D` to check for files unique to the branch
/// 2. `git diff --numstat` to check for line-level content unique to the branch
///
/// A branch is considered merged if all its content (files and lines) exists in main,
/// even if main has moved ahead with additional commits.
///
/// # Safety Levels
///
/// - **Safe** (`Merged`): No data loss risk - all branch content is in main
/// - **Uncertain** (`Unclear`): Could not verify status
/// - **Dangerous** (`Unmerged`): Has unique content not in main
///
/// # Limitations
///
/// - Mode-only changes (e.g., chmod +x) without content changes are not detected
/// - Renames are shown as delete+add, which conservatively triggers "unmerged"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum MergeStatus {
    /// All branch content is in main (regular merge, squash merge, or cherry-pick).
    Merged,
    /// Has unique content not in master/main.
    Unmerged,
    /// Cannot determine status.
    Unclear,
}

impl MergeStatus {
    /// Returns true if the branch is safe to delete (changes are in main).
    #[must_use]
    pub fn is_safely_deletable(&self) -> bool {
        matches!(self, Self::Merged)
    }

    /// Returns true if this status requires user caution (potential data loss).
    #[must_use]
    pub fn requires_caution(&self) -> bool {
        matches!(self, Self::Unmerged)
    }

    /// Returns true if the merge status is uncertain.
    #[must_use]
    pub fn is_uncertain(&self) -> bool {
        matches!(self, Self::Unclear)
    }
}

impl std::fmt::Display for MergeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Merged => "merged",
            Self::Unmerged => "unmerged",
            Self::Unclear => "unclear",
        };
        write!(f, "{}", label)
    }
}

/// The status of a branch's remote tracking reference.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum TrackingStatus {
    /// Remote tracking branch exists (e.g., "origin/feature-x").
    RemoteExists(String),
    /// Remote tracking branch was deleted from the remote.
    RemoteGone,
    /// No remote tracking configured for this branch (local-only branch).
    NoUpstream,
    /// Could not determine tracking status (git command failed).
    Unknown,
}

impl TrackingStatus {
    /// Returns the full remote tracking branch reference (e.g., "origin/feature-x").
    #[must_use]
    pub fn remote_name(&self) -> Option<&str> {
        match self {
            Self::RemoteExists(name) => Some(name),
            _ => None,
        }
    }

    /// Returns true if the remote tracking branch is actively maintained.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::RemoteExists(_))
    }
}

impl std::fmt::Display for TrackingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::RemoteExists(_) => "exists",
            Self::RemoteGone => "gone",
            Self::NoUpstream => "local",
            Self::Unknown => "unknown",
        };
        write!(f, "{}", label)
    }
}

/// The result of attempting to delete a single branch.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct DeletionResult {
    /// The name of the branch that was targeted for deletion.
    pub branch: String,
    /// The outcome of the deletion attempt.
    pub outcome: DeletionOutcome,
}

/// The outcome of a branch deletion attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeletionOutcome {
    /// Branch was deleted using safe delete (`git branch -d`).
    Deleted,
    /// Branch was deleted using force delete (`git branch -D`).
    ForceDeleted,
    /// Branch was skipped (e.g., protected or current branch).
    Skipped { reason: String },
    /// Deletion failed with an error.
    Failed { error: String },
}

/// Controls how unmerged branches are handled during deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeletionMode {
    /// Only delete branches that are safely deletable (merged).
    /// Unmerged branches will be skipped.
    #[default]
    Safe,
    /// Force delete unmerged branches without additional confirmation.
    /// The caller is responsible for obtaining user confirmation before using this.
    Force,
}

/// The result of a cleanup operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct CleanupResult {
    /// The detected main branch name ("master" or "main").
    pub main_branch: String,
    /// Results for each branch deletion attempt.
    pub deletions: Vec<DeletionResult>,
    /// The branch we switched away from to delete it, if any.
    ///
    /// When the user selects their current branch for deletion, we must first
    /// switch to main before deleting. This field records the original branch
    /// so we can inform the user about the switch.
    ///
    /// `None` when no switch was needed (current branch wasn't selected).
    pub switched_from: Option<String>,
}

/// The result of the interactive cleanup flow.
///
/// Contains all information needed for the presentation layer to display
/// results and summary.
#[derive(Debug)]
pub struct InteractiveResult {
    /// The cleanup result with deletions and metadata.
    pub result: CleanupResult,
    /// Branches remaining after cleanup (for summary display).
    pub remaining: Vec<BranchInfo>,
    /// Whether dry-run mode was active (no actual deletions).
    pub dry_run: bool,
    /// Branches that would have been deleted in dry-run mode.
    pub dry_run_branches: Vec<String>,
}

/// Callback trait for cleanup progress reporting.
///
/// Allows the presentation layer to handle output without coupling
/// the domain logic to specific output mechanisms.
pub trait CleanupCallbacks {
    /// Called when fetching/pruning begins.
    fn on_fetching(&self);

    /// Called when analyzing branches begins.
    fn on_analyzing(&self);

    /// Called when a detached HEAD state is detected.
    fn on_detached_head(&self);

    /// Called when no branches are available to clean.
    fn on_no_branches(&self);

    /// Called when the user selects their current branch for deletion.
    fn on_current_branch_selected(&self, branch_name: &str);

    /// Called after switching away from the current branch.
    fn on_switched_branch(&self, to_branch: &str);

    /// Called when unclear branches are selected (warning).
    fn on_unclear_warning(&self);

    /// Called before deletion begins.
    fn on_deleting(&self);

    /// Called after each branch deletion.
    fn on_deletion_result(&self, result: &DeletionResult);

    /// Called when the user cancels the operation.
    fn on_cancelled(&self);

    /// Called in dry-run mode to show what would be deleted.
    fn on_dry_run(&self, branches: &[&BranchInfo]);
}

/// No-op callbacks for when progress tracking is not needed.
///
/// This is the null object pattern for `CleanupCallbacks` - use it in tests
/// or when you don't need any output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoOpCleanupCallbacks;

impl CleanupCallbacks for NoOpCleanupCallbacks {
    fn on_fetching(&self) {}
    fn on_analyzing(&self) {}
    fn on_detached_head(&self) {}
    fn on_no_branches(&self) {}
    fn on_current_branch_selected(&self, _branch_name: &str) {}
    fn on_switched_branch(&self, _to_branch: &str) {}
    fn on_unclear_warning(&self) {}
    fn on_deleting(&self) {}
    fn on_deletion_result(&self, _result: &DeletionResult) {}
    fn on_cancelled(&self) {}
    fn on_dry_run(&self, _branches: &[&BranchInfo]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_status_display_trait() {
        assert_eq!(MergeStatus::Merged.to_string(), "merged");
        assert_eq!(MergeStatus::Unmerged.to_string(), "unmerged");
        assert_eq!(MergeStatus::Unclear.to_string(), "unclear");
    }

    #[test]
    fn test_merge_status_is_safely_deletable() {
        assert!(MergeStatus::Merged.is_safely_deletable());
        assert!(!MergeStatus::Unmerged.is_safely_deletable());
        assert!(!MergeStatus::Unclear.is_safely_deletable());
    }

    #[test]
    fn test_merge_status_requires_caution() {
        assert!(!MergeStatus::Merged.requires_caution());
        assert!(MergeStatus::Unmerged.requires_caution());
        assert!(!MergeStatus::Unclear.requires_caution());
    }

    #[test]
    fn test_merge_status_is_uncertain() {
        assert!(!MergeStatus::Merged.is_uncertain());
        assert!(!MergeStatus::Unmerged.is_uncertain());
        assert!(MergeStatus::Unclear.is_uncertain());
    }

    #[test]
    fn test_tracking_status_display_trait() {
        assert_eq!(
            TrackingStatus::RemoteExists("origin/foo".to_string()).to_string(),
            "exists"
        );
        assert_eq!(TrackingStatus::RemoteGone.to_string(), "gone");
        assert_eq!(TrackingStatus::NoUpstream.to_string(), "local");
        assert_eq!(TrackingStatus::Unknown.to_string(), "unknown");
    }

    #[test]
    fn test_tracking_status_is_active() {
        assert!(TrackingStatus::RemoteExists("origin/foo".to_string()).is_active());
        assert!(!TrackingStatus::RemoteGone.is_active());
        assert!(!TrackingStatus::NoUpstream.is_active());
        assert!(!TrackingStatus::Unknown.is_active());
    }

    #[test]
    fn test_tracking_status_remote_name() {
        assert_eq!(
            TrackingStatus::RemoteExists("origin/foo".to_string()).remote_name(),
            Some("origin/foo")
        );
        assert_eq!(TrackingStatus::RemoteGone.remote_name(), None);
        assert_eq!(TrackingStatus::NoUpstream.remote_name(), None);
        assert_eq!(TrackingStatus::Unknown.remote_name(), None);
    }

    #[test]
    fn test_deletion_mode_default_is_safe() {
        assert_eq!(DeletionMode::default(), DeletionMode::Safe);
    }
}
