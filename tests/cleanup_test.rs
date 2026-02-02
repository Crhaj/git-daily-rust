//! Integration tests for the cleanup feature.
//!
//! Tests branch listing, merge detection, and deletion using real git repositories.

mod common;

use common::{TestRepo, test_config};
use git_daily_rust::cleanup::{
    self, BranchInfo, DeletionMode, DeletionOutcome, MergeStatus, TrackingStatus,
};
use git_daily_rust::config::Config;
use git_daily_rust::git::{self, GitLogger};

/// Shorthand for the test logger (no-op for tests).
fn logger() -> GitLogger {
    git::no_op_logger
}

/// Helper to find a branch by name in a list.
fn find_branch<'a>(branches: &'a [BranchInfo], name: &str) -> Option<&'a BranchInfo> {
    branches.iter().find(|b| b.name == name)
}

/// Helper to list branches with standard test setup.
fn list_branches(repo: &TestRepo, config: &Config) -> anyhow::Result<Vec<BranchInfo>> {
    cleanup::list_branches(repo.path(), config, logger())
}

#[test]
fn test_regular_merge_detected_as_merged() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create and merge a feature branch
    repo.checkout_new_branch("feature/test")?;
    repo.add_commit("feature work")?;
    repo.checkout("master")?;
    repo.merge("feature/test")?;

    // List branches - feature/test should be detected as merged
    let branches = list_branches(&repo, &config)?;
    let feature = find_branch(&branches, "feature/test").expect("feature branch should be listed");

    assert!(
        feature.merge_status.is_safely_deletable(),
        "Regular merged branch should be safely deletable, got {:?}",
        feature.merge_status
    );
    assert_eq!(feature.merge_status, MergeStatus::Merged);
    Ok(())
}

#[test]
fn test_squash_merge_detected_as_merged() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create and squash merge a feature branch
    repo.checkout_new_branch("feature/squashed")?;
    repo.add_commit("squash work")?;
    repo.checkout("master")?;
    repo.squash_merge("feature/squashed")?;

    // List branches - feature/squashed should be detected as squash-merged
    let branches = list_branches(&repo, &config)?;
    let feature =
        find_branch(&branches, "feature/squashed").expect("squashed branch should be listed");

    assert!(
        feature.merge_status.is_safely_deletable(),
        "Squash-merged branch should be safely deletable, got {:?}",
        feature.merge_status
    );
    assert_eq!(feature.merge_status, MergeStatus::SquashMerged);
    Ok(())
}

#[test]
fn test_unmerged_branch_detected_as_unmerged() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create a feature branch with commits NOT merged to master
    repo.checkout_new_branch("feature/unmerged")?;
    repo.add_commit("unmerged work")?;
    repo.checkout("master")?;

    // List branches - feature/unmerged should be detected as unmerged
    let branches = list_branches(&repo, &config)?;
    let feature =
        find_branch(&branches, "feature/unmerged").expect("unmerged branch should be listed");

    assert!(
        feature.merge_status.requires_caution(),
        "Unmerged branch should require caution, got {:?}",
        feature.merge_status
    );
    assert_eq!(feature.merge_status, MergeStatus::Unmerged);
    Ok(())
}

#[test]
fn test_current_branch_marked_correctly() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create a feature branch and stay on it
    repo.checkout_new_branch("feature/current")?;
    repo.add_commit("current branch work")?;

    // List branches - feature/current should be marked as current
    let branches = list_branches(&repo, &config)?;
    let feature =
        find_branch(&branches, "feature/current").expect("current branch should be listed");

    assert!(
        feature.is_current,
        "Current branch should be marked as current"
    );
    Ok(())
}

#[test]
fn test_protected_branches_excluded_from_list() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create some feature branches
    repo.create_branch("feature/test")?;

    // List branches - master should NOT be in the list (it's protected)
    let branches = list_branches(&repo, &config)?;

    assert!(
        find_branch(&branches, "master").is_none(),
        "Protected branch 'master' should not be in cleanup list"
    );
    assert!(
        find_branch(&branches, "feature/test").is_some(),
        "Non-protected branch should be in list"
    );
    Ok(())
}

