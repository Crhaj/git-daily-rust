mod common;

use common::{init_repo, setup_workspace_with_repos, test_config, CountingCallbacks, TestRepo};
use git_daily_rust::git::{self, no_op_logger};
use git_daily_rust::output::NoOpCallbacks;
use git_daily_rust::repo::{self, OriginalHead, UpdateOutcome, UpdateStep};
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use tempfile::TempDir;

/// Shorthand for the test logger (no-op for tests)
fn logger() -> git::GitLogger {
    no_op_logger
}

#[test]
fn test_repo_creation() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let branch = git::get_current_branch(repo.path(), &test_config(), logger())?;
    assert_eq!(branch, "master");
    Ok(())
}

#[test]
fn test_repo_with_remote() -> anyhow::Result<()> {
    let repo = TestRepo::with_remote(None)?;
    let branch = git::get_current_branch(repo.path(), &test_config(), logger())?;
    assert_eq!(branch, "master");
    git::fetch_prune(repo.path(), &test_config(), logger())?;
    Ok(())
}

#[test]
fn test_create_branch() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;
    repo.create_branch("feature")?;

    let branch = git::get_current_branch(repo.path(), &config, logger())?;
    assert_eq!(branch, "master");

    git::checkout(repo.path(), &config, "feature", logger())?;
    let branch = git::get_current_branch(repo.path(), &config, logger())?;
    assert_eq!(branch, "feature");

    Ok(())
}

#[test]
fn test_make_dirty() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;
    assert!(!git::has_uncommitted_changes(repo.path(), &config, logger())?);
    repo.make_dirty()?;
    assert!(git::has_uncommitted_changes(repo.path(), &config, logger())?);
    Ok(())
}

#[test]
fn test_make_untracked() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;
    assert!(!repo.file_exists("untracked.txt"));
    repo.make_untracked()?;
    assert!(repo.file_exists("untracked.txt"));
    assert!(git::has_uncommitted_changes(repo.path(), &config, logger())?);
    Ok(())
}

#[test]
fn test_has_stash() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;
    assert!(!repo.has_stash()?);
    repo.make_dirty()?;
    git::stash(repo.path(), &config, logger())?;
    assert!(repo.has_stash()?);
    Ok(())
}

#[test]
fn test_file_exists() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    assert!(repo.file_exists("README.md"));
    assert!(!repo.file_exists("nonexistent.txt"));
    Ok(())
}

#[test]
fn test_update_returns_to_original_branch() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(None)?;
    repo.create_branch("feature")?;
    git::checkout(repo.path(), &config, "feature", logger())?;

    let result = repo::update(repo.path(), &NoOpCallbacks, &config);

    assert!(matches!(result.outcome, UpdateOutcome::Success(_)));
    let branch = git::get_current_branch(repo.path(), &config, logger())?;
    assert_eq!(branch, "feature");
    Ok(())
}

#[test]
fn test_update_stashes_and_restores_uncommitted_changes() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(None)?;
    repo.make_dirty()?;
    assert!(git::has_uncommitted_changes(repo.path(), &config, logger())?);

    let result = repo::update(repo.path(), &NoOpCallbacks, &config);

    assert!(matches!(result.outcome, UpdateOutcome::Success(_)));
    assert!(git::has_uncommitted_changes(repo.path(), &config, logger())?);
    assert!(!repo.has_stash()?);
    Ok(())
}

#[test]
fn test_update_untracked_only_no_pop() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(None)?;
    repo.make_untracked()?;
    assert!(!repo.has_stash()?);
    assert!(git::has_uncommitted_changes(repo.path(), &config, logger())?);

    let result = repo::update(repo.path(), &NoOpCallbacks, &config);

    assert!(matches!(result.outcome, UpdateOutcome::Success(_)));
    assert!(!repo.has_stash()?);
    assert!(git::has_uncommitted_changes(repo.path(), &config, logger())?);
    Ok(())
}

