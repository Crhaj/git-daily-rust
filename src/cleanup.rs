//! Branch cleanup domain logic.
//!
//! Core types and functions for analyzing and deleting stale local branches.
//! Supports detection of both regular merges and squash merges.
//!
//! # Architecture
//!
//! This module contains:
//! - Core types (`BranchInfo`, `MergeStatus`, `CleanupResult`, etc.)
//! - Git operations for branch analysis
//! - Interactive orchestration via [`run_interactive`]
//!
//! The interactive flow is decoupled from terminal I/O via the [`Prompter`] trait,
//! enabling full testability with [`MockPrompter`].
//!
//! [`Prompter`]: crate::prompt::Prompter
//! [`MockPrompter`]: crate::prompt::MockPrompter

use std::collections::HashSet;
use std::path::Path;

use anyhow::Context;

use crate::config::Config;
use crate::constants::{MAIN_BRANCH, MASTER_BRANCH};
use crate::git::{self, GitLogger};
use crate::prompt::{ConfirmAction, Prompter};

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
///
/// # Squash-Merge Detection
///
/// Git's `branch --merged` only detects traditional merges. Squash merges
/// rewrite commits, so the branch appears unmerged even though its changes
/// are in main. We detect these using `git merge-tree`, which simulates
/// a merge - if it produces no diff, the changes are already present.
///
/// # Safety Levels
///
/// - **Safe** (`Merged`, `SquashMerged`): No data loss risk
/// - **Uncertain** (`Unclear`): Conflicts prevent verification; may be squash-merged
/// - **Dangerous** (`Unmerged`): Definitely has unique, potentially unrecoverable work
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
    /// Returns true if the branch is safe to delete without force.
    #[must_use]
    pub fn is_safely_deletable(&self) -> bool {
        matches!(self, Self::Merged | Self::SquashMerged)
    }

    /// Returns true if this status requires user caution (potential data loss).
    #[must_use]
    pub fn requires_caution(&self) -> bool {
        matches!(self, Self::Unmerged)
    }

    /// Returns true if the merge status is uncertain and needs verification.
    #[must_use]
    pub fn is_uncertain(&self) -> bool {
        matches!(self, Self::Unclear)
    }
}