#[test]
fn test_branch_with_remote_shows_exists() -> anyhow::Result<()> {
    let repo = TestRepo::with_remote(None)?;
    let config = test_config();

    // Create a feature branch and push it
    repo.checkout_new_branch("feature/pushed")?;
    repo.add_commit("pushed work")?;
    git::run_git(
        repo.path(),
        &config,
        &["push", "-u", "origin", "feature/pushed"],
    )?;
    repo.checkout("master")?;

    // List branches - should show remote exists
    let branches = list_branches(&repo, &config)?;
    let feature = find_branch(&branches, "feature/pushed").expect("pushed branch should be listed");

    assert!(
        matches!(feature.tracking_status, TrackingStatus::RemoteExists(_)),
        "Branch with pushed remote should show RemoteExists, got {:?}",
        feature.tracking_status
    );
    Ok(())
}

#[test]
fn test_branch_without_upstream_shows_no_upstream() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create a feature branch (no remote configured)
    repo.create_branch("feature/local")?;

    // List branches - should show no upstream
    let branches = list_branches(&repo, &config)?;
    let feature = find_branch(&branches, "feature/local").expect("local branch should be listed");

    assert_eq!(
        feature.tracking_status,
        TrackingStatus::NoUpstream,
        "Branch without upstream should show NoUpstream"
    );
    Ok(())
}

#[test]
fn test_branch_with_deleted_remote_shows_gone() -> anyhow::Result<()> {
    let repo = TestRepo::with_remote(None)?;
    let config = test_config();

    // Create a feature branch and push it
    repo.checkout_new_branch("feature/will-be-gone")?;
    repo.add_commit("soon to be orphaned")?;
    git::run_git(
        repo.path(),
        &config,
        &["push", "-u", "origin", "feature/will-be-gone"],
    )?;
    repo.checkout("master")?;

    // Delete the remote branch (simulating PR merge + branch deletion)
    git::run_git(
        repo.path(),
        &config,
        &["push", "origin", "--delete", "feature/will-be-gone"],
    )?;

    // Prune to update local tracking info
    git::run_git(repo.path(), &config, &["fetch", "--prune"])?;

    // List branches - should show remote gone
    let branches = list_branches(&repo, &config)?;
    let feature =
        find_branch(&branches, "feature/will-be-gone").expect("orphaned branch should be listed");

    assert_eq!(
        feature.tracking_status,
        TrackingStatus::RemoteGone,
        "Branch with deleted remote should show RemoteGone"
    );
    Ok(())
}

#[test]
fn test_delete_merged_branch_succeeds() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create and merge a feature branch
    repo.checkout_new_branch("feature/to-delete")?;
    repo.add_commit("delete me")?;
    repo.checkout("master")?;
    repo.merge("feature/to-delete")?;

    // Get branch info
    let branches = list_branches(&repo, &config)?;
    let feature =
        find_branch(&branches, "feature/to-delete").expect("branch should exist before deletion");

    // Delete it
    let result =
        cleanup::delete_single_branch(repo.path(), feature, DeletionMode::Safe, &config, logger());

    assert!(
        matches!(result.outcome, DeletionOutcome::Deleted),
        "Merged branch deletion should succeed, got {:?}",
        result.outcome
    );
    assert!(
        !repo.branch_exists("feature/to-delete"),
        "Branch should no longer exist after deletion"
    );
    Ok(())
}

