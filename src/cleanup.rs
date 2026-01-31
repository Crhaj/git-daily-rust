//! Branch cleanup domain logic.
//!
//! Core types and functions for analyzing and deleting stale local branches.
//! Supports detection of both regular merges and squash merges.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Context;

use crate::config::Config;
use crate::constants::{MAIN_BRANCH, MASTER_BRANCH};
use crate::git::{self, GitLogger};

// Git conflict markers (7 characters each)
const CONFLICT_START: &str = "<<<<<<<";
const CONFLICT_MIDDLE: &str = "=======";
const CONFLICT_END: &str = ">>>>>>>";
const CONFLICT_PREFIX: &str = "CONFLICT";

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum MergeStatus {
    /// Fully merged into master/main via regular merge.
    Merged,
    /// Changes present in master/main (squash merge detected via merge-tree).
    SquashMerged,
    /// Definitely has unique changes not in master/main.
    Unmerged,
    /// Cannot determine - merge-tree showed conflicts, may be squash-merged.
    Unclear,
}

impl MergeStatus {
    /// Returns a human-readable label for displaying in the UI.
    ///
    /// Both `Merged` and `SquashMerged` return "merged" since the distinction
    /// is typically not relevant to end users.
    #[must_use]
    pub fn display(&self) -> &'static str {
        match self {
            MergeStatus::Merged | MergeStatus::SquashMerged => "merged",
            MergeStatus::Unmerged => "unmerged",
            MergeStatus::Unclear => "unclear",
        }
    }

    /// Returns true if the branch is safe to delete without force.
    #[must_use]
    pub fn is_safely_deletable(&self) -> bool {
        matches!(self, MergeStatus::Merged | MergeStatus::SquashMerged)
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
    /// Returns a human-readable label for displaying in the UI.
    #[must_use]
    pub fn display(&self) -> &'static str {
        match self {
            TrackingStatus::RemoteExists(_) => "exists",
            TrackingStatus::RemoteGone => "gone",
            TrackingStatus::NoUpstream => "local",
            TrackingStatus::Unknown => "unknown",
        }
    }

    /// Returns the full remote tracking branch reference (e.g., "origin/feature-x").
    #[must_use]
    pub fn remote_name(&self) -> Option<&str> {
        match self {
            TrackingStatus::RemoteExists(name) => Some(name),
            _ => None,
        }
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
    /// If we had to switch away from a branch to delete it, the original branch name.
    pub switched_from: Option<String>,
}

/// Detects the main branch (master or main) for the repository.
///
/// Tries master first, falls back to main. Returns an error if neither exists.
///
/// # Errors
///
/// Returns an error if:
/// - Git commands fail to execute
/// - Neither `master` nor `main` branch exists in the repository
pub fn detect_main_branch(
    repo: &Path,
    config: &Config,
    logger: GitLogger,
) -> anyhow::Result<&'static str> {
    let output = git::list_branches_with_upstream(repo, config, logger)
        .context("Failed to list branches")?;
    detect_main_branch_from_output(&output)
}

/// Determines if a branch is merged into main.
///
/// Uses a two-phase approach:
/// 1. Traditional merge check via `git branch --merged` (fast)
/// 2. Squash-merge detection via `git merge-tree` (catches squash merges)
///
/// Note: This function calls git for each invocation. For checking multiple
/// branches, use [`list_branches`] which pre-computes the merged set for O(1)
/// lookups instead of O(N) git calls.
pub(crate) fn check_merge_status(
    repo: &Path,
    branch: &str,
    main_branch: &str,
    config: &Config,
    logger: GitLogger,
) -> MergeStatus {
    // Phase 1: Traditional merge check (fast)
    if is_traditionally_merged(repo, branch, main_branch, config, logger) {
        return MergeStatus::Merged;
    }

    // Phase 2: Squash-merge check using merge-tree
    check_squash_merge_status(repo, branch, main_branch, config, logger)
}

/// Fetches the set of branches merged into the main branch.
///
/// Returns an empty set if the git command fails. This is intentional:
/// branches not found in this set will fall through to squash-merge detection,
/// which provides a more accurate (though slower) check.
fn get_merged_branches(
    repo: &Path,
    main_branch: &str,
    config: &Config,
    logger: GitLogger,
) -> HashSet<String> {
    let Ok(output) = git::list_merged_branches(repo, config, main_branch, logger) else {
        return HashSet::new();
    };

    output
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim().to_string())
        .collect()
}