impl std::fmt::Display for MergeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Both Merged and SquashMerged display as "merged" since the
        // distinction is typically not relevant to end users.
        let label = match self {
            Self::Merged | Self::SquashMerged => "merged",
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
#[allow(dead_code)] // Used by CLI in Phase 6
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
#[allow(dead_code)] // Called by check_merge_status
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

/// Returns true if the repository is in detached HEAD state.
///
/// This is useful for warning users that they may accidentally delete
/// the branch they were previously on.
pub fn is_detached_head(repo: &Path, config: &Config, logger: GitLogger) -> bool {
    get_current_branch_name(repo, config, logger).is_none()
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
    /// Called when analyzing branches begins.
    fn on_analyzing(&self);

    /// Called when a detached HEAD state is detected.
    fn on_detached_head(&self);

    /// Called when no branches are available to clean.
    fn on_no_branches(&self);

    /// Called to display the branch list before selection.
    fn on_branch_list(&self, branches: &[BranchInfo]);

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

/// Runs the interactive branch cleanup flow.
///
/// This is the main orchestration function that handles:
/// - Branch listing and analysis
/// - User selection with back-navigation
/// - Current branch switching
/// - Three-tier confirmation (safe/unclear/unmerged)
/// - Deletion with progress reporting
///
/// # Architecture
///
/// This function lives in the domain layer but accepts trait objects for:
/// - `prompter`: User interaction (testable with `MockPrompter`)
/// - `callbacks`: Progress reporting (decouples from presentation)
///
/// # Arguments
///
/// * `repo` - Path to the git repository
/// * `dry_run` - If true, show what would be deleted without deleting
/// * `prompter` - Implementation of interactive prompts
/// * `callbacks` - Implementation of progress callbacks
/// * `config` - Application configuration
/// * `logger` - Git command logger
///
/// # Returns
///
/// Returns `Ok(Some(result))` on successful completion, `Ok(None)` if the user
/// cancelled, or `Err` on failure.
///
/// # Errors
///
/// Returns an error if:
/// - Git commands fail
/// - Neither master nor main branch exists
/// - Prompt interaction fails (e.g., terminal error)
pub fn run_interactive(
    repo: &Path,
    dry_run: bool,
    prompter: &dyn Prompter,
    callbacks: &dyn CleanupCallbacks,
    config: &Config,
    logger: GitLogger,
) -> anyhow::Result<Option<InteractiveResult>> {
    callbacks.on_analyzing();

    // Warn if in detached HEAD state
    if is_detached_head(repo, config, logger) {
        callbacks.on_detached_head();
    }

    // List all branches with their status
    let branches =
        list_branches(repo, config, logger).context("Failed to analyze branches for cleanup")?;

    if branches.is_empty() {
        callbacks.on_no_branches();
        return Ok(None);
    }

    // Selection loop - allows user to go back and re-select
    let (selected_indices, switched_from, has_dangerous) = loop {
        callbacks.on_branch_list(&branches);

        // Build selection items for prompt
        let items: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();

        // Get user selection
        let selected_indices = prompter.multi_select("\nSelect branches to delete", &items)?;

        if selected_indices.is_empty() {
            callbacks.on_cancelled();
            return Ok(None);
        }

        let selected: Vec<&BranchInfo> = selected_indices.iter().map(|&i| &branches[i]).collect();

        // Check if current branch is selected - need to switch away first
        let current_branch_selected = selected.iter().find(|b| b.is_current);
        let mut switched_from: Option<String> = None;

        if let Some(current) = current_branch_selected {
            let main_branch =
                detect_main_branch(repo, config, logger).context("Failed to detect main branch")?;

            // Prevent switching to self (shouldn't happen, but defense-in-depth)
            if current.name == main_branch {
                anyhow::bail!("Cannot delete '{}' - it is the main branch", current.name);
            }

            callbacks.on_current_branch_selected(&current.name);

            let switch_confirmed =
                prompter.confirm(&format!("Switch to '{}' to continue?", main_branch), true)?;

            if !switch_confirmed {
                callbacks.on_cancelled();
                return Ok(None);
            }

            // Switch to main branch
            git::checkout(repo, config, main_branch, logger)
                .context("Failed to switch to main branch")?;

            callbacks.on_switched_branch(main_branch);
            switched_from = Some(current.name.clone());
        }

        // Three-tier safety check
        let has_definitely_unmerged = selected.iter().any(|b| b.merge_status.requires_caution());
        let has_unclear = selected.iter().any(|b| b.merge_status.is_uncertain());

        let action = if has_definitely_unmerged {
            // Highest risk: definitely unmerged branches
            let typed_correctly = prompter.type_to_confirm(
                "You selected unmerged branches. Type 'delete' to confirm",
                "delete",
            )?;

            if typed_correctly {
                ConfirmAction::Yes
            } else {
                prompter.confirm_with_back("Confirmation failed. What would you like to do?")?
            }
        } else if has_unclear {
            // Medium risk: unclear branches
            callbacks.on_unclear_warning();
            prompter.confirm_with_back(&format!(
                "Delete {} branch(es) including unclear status?",
                selected.len()
            ))?
        } else {
            // Low risk: all safely deletable
            prompter.confirm_with_back(&format!("Delete {} branch(es)?", selected.len()))?
        };

        match action {
            ConfirmAction::Yes => break (selected_indices, switched_from, has_definitely_unmerged),
            ConfirmAction::No => {
                callbacks.on_cancelled();
                return Ok(None);
            }
            ConfirmAction::Back => continue,
        }
    };

    // Reconstruct selected branches from indices (branches may have been borrowed)
    let selected_branches: Vec<&BranchInfo> =
        selected_indices.iter().map(|&i| &branches[i]).collect();

    // Handle dry-run mode
    if dry_run {
        let dry_run_branches: Vec<String> =
            selected_branches.iter().map(|b| b.name.clone()).collect();
        callbacks.on_dry_run(&selected_branches);

        return Ok(Some(InteractiveResult {
            result: CleanupResult {
                main_branch: detect_main_branch(repo, config, logger)
                    .context("Failed to detect main branch")?
                    .to_string(),
                deletions: vec![],
                switched_from,
            },
            remaining: branches,
            dry_run: true,
            dry_run_branches,
        }));
    }

    // Perform deletions
    callbacks.on_deleting();

    let has_non_safe = selected_branches
        .iter()
        .any(|b| !b.merge_status.is_safely_deletable());

    let deletion_mode = if has_dangerous || has_non_safe {
        DeletionMode::Force
    } else {
        DeletionMode::Safe
    };

    let deletions = delete_branches(
        repo,
        &selected_branches,
        deletion_mode,
        config,
        logger,
        |r| {
            callbacks.on_deletion_result(r);
        },
    );

    // Calculate remaining branches
    let deleted_names: HashSet<_> = deletions
        .iter()
        .filter(|d| {
            matches!(
                d.outcome,
                DeletionOutcome::Deleted | DeletionOutcome::ForceDeleted
            )
        })
        .map(|d| d.branch.as_str())
        .collect();

    let remaining: Vec<BranchInfo> = branches
        .into_iter()
        .filter(|b| !deleted_names.contains(b.name.as_str()))
        .collect();

    let result = CleanupResult {
        main_branch: detect_main_branch(repo, config, logger)
            .context("Failed to detect main branch")?
            .to_string(),
        deletions,
        switched_from,
    };

    Ok(Some(InteractiveResult {
        result,
        remaining,
        dry_run: false,
        dry_run_branches: vec![],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_status_display_trait() {
        assert_eq!(MergeStatus::Merged.to_string(), "merged");
        assert_eq!(MergeStatus::SquashMerged.to_string(), "merged");
        assert_eq!(MergeStatus::Unmerged.to_string(), "unmerged");
        assert_eq!(MergeStatus::Unclear.to_string(), "unclear");
    }

    #[test]
    fn test_merge_status_is_safely_deletable() {
        assert!(MergeStatus::Merged.is_safely_deletable());
        assert!(MergeStatus::SquashMerged.is_safely_deletable());
        assert!(!MergeStatus::Unmerged.is_safely_deletable());
        assert!(!MergeStatus::Unclear.is_safely_deletable());
    }

    #[test]
    fn test_merge_status_requires_caution() {
        assert!(!MergeStatus::Merged.requires_caution());
        assert!(!MergeStatus::SquashMerged.requires_caution());
        assert!(MergeStatus::Unmerged.requires_caution());
        assert!(!MergeStatus::Unclear.requires_caution());
    }

    #[test]
    fn test_merge_status_is_uncertain() {
        assert!(!MergeStatus::Merged.is_uncertain());
        assert!(!MergeStatus::SquashMerged.is_uncertain());
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
        // If git output ever includes additional pipe-delimited fields, we should
        // gracefully ignore them rather than failing
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