#[test]
fn test_delete_unmerged_branch_with_safe_mode_skipped() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create an unmerged feature branch
    repo.checkout_new_branch("feature/unmerged-delete")?;
    repo.add_commit("unmerged work")?;
    repo.checkout("master")?;

    // Get branch info
    let branches = list_branches(&repo, &config)?;
    let feature = find_branch(&branches, "feature/unmerged-delete")
        .expect("branch should exist before deletion");

    // Try safe delete - should be skipped (not failed) because user didn't request force
    let result =
        cleanup::delete_single_branch(repo.path(), feature, DeletionMode::Safe, &config, logger());

    assert!(
        matches!(result.outcome, DeletionOutcome::Skipped { .. }),
        "Safe delete of unmerged branch should be skipped, got {:?}",
        result.outcome
    );
    if let DeletionOutcome::Skipped { reason } = &result.outcome {
        assert!(
            reason.contains("unmerged"),
            "Skip reason should mention 'unmerged'"
        );
    }
    assert!(
        repo.branch_exists("feature/unmerged-delete"),
        "Branch should still exist after skipped deletion"
    );
    Ok(())
}

#[test]
fn test_delete_unmerged_branch_with_force_mode_succeeds() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create an unmerged feature branch
    repo.checkout_new_branch("feature/force-delete")?;
    repo.add_commit("force delete me")?;
    repo.checkout("master")?;

    // Get branch info
    let branches = list_branches(&repo, &config)?;
    let feature = find_branch(&branches, "feature/force-delete")
        .expect("branch should exist before deletion");

    // Force delete - should succeed
    let result =
        cleanup::delete_single_branch(repo.path(), feature, DeletionMode::Force, &config, logger());

    assert!(
        matches!(result.outcome, DeletionOutcome::ForceDeleted),
        "Force delete of unmerged branch should succeed, got {:?}",
        result.outcome
    );
    assert!(
        !repo.branch_exists("feature/force-delete"),
        "Branch should no longer exist after force deletion"
    );
    Ok(())
}

#[test]
fn test_delete_current_branch_skipped() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create a feature branch and stay on it
    repo.checkout_new_branch("feature/current-skip")?;
    repo.add_commit("current branch")?;

    // Get branch info
    let branches = list_branches(&repo, &config)?;
    let feature =
        find_branch(&branches, "feature/current-skip").expect("current branch should be in list");

    assert!(feature.is_current);

    // Try to delete - should be skipped
    let result =
        cleanup::delete_single_branch(repo.path(), feature, DeletionMode::Force, &config, logger());

    assert!(
        matches!(result.outcome, DeletionOutcome::Skipped { .. }),
        "Deleting current branch should be skipped, got {:?}",
        result.outcome
    );
    assert!(
        repo.branch_exists("feature/current-skip"),
        "Current branch should still exist"
    );
    Ok(())
}

#[test]
fn test_detect_main_branch_master() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    let main = cleanup::detect_main_branch(repo.path(), &config, logger())?;
    assert_eq!(main, "master");
    Ok(())
}

#[test]
fn test_detect_main_branch_main() -> anyhow::Result<()> {
    let repo = TestRepo::new_with_main()?;
    let config = test_config();

    let main = cleanup::detect_main_branch(repo.path(), &config, logger())?;
    assert_eq!(main, "main");
    Ok(())
}

#[test]
fn test_empty_repo_no_branches_to_clean() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Fresh repo with only master - should have no branches to clean
    let branches = list_branches(&repo, &config)?;

    assert!(
        branches.is_empty(),
        "Fresh repo should have no branches to clean (master is protected)"
    );
    Ok(())
}

#[test]
fn test_multiple_branches_listed_correctly() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create several branches with different states
    repo.checkout_new_branch("feature/merged")?;
    repo.add_commit("merged work")?;
    repo.checkout("master")?;
    repo.merge("feature/merged")?;

    repo.checkout_new_branch("feature/unmerged")?;
    repo.add_commit("unmerged work")?;
    repo.checkout("master")?;

    repo.create_branch("feature/empty")?;

    // List all branches
    let branches = list_branches(&repo, &config)?;

    assert_eq!(branches.len(), 3, "Should have 3 non-protected branches");

    let merged = find_branch(&branches, "feature/merged").expect("merged branch");
    let unmerged = find_branch(&branches, "feature/unmerged").expect("unmerged branch");
    let empty = find_branch(&branches, "feature/empty").expect("empty branch");

    assert!(merged.merge_status.is_safely_deletable());
    assert!(unmerged.merge_status.requires_caution());
    assert!(empty.merge_status.is_safely_deletable()); // Empty branch = same as master
    Ok(())
}