#[test]
fn test_update_handles_repo_already_on_main() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(Some("main"))?;
    let branch = git::get_current_branch(repo.path(), &config, logger())?;
    assert_eq!(branch, "main");

    let result = repo::update(repo.path(), &NoOpCallbacks, &config);

    assert!(matches!(result.outcome, UpdateOutcome::Success(_)));
    let branch = git::get_current_branch(repo.path(), &config, logger())?;
    assert_eq!(branch, "main");
    Ok(())
}

#[test]
fn test_update_falls_back_to_main_when_no_master_branch() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(Some("main"))?;
    repo.create_branch("feature")?;
    git::checkout(repo.path(), &config, "feature", logger())?;

    let result = repo::update(repo.path(), &NoOpCallbacks, &config);

    match result.outcome {
        UpdateOutcome::Success(success) => {
            assert_eq!(success.master_branch, "main");
            assert_eq!(success.original_head, OriginalHead::Branch("feature".to_string()));
        }
        UpdateOutcome::Failed(failure) => anyhow::bail!("update failed: {}", failure.error),
    }

    let branch = git::get_current_branch(repo.path(), &config, logger())?;
    assert_eq!(branch, "feature");
    Ok(())
}

#[test]
fn test_update_reports_failure_when_fetch_fails_without_remote() -> anyhow::Result<()> {
    let config = test_config();
    let mut repo = TestRepo::with_remote(None)?;
    repo.remove_remote();

    let result = repo::update(repo.path(), &NoOpCallbacks, &config);

    match result.outcome {
        UpdateOutcome::Failed(failure) => {
            assert_eq!(failure.step, UpdateStep::Fetching);
            // Error should mention fetch or remote in some form
            let error_lower = failure.error.to_lowercase();
            assert!(
                error_lower.contains("fetch") || error_lower.contains("remote"),
                "Expected error to mention 'fetch' or 'remote', got: {}",
                failure.error
            );
        }
        UpdateOutcome::Success(_) => anyhow::bail!("expected update to fail without a remote"),
    }
    Ok(())
}

#[test]
fn test_update_is_idempotent() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(None)?;
    repo.create_branch("feature")?;
    git::checkout(repo.path(), &config, "feature", logger())?;

    // First update
    let result1 = repo::update(repo.path(), &NoOpCallbacks, &config);
    assert!(matches!(result1.outcome, UpdateOutcome::Success(_)));

    // Second update should also succeed
    let result2 = repo::update(repo.path(), &NoOpCallbacks, &config);
    assert!(matches!(result2.outcome, UpdateOutcome::Success(_)));

    // Should still be on feature branch
    assert_eq!(git::get_current_branch(repo.path(), &config, logger())?, "feature");
    Ok(())
}

#[test]
fn test_update_workspace_updates_multiple_repos() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(&workspace, &[("repo-a", "master"), ("repo-b", "master")])?;

    let repos = repo::find_git_repos(workspace.path());
    assert_eq!(repos.len(), 2);

    let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);

    assert_eq!(results.len(), 2);
    assert!(results
        .iter()
        .all(|result| matches!(result.outcome, UpdateOutcome::Success(_))));
    Ok(())
}

#[test]
fn test_workspace_mixed_success_and_failure() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;

    let repo_a = workspace.path().join("repo-a");
    let remote_a = workspace.path().join("repo-a-remote");
    std::fs::create_dir_all(&repo_a)?;
    std::fs::create_dir_all(&remote_a)?;
    init_repo(&repo_a, "master")?;
    git::run_git(&remote_a, &config, &["init", "--bare"])?;
    git::run_git(
        &repo_a,
        &config,
        &["remote", "add", "origin", remote_a.to_str().unwrap()],
    )?;
    git::run_git(&repo_a, &config, &["push", "-u", "origin", "master"])?;

    let repo_b = workspace.path().join("repo-b");
    std::fs::create_dir_all(&repo_b)?;
    init_repo(&repo_b, "master")?;
    git::run_git(&repo_b, &config, &["remote", "add", "origin", "/nonexistent/path"])?;

    let repos = repo::find_git_repos(workspace.path());
    assert_eq!(repos.len(), 2);

    let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);

    assert_eq!(results.len(), 2);

    let (successes, failures): (Vec<_>, Vec<_>) = results
        .iter()
        .partition(|r| matches!(r.outcome, UpdateOutcome::Success(_)));

    assert_eq!(successes.len(), 1);
    assert_eq!(failures.len(), 1);

    let failure = failures.first().unwrap();
    match &failure.outcome {
        UpdateOutcome::Failed(f) => {
            assert_eq!(f.step, UpdateStep::Fetching);
        }
        _ => panic!("Expected failure"),
    }
    Ok(())
}

