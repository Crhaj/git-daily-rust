//! Git operations for branch analysis and deletion.
//!
//! This module provides functions to:
//! - Detect the main branch (master/main)
//! - List branches with their merge and tracking status
//! - Delete branches (single or batch)

use std::collections::HashSet;
use std::path::Path;

use anyhow::Context;

use crate::config::Config;
use crate::constants::{MAIN_BRANCH, MASTER_BRANCH};
use crate::git::{self, GitLogger};

use super::types::{
    BranchInfo, DeletionMode, DeletionOutcome, DeletionResult, MergeStatus, TrackingStatus,
};

// Git conflict markers (7 characters each)
const CONFLICT_START: &str = "<<<<<<<";
const CONFLICT_MIDDLE: &str = "=======";
const CONFLICT_END: &str = ">>>>>>>";
const CONFLICT_PREFIX: &str = "CONFLICT";

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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
/// # Performance
///
/// - O(1) lookup for traditional merges (pre-computed `git branch --merged`)
/// - O(U * 2) git calls for unmerged branches (merge-base + merge-tree each)
/// - O(N) git calls for remote tracking status (one per branch)
///
/// For repositories with hundreds of branches, consider batch-fetching
/// remote refs with `git ls-remote --heads origin` upfront.
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
        let merge_status = check_merge_status_with_cache(
            repo,
            name,
            main_branch,
            config,
            &merged_branches,
            logger,
        );
        // Note: This performs one git command per branch to check remote status.
        // For repos with many branches, we could batch-fetch all remote refs upfront.
        // Current implementation prioritizes simplicity and correctness.
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
    config: &Config,
    merged_branches: &HashSet<String>,
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
pub(crate) fn detect_main_branch_from_output(output: &str) -> anyhow::Result<&'static str> {
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

/// Returns true if the repository is in detached HEAD state.
///
/// This is useful for warning users that they may accidentally delete
/// the branch they were previously on.
pub fn is_detached_head(repo: &Path, config: &Config, logger: GitLogger) -> bool {
    get_current_branch_name(repo, config, logger).is_none()
}

/// Parses a line from `git for-each-ref` output into (name, upstream).
pub(crate) fn parse_branch_line(line: &str) -> (&str, &str) {
    let mut parts = line.split('|');
    let name = parts.next().unwrap_or("");
    let upstream = parts.next().unwrap_or("");
    (name, upstream)
}

/// Returns true if this branch should be excluded from the cleanup list.
pub(crate) fn should_skip_branch(name: &str, main_branch: &str) -> bool {
    name.is_empty() || name == main_branch || is_protected_branch(name)
}

/// Returns true if the branch is protected and should never be deleted.
pub(crate) fn is_protected_branch(name: &str) -> bool {
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

/// Deletes multiple branches, continuing on failure.
///
/// Calls `on_result` after each deletion to allow progress reporting.
/// Returns all deletion results.
///
/// # Arguments
///
/// * `branches` - The branches to delete
/// * `mode` - Whether to force-delete unmerged branches
/// * `on_result` - Called after each deletion with the result
pub fn delete_branches<F>(
    repo: &Path,
    branches: &[&BranchInfo],
    mode: DeletionMode,
    config: &Config,
    logger: GitLogger,
    mut on_result: F,
) -> Vec<DeletionResult>
where
    F: FnMut(&DeletionResult),
{
    let mut results = Vec::with_capacity(branches.len());
    for branch in branches {
        let result = delete_single_branch(repo, branch, mode, config, logger);
        on_result(&result);
        results.push(result);
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains_conflict_markers_detects_standard_markers() {
        assert!(contains_conflict_markers(&format!(
            "{} HEAD",
            CONFLICT_START
        )));
        assert!(contains_conflict_markers(CONFLICT_MIDDLE));
        assert!(contains_conflict_markers(&format!(
            "{} branch",
            CONFLICT_END
        )));
        assert!(contains_conflict_markers(
            "CONFLICT (content): Merge conflict in file.txt"
        ));
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
    fn test_parse_branch_line_ignores_extra_fields() {
        let (name, upstream) = parse_branch_line("feature|origin/feature|extra|fields");
        assert_eq!(name, "feature");
        assert_eq!(upstream, "origin/feature");
    }

    #[test]
    fn test_is_protected_branch() {
        assert!(is_protected_branch("master"));
        assert!(is_protected_branch("main"));
        assert!(!is_protected_branch("develop"));
        assert!(!is_protected_branch("feature/foo"));
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