/// Checks if a branch is merged using traditional `git branch --merged`.
///
/// Note: For batch operations, prefer using `get_merged_branches` once and
/// checking membership to avoid repeated git calls.
fn is_traditionally_merged(
    repo: &Path,
    branch: &str,
    main_branch: &str,
    config: &Config,
    logger: GitLogger,
) -> bool {
    let merged = get_merged_branches(repo, main_branch, config, logger);
    merged.contains(branch)
}

/// Checks for squash-merge using merge-tree simulation.
fn check_squash_merge_status(
    repo: &Path,
    branch: &str,
    main_branch: &str,
    config: &Config,
    logger: GitLogger,
) -> MergeStatus {
    let base = match git::merge_base(repo, config, main_branch, branch, logger) {
        Ok(b) => b.trim().to_string(),
        Err(_) => return MergeStatus::Unclear,
    };

    match git::merge_tree(repo, config, &base, main_branch, branch, logger) {
        Ok(output) if output.trim().is_empty() => MergeStatus::SquashMerged,
        Ok(output) if contains_conflict_markers(&output) => MergeStatus::Unclear,
        Ok(_) => MergeStatus::Unmerged,
        Err(_) => MergeStatus::Unclear,
    }
}

/// Checks if merge-tree output contains conflict markers.
fn contains_conflict_markers(output: &str) -> bool {
    output.contains(CONFLICT_START)
        || output.contains(CONFLICT_MIDDLE)
        || output.contains(CONFLICT_END)
        || output.lines().any(|line| line.starts_with(CONFLICT_PREFIX))
}

/// Determines the tracking status for a branch given its upstream ref.
///
/// # Arguments
///
/// * `upstream_ref` - The upstream tracking reference (e.g., "origin/feature-x"),
///   or an empty string if the branch has no upstream configured.
pub fn check_tracking_status(
    repo: &Path,
    upstream_ref: &str,
    config: &Config,
    logger: GitLogger,
) -> TrackingStatus {
    if upstream_ref.is_empty() {
        return TrackingStatus::NoUpstream;
    }

    match git::remote_ref_exists(repo, config, upstream_ref, logger) {
        Ok(true) => TrackingStatus::RemoteExists(upstream_ref.to_string()),
        Ok(false) => TrackingStatus::RemoteGone,
        Err(_) => TrackingStatus::Unknown,
    }
}

/// Lists all local branches with their status information.
///
/// Excludes protected branches (master/main) from the list.
///
/// # Errors
///
/// Returns an error if:
/// - Git commands fail to execute
/// - Neither `master` nor `main` branch exists in the repository
pub fn list_branches(
    repo: &Path,
    config: &Config,
    logger: GitLogger,
) -> anyhow::Result<Vec<BranchInfo>> {
    let output = git::list_branches_with_upstream(repo, config, logger)
        .context("Failed to list branches")?;

    let main_branch = detect_main_branch_from_output(&output)?;
    let current_branch = get_current_branch_name(repo, config, logger);

    // Pre-compute merged branches once for O(1) lookup per branch
    let merged_branches = get_merged_branches(repo, main_branch, config, logger);

    let lines: Vec<&str> = output.lines().collect();
    let mut branches = Vec::with_capacity(lines.len());

    for line in lines {
        let (name, upstream) = parse_branch_line(line);

        if should_skip_branch(name, main_branch) {
            continue;
        }

        let is_current = current_branch.as_deref() == Some(name);
        let merge_status =
            check_merge_status_with_cache(repo, name, main_branch, &merged_branches, config, logger);
        let tracking_status = check_tracking_status(repo, upstream, config, logger);

        branches.push(BranchInfo {
            name: name.to_string(),
            is_current,
            merge_status,
            tracking_status,
        });
    }

    Ok(branches)
}

/// Checks merge status using a pre-computed set of merged branches.
///
/// This avoids repeated `git branch --merged` calls when checking multiple branches.
fn check_merge_status_with_cache(
    repo: &Path,
    branch: &str,
    main_branch: &str,
    merged_branches: &HashSet<String>,
    config: &Config,
    logger: GitLogger,
) -> MergeStatus {
    // Phase 1: Check pre-computed merged set (O(1) lookup)
    if merged_branches.contains(branch) {
        return MergeStatus::Merged;
    }

    // Phase 2: Squash-merge check using merge-tree
    check_squash_merge_status(repo, branch, main_branch, config, logger)
}