#[test]
fn test_workspace_with_dirty_repos() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(
        &workspace,
        &[("repo-clean", "master"), ("repo-dirty", "master")],
    )?;

    let dirty_path = workspace.path().join("repo-dirty");
    std::fs::write(dirty_path.join("README.md"), "# Modified\n")?;
    assert!(git::has_uncommitted_changes(&dirty_path, &config, logger())?);

    let repos = repo::find_git_repos(workspace.path());
    let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);

    assert!(results
        .iter()
        .all(|r| matches!(r.outcome, UpdateOutcome::Success(_))));

    assert!(git::has_uncommitted_changes(&dirty_path, &config, logger())?);

    let dirty_result = results
        .iter()
        .find(|r| r.path.ends_with("repo-dirty"))
        .unwrap();
    match &dirty_result.outcome {
        UpdateOutcome::Success(s) => assert!(s.had_stash),
        _ => panic!("Expected success"),
    }
    Ok(())
}

#[test]
fn test_workspace_callbacks_called_for_each_repo() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(
        &workspace,
        &[
            ("repo-a", "master"),
            ("repo-b", "master"),
            ("repo-c", "master"),
        ],
    )?;

    let repos = repo::find_git_repos(workspace.path());
    assert_eq!(repos.len(), 3);

    let (callbacks, step_count, complete_count) = CountingCallbacks::new();
    let results = repo::update_workspace(&repos, |_| callbacks.clone(), &config);

    assert_eq!(results.len(), 3);
    assert_eq!(complete_count.load(Ordering::SeqCst), 3);

    // Each repo goes through ~7 steps (Started, DetectingBranch, CheckingChanges,
    // Fetching, CheckingOut, Pulling, RestoringBranch, Completed)
    const MIN_STEPS_PER_REPO: usize = 7;
    let expected_min_steps = repos.len() * MIN_STEPS_PER_REPO;
    let total_steps = step_count.load(Ordering::SeqCst);
    assert!(
        total_steps >= expected_min_steps,
        "Expected at least {} step callbacks ({} repos × {} steps), got {}",
        expected_min_steps, repos.len(), MIN_STEPS_PER_REPO, total_steps
    );
    Ok(())
}

#[test]
fn test_workspace_repos_on_different_branches() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(
        &workspace,
        &[("repo-on-master", "master"), ("repo-on-main", "main")],
    )?;

    let master_repo = workspace.path().join("repo-on-master");
    let main_repo = workspace.path().join("repo-on-main");

    git::run_git(&master_repo, &config, &["branch", "feature-x"])?;
    git::checkout(&master_repo, &config, "feature-x", logger())?;

    git::run_git(&main_repo, &config, &["branch", "develop"])?;
    git::checkout(&main_repo, &config, "develop", logger())?;

    let repos = repo::find_git_repos(workspace.path());
    let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);

    assert_eq!(results.len(), 2);
    assert!(results
        .iter()
        .all(|r| matches!(r.outcome, UpdateOutcome::Success(_))));

    assert_eq!(git::get_current_branch(&master_repo, &config, logger())?, "feature-x");
    assert_eq!(git::get_current_branch(&main_repo, &config, logger())?, "develop");

    for result in &results {
        match &result.outcome {
            UpdateOutcome::Success(s) => {
                if result.path.ends_with("repo-on-master") {
                    assert_eq!(s.original_head, OriginalHead::Branch("feature-x".to_string()));
                    assert_eq!(s.master_branch, "master");
                } else if result.path.ends_with("repo-on-main") {
                    assert_eq!(s.original_head, OriginalHead::Branch("develop".to_string()));
                    assert_eq!(s.master_branch, "main");
                }
            }
            _ => panic!("Expected success"),
        }
    }
    Ok(())
}

