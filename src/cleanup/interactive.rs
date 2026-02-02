//! Interactive cleanup flow orchestration.
//!
//! This module contains the main interactive flow for branch cleanup:
//! - Branch listing and analysis
//! - User selection with back-navigation
//! - Three-tier confirmation (safe/unclear/unmerged)
//! - Deletion with progress reporting
//!
//! The flow is decoupled from terminal I/O via the [`Prompter`] trait,
//! enabling full testability with [`MockPrompter`].
//!
//! [`Prompter`]: crate::prompt::Prompter
//! [`MockPrompter`]: crate::prompt::MockPrompter

use std::collections::HashSet;
use std::path::Path;

use anyhow::Context;

use crate::config::Config;
use crate::git::{self, GitLogger};
use crate::prompt::{ConfirmAction, Prompter};

use super::operations::{delete_branches, detect_main_branch, is_detached_head, list_branches};
use super::types::{
    BranchInfo, CleanupCallbacks, CleanupResult, DeletionMode, DeletionOutcome, InteractiveResult,
};

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

    // Detect main branch once upfront (avoid repeated git calls)
    let main_branch =
        detect_main_branch(repo, config, logger).context("Failed to detect main branch")?;

    // List all branches with their status
    let branches =
        list_branches(repo, config, logger).context("Failed to analyze branches for cleanup")?;

    if branches.is_empty() {
        callbacks.on_no_branches();
        return Ok(None);
    }

    // Selection loop - allows user to go back and re-select
    // Note: No side effects (checkout) happen inside the loop to avoid stale state on Back
    let (selected_indices, current_branch_name, has_dangerous) = loop {
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

        // Check if current branch is selected - will need to switch away after confirmation
        let current_branch_selected = selected.iter().find(|b| b.is_current).map(|b| &b.name);

        if let Some(current_name) = current_branch_selected {
            // Prevent switching to self (shouldn't happen, but defense-in-depth)
            if current_name == main_branch {
                anyhow::bail!("Cannot delete '{}' - it is the main branch", current_name);
            }

            callbacks.on_current_branch_selected(current_name);

            let switch_confirmed =
                prompter.confirm(&format!("Switch to '{}' to continue?", main_branch), true)?;

            if !switch_confirmed {
                callbacks.on_cancelled();
                return Ok(None);
            }
            // Note: Actual checkout deferred until after loop to avoid stale state on Back
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
            ConfirmAction::Yes => {
                break (
                    selected_indices,
                    current_branch_selected.cloned(),
                    has_definitely_unmerged,
                );
            }
            ConfirmAction::No => {
                callbacks.on_cancelled();
                return Ok(None);
            }
            ConfirmAction::Back => continue,
        }
    };

    // Handle current branch switch - deferred from loop to avoid stale state on Back
    let switched_from = if let Some(current_name) = current_branch_name {
        git::checkout(repo, config, main_branch, logger)
            .context("Failed to switch to main branch")?;

        callbacks.on_switched_branch(main_branch);
        Some(current_name) // moved, not cloned
    } else {
        None
    };

    // Reconstruct selected branches with updated is_current flags
    // After switching, the old "current" branch is no longer current
    let selected_branches: Vec<BranchInfo> = selected_indices
        .iter()
        .map(|&i| {
            let mut branch = branches[i].clone();
            if switched_from.is_some() {
                branch.is_current = false;
            }
            branch
        })
        .collect();
    let selected_branch_refs: Vec<&BranchInfo> = selected_branches.iter().collect();

    // Handle dry-run mode
    if dry_run {
        let dry_run_branches: Vec<String> =
            selected_branches.iter().map(|b| b.name.clone()).collect();
        callbacks.on_dry_run(&selected_branch_refs);

        // Update is_current flags if we switched branches
        let remaining: Vec<BranchInfo> = branches
            .into_iter()
            .map(|mut b| {
                if switched_from.is_some() {
                    b.is_current = false;
                }
                b
            })
            .collect();

        return Ok(Some(InteractiveResult {
            result: CleanupResult {
                main_branch: main_branch.to_string(),
                deletions: vec![],
                switched_from,
            },
            remaining,
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
        &selected_branch_refs,
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

    // Update is_current flags: if we switched to main, no listed branch is current
    // (main is excluded from the list since it's protected)
    let remaining: Vec<BranchInfo> = branches
        .into_iter()
        .filter(|b| !deleted_names.contains(b.name.as_str()))
        .map(|mut b| {
            if switched_from.is_some() {
                b.is_current = false;
            }
            b
        })
        .collect();

    let result = CleanupResult {
        main_branch: main_branch.to_string(),
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