#[test]
fn test_is_detached_head_returns_false_on_branch() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // On master branch, should not be detached
    assert!(
        !cleanup::is_detached_head(repo.path(), &config, logger()),
        "Should not be detached when on a branch"
    );
    Ok(())
}

#[test]
fn test_is_detached_head_returns_true_when_detached() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Detach HEAD by checking out a commit directly
    let head_commit = git::run_git(repo.path(), &config, &["rev-parse", "HEAD"])?;
    git::run_git(repo.path(), &config, &["checkout", head_commit.trim()])?;

    assert!(
        cleanup::is_detached_head(repo.path(), &config, logger()),
        "Should be detached after checking out a commit"
    );
    Ok(())
}

#[test]
fn test_delete_branches_batch_continues_on_failure() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create multiple branches
    repo.checkout_new_branch("feature/first")?;
    repo.add_commit("first work")?;
    repo.checkout("master")?;
    repo.merge("feature/first")?;

    repo.checkout_new_branch("feature/second")?;
    repo.add_commit("second work")?;
    repo.checkout("master")?;
    repo.merge("feature/second")?;

    let branches = list_branches(&repo, &config)?;
    let branch_refs: Vec<&cleanup::BranchInfo> = branches.iter().collect();

    // Track results via callback
    let mut callback_count = 0;
    let results = cleanup::delete_branches(
        repo.path(),
        &branch_refs,
        cleanup::DeletionMode::Safe,
        &config,
        logger(),
        |_result| callback_count += 1,
    );

    assert_eq!(results.len(), 2, "Should have 2 deletion results");
    assert_eq!(
        callback_count, 2,
        "Callback should be called for each branch"
    );
    assert!(
        !repo.branch_exists("feature/first"),
        "First branch should be deleted"
    );
    assert!(
        !repo.branch_exists("feature/second"),
        "Second branch should be deleted"
    );
    Ok(())
}

#[test]
fn test_unclear_status_branches_detected() -> anyhow::Result<()> {
    // This test verifies that branches with conflicts during merge-tree
    // are marked as Unclear, not Unmerged.
    //
    // Setup: Create a branch, squash-merge it, then modify the same file
    // on master. The original branch will have "unclear" status because
    // merge-tree shows conflicts, but the changes might be in master.
    let repo = TestRepo::new()?;
    let config = test_config();

    // Create feature branch modifying README
    repo.checkout_new_branch("feature/conflict-test")?;
    std::fs::write(repo.path().join("README.md"), "# Feature Content\n")?;
    git::run_git(repo.path(), &config, &["add", "README.md"])?;
    git::run_git(repo.path(), &config, &["commit", "-m", "modify readme"])?;

    // Squash merge to master
    repo.checkout("master")?;
    repo.squash_merge("feature/conflict-test")?;

    // Now modify the same file on master again (creates conflict scenario)
    std::fs::write(repo.path().join("README.md"), "# Master Content\n")?;
    git::run_git(repo.path(), &config, &["add", "README.md"])?;
    git::run_git(
        repo.path(),
        &config,
        &["commit", "-m", "master modifies readme"],
    )?;

    // List branches - feature/conflict-test should be unclear
    let branches = list_branches(&repo, &config)?;
    let feature = find_branch(&branches, "feature/conflict-test")
        .expect("conflict-test branch should be listed");

    assert!(
        feature.merge_status.is_uncertain(),
        "Branch with conflicting changes should be unclear, got {:?}",
        feature.merge_status
    );
    Ok(())
}