#[test]
fn test_workspace_empty_directory() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    let repos = repo::find_git_repos(workspace.path());
    assert!(repos.is_empty());

    let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);
    assert!(results.is_empty());
    Ok(())
}

#[test]
fn test_workspace_nested_repos_not_discovered() -> anyhow::Result<()> {
    let workspace = TempDir::new()?;

    let outer_repo = workspace.path().join("outer-repo");
    std::fs::create_dir_all(&outer_repo)?;
    init_repo(&outer_repo, "master")?;

    let nested_repo = outer_repo.join("nested-repo");
    std::fs::create_dir_all(&nested_repo)?;
    init_repo(&nested_repo, "master")?;

    let repos = repo::find_git_repos(workspace.path());

    assert_eq!(repos.len(), 1);
    assert!(repos[0].ends_with("outer-repo"));
    Ok(())
}

#[test]
fn test_workspace_order_independence() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(
        &workspace,
        &[("alpha", "master"), ("beta", "master"), ("gamma", "master")],
    )?;

    let repos = repo::find_git_repos(workspace.path());
    let expected_names: HashSet<&str> = ["alpha", "beta", "gamma"].into_iter().collect();

    for _ in 0..3 {
        let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);
        assert_eq!(results.len(), 3);

        let result_names: HashSet<&str> = results
            .iter()
            .filter_map(|r| r.path.file_name())
            .filter_map(|n| n.to_str())
            .collect();

        assert_eq!(result_names, expected_names);
    }
    Ok(())
}

#[test]
fn test_workspace_with_untracked_files_only() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(&workspace, &[("repo-untracked", "master")])?;

    let repo_path = workspace.path().join("repo-untracked");
    std::fs::write(repo_path.join("untracked.txt"), "untracked content\n")?;
    assert!(git::has_uncommitted_changes(&repo_path, &config, logger())?);

    let repos = repo::find_git_repos(workspace.path());
    let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);

    assert_eq!(results.len(), 1);
    match &results[0].outcome {
        UpdateOutcome::Success(s) => {
            assert!(!s.had_stash);
        }
        UpdateOutcome::Failed(f) => panic!("Expected success, got failure: {}", f.error),
    }

    assert!(repo_path.join("untracked.txt").exists());
    Ok(())
}

#[test]
fn test_update_handles_detached_head() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(None)?;

    // Get the current commit SHA before detaching
    let original_commit = git::get_current_commit(repo.path(), &config, logger())?;

    // Detach HEAD
    git::run_git(repo.path(), &config, &["checkout", "--detach", "HEAD"])?;

    // Verify we're in detached HEAD state
    let branch = git::get_current_branch(repo.path(), &config, logger())?;
    assert_eq!(branch, "HEAD", "Expected detached HEAD state");

    let result = repo::update(repo.path(), &NoOpCallbacks, &config);

    match &result.outcome {
        UpdateOutcome::Success(success) => {
            assert!(success.original_head.is_detached(), "Expected detached HEAD state");
            assert_eq!(
                success.original_head,
                OriginalHead::DetachedAt(original_commit.clone()),
                "Expected original_head to be DetachedAt with the commit SHA"
            );
        }
        UpdateOutcome::Failed(failure) => {
            anyhow::bail!("Expected success, got failure: {}", failure.error)
        }
    }

    // Verify we're back at the original commit (still detached)
    let current_commit = git::get_current_commit(repo.path(), &config, logger())?;
    assert_eq!(current_commit, original_commit);

    Ok(())
}