/// Detects main branch from already-fetched branch list output.
fn detect_main_branch_from_output(output: &str) -> anyhow::Result<&'static str> {
    let has_master = output
        .lines()
        .filter_map(|line| line.split('|').next())
        .any(|b| b == MASTER_BRANCH);

    let has_main = output
        .lines()
        .filter_map(|line| line.split('|').next())
        .any(|b| b == MAIN_BRANCH);

    match (has_master, has_main) {
        (true, _) => Ok(MASTER_BRANCH),
        (false, true) => Ok(MAIN_BRANCH),
        (false, false) => anyhow::bail!(
            "Neither '{}' nor '{}' branch exists",
            MASTER_BRANCH,
            MAIN_BRANCH
        ),
    }
}

/// Gets the current branch name, or None if in detached HEAD state.
fn get_current_branch_name(repo: &Path, config: &Config, logger: GitLogger) -> Option<String> {
    git::get_current_branch(repo, config, logger)
        .ok()
        .filter(|b| b != "HEAD")
}

/// Parses a line from `git for-each-ref` output into (name, upstream).
fn parse_branch_line(line: &str) -> (&str, &str) {
    let mut parts = line.split('|');
    let name = parts.next().unwrap_or("");
    let upstream = parts.next().unwrap_or("");
    (name, upstream)
}

/// Returns true if this branch should be excluded from the cleanup list.
fn should_skip_branch(name: &str, main_branch: &str) -> bool {
    name.is_empty() || name == main_branch || is_protected_branch(name)
}

/// Returns true if the branch is protected and should never be deleted.
fn is_protected_branch(name: &str) -> bool {
    name == MASTER_BRANCH || name == MAIN_BRANCH
}

/// Returns the reason a branch should be skipped, if any.
fn skip_reason(branch: &BranchInfo) -> Option<&'static str> {
    if is_protected_branch(&branch.name) {
        return Some("protected branch");
    }
    if branch.is_current {
        return Some("current branch");
    }
    None
}

/// Deletes a single branch.
///
/// For merged branches, uses safe delete (`git branch -d`).
/// For unmerged branches, behavior depends on `mode`:
/// - `DeletionMode::Safe`: Skip with reason "unmerged"
/// - `DeletionMode::Force`: Force delete with `git branch -D`
///
/// Will skip protected and current branches regardless of mode.
pub fn delete_single_branch(
    repo: &Path,
    branch: &BranchInfo,
    mode: DeletionMode,
    config: &Config,
    logger: GitLogger,
) -> DeletionResult {
    if let Some(reason) = skip_reason(branch) {
        return DeletionResult {
            branch: branch.name.clone(),
            outcome: DeletionOutcome::Skipped {
                reason: reason.to_string(),
            },
        };
    }

    // Merged branches: safe delete
    if branch.merge_status.is_safely_deletable() {
        return match git::delete_branch(repo, config, &branch.name, logger) {
            Ok(()) => DeletionResult {
                branch: branch.name.clone(),
                outcome: DeletionOutcome::Deleted,
            },
            Err(e) => DeletionResult {
                branch: branch.name.clone(),
                outcome: DeletionOutcome::Failed {
                    error: e.to_string(),
                },
            },
        };
    }

    // Unmerged branches: require Force mode
    if mode == DeletionMode::Safe {
        return DeletionResult {
            branch: branch.name.clone(),
            outcome: DeletionOutcome::Skipped {
                reason: "unmerged".to_string(),
            },
        };
    }

    match git::delete_branch_force(repo, config, &branch.name, logger) {
        Ok(()) => DeletionResult {
            branch: branch.name.clone(),
            outcome: DeletionOutcome::ForceDeleted,
        },
        Err(e) => DeletionResult {
            branch: branch.name.clone(),
            outcome: DeletionOutcome::Failed {
                error: e.to_string(),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_status_display() {
        assert_eq!(MergeStatus::Merged.display(), "merged");
        assert_eq!(MergeStatus::SquashMerged.display(), "merged");
        assert_eq!(MergeStatus::Unmerged.display(), "unmerged");
        assert_eq!(MergeStatus::Unclear.display(), "unclear");
    }

    #[test]
    fn test_merge_status_is_safely_deletable() {
        assert!(MergeStatus::Merged.is_safely_deletable());
        assert!(MergeStatus::SquashMerged.is_safely_deletable());
        assert!(!MergeStatus::Unmerged.is_safely_deletable());
        assert!(!MergeStatus::Unclear.is_safely_deletable());
    }

    #[test]
    fn test_tracking_status_display() {
        assert_eq!(
            TrackingStatus::RemoteExists("origin/foo".to_string()).display(),
            "exists"
        );
        assert_eq!(TrackingStatus::RemoteGone.display(), "gone");
        assert_eq!(TrackingStatus::NoUpstream.display(), "local");
        assert_eq!(TrackingStatus::Unknown.display(), "unknown");
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
    fn test_contains_conflict_markers_detects_standard_markers() {
        assert!(contains_conflict_markers(&format!("{} HEAD", CONFLICT_START)));
        assert!(contains_conflict_markers(CONFLICT_MIDDLE));
        assert!(contains_conflict_markers(&format!("{} branch", CONFLICT_END)));
        assert!(contains_conflict_markers("CONFLICT (content): Merge conflict in file.txt"));
    }

    #[test]
    fn test_contains_conflict_markers_rejects_non_markers() {
        assert!(!contains_conflict_markers("normal content"));
        assert!(!contains_conflict_markers("======")); // Only 6 equals, need 7
        assert!(!contains_conflict_markers("some === code"));
    }

    #[test]
    fn test_parse_branch_line_with_upstream() {
        let (name, upstream) = parse_branch_line("feature/foo|origin/feature/foo");
        assert_eq!(name, "feature/foo");
        assert_eq!(upstream, "origin/feature/foo");
    }

    #[test]
    fn test_parse_branch_line_without_upstream() {
        let (name, upstream) = parse_branch_line("feature/foo|");
        assert_eq!(name, "feature/foo");
        assert_eq!(upstream, "");
    }

    #[test]
    fn test_parse_branch_line_malformed() {
        let (name, upstream) = parse_branch_line("just-a-name");
        assert_eq!(name, "just-a-name");
        assert_eq!(upstream, "");
    }

    #[test]
    fn test_is_protected_branch() {
        assert!(is_protected_branch("master"));
        assert!(is_protected_branch("main"));
        assert!(!is_protected_branch("develop"));
        assert!(!is_protected_branch("feature/foo"));
    }

    #[test]
    fn test_deletion_mode_default_is_safe() {
        assert_eq!(DeletionMode::default(), DeletionMode::Safe);
    }

    #[test]
    fn test_skip_reason() {
        let protected = BranchInfo {
            name: "master".to_string(),
            is_current: false,
            merge_status: MergeStatus::Merged,
            tracking_status: TrackingStatus::NoUpstream,
        };
        assert_eq!(skip_reason(&protected), Some("protected branch"));

        let current = BranchInfo {
            name: "feature".to_string(),
            is_current: true,
            merge_status: MergeStatus::Unmerged,
            tracking_status: TrackingStatus::NoUpstream,
        };
        assert_eq!(skip_reason(&current), Some("current branch"));

        let deletable = BranchInfo {
            name: "feature".to_string(),
            is_current: false,
            merge_status: MergeStatus::Merged,
            tracking_status: TrackingStatus::NoUpstream,
        };
        assert_eq!(skip_reason(&deletable), None);
    }

    #[test]
    fn test_should_skip_branch() {
        assert!(should_skip_branch("", "master"));
        assert!(should_skip_branch("master", "master"));
        assert!(should_skip_branch("main", "master")); // main is protected regardless
        assert!(!should_skip_branch("feature/foo", "master"));
    }

    #[test]
    fn test_detect_main_branch_from_output_prefers_master() {
        let output = "main|origin/main\nmaster|origin/master\nfeature|";
        assert_eq!(detect_main_branch_from_output(output).unwrap(), "master");
    }

    #[test]
    fn test_detect_main_branch_from_output_falls_back_to_main() {
        let output = "main|origin/main\nfeature|";
        assert_eq!(detect_main_branch_from_output(output).unwrap(), "main");
    }

    #[test]
    fn test_detect_main_branch_from_output_errors_when_neither() {
        let output = "feature|origin/feature\ndevelop|";
        assert!(detect_main_branch_from_output(output).is_err());
    }
}
