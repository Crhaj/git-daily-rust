mod common;

use common::{init_repo, setup_workspace_with_repos, CountingCallbacks, TestRepo};
use git_daily_rust::git;
use git_daily_rust::repo::{self, NoOpCallbacks, UpdateOutcome, UpdateStep};
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use tempfile::TempDir;

#[test]
fn test_repo_creation() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    let branch = git::get_current_branch(repo.path())?;
    assert_eq!(branch, "master");
    Ok(())
}

#[test]
fn test_repo_with_remote() -> anyhow::Result<()> {
    let (repo, _remote) = TestRepo::with_remote(None)?;
    let branch = git::get_current_branch(repo.path())?;
    assert_eq!(branch, "master");
    git::fetch_prune(repo.path())?;
    Ok(())
}

#[test]
fn test_create_branch() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    repo.create_branch("feature")?;

    let branch = git::get_current_branch(repo.path())?;
    assert_eq!(branch, "master");

    git::checkout(repo.path(), "feature")?;
    let branch = git::get_current_branch(repo.path())?;
    assert_eq!(branch, "feature");

    Ok(())
}

#[test]
fn test_make_dirty() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    assert!(!git::has_uncommitted_changes(repo.path())?);
    repo.make_dirty()?;
    assert!(git::has_uncommitted_changes(repo.path())?);
    Ok(())
}

#[test]
fn test_make_untracked() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    assert!(!repo.file_exists("untracked.txt"));
    repo.make_untracked()?;
    assert!(repo.file_exists("untracked.txt"));
    assert!(git::has_uncommitted_changes(repo.path())?);
    Ok(())
}

#[test]
fn test_has_stash() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;
    assert!(!repo.has_stash()?);
    repo.make_dirty()?;
    git::stash(repo.path())?;
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
    let (repo, _remote) = TestRepo::with_remote(None)?;
    repo.create_branch("feature")?;
    git::checkout(repo.path(), "feature")?;

    let result = repo::update(repo.path(), |_| {});

    assert!(matches!(result.outcome, UpdateOutcome::Success(_)));
    let branch = git::get_current_branch(repo.path())?;
    assert_eq!(branch, "feature");
    Ok(())
}

#[test]
fn test_update_stashes_and_restores_uncommitted_changes() -> anyhow::Result<()> {
    let (repo, _remote) = TestRepo::with_remote(None)?;
    repo.make_dirty()?;
    assert!(git::has_uncommitted_changes(repo.path())?);

    let result = repo::update(repo.path(), |_| {});

    assert!(matches!(result.outcome, UpdateOutcome::Success(_)));
    assert!(git::has_uncommitted_changes(repo.path())?);
    assert!(!repo.has_stash()?);
    Ok(())
}

#[test]
fn test_update_untracked_only_no_pop() -> anyhow::Result<()> {
    let (repo, _remote) = TestRepo::with_remote(None)?;
    repo.make_untracked()?;
    assert!(!repo.has_stash()?);
    assert!(git::has_uncommitted_changes(repo.path())?);

    let result = repo::update(repo.path(), |_| {});

    assert!(matches!(result.outcome, UpdateOutcome::Success(_)));
    assert!(!repo.has_stash()?);
    assert!(git::has_uncommitted_changes(repo.path())?);
    Ok(())
}

#[test]
fn test_update_handles_repo_already_on_main() -> anyhow::Result<()> {
    let (repo, _remote) = TestRepo::with_remote(Some("main"))?;
    let branch = git::get_current_branch(repo.path())?;
    assert_eq!(branch, "main");

    let result = repo::update(repo.path(), |_| {});

    assert!(matches!(result.outcome, UpdateOutcome::Success(_)));
    let branch = git::get_current_branch(repo.path())?;
    assert_eq!(branch, "main");
    Ok(())
}

#[test]
fn test_update_falls_back_to_main_when_no_master_branch() -> anyhow::Result<()> {
    let (repo, _remote) = TestRepo::with_remote(Some("main"))?;
    repo.create_branch("feature")?;
    git::checkout(repo.path(), "feature")?;

    let result = repo::update(repo.path(), |_| {});

    match result.outcome {
        UpdateOutcome::Success(success) => {
            assert_eq!(success.master_branch, "main");
            assert_eq!(success.original_branch, "feature");
        }
        UpdateOutcome::Failed(failure) => anyhow::bail!("update failed: {}", failure.error),
    }

    let branch = git::get_current_branch(repo.path())?;
    assert_eq!(branch, "feature");
    Ok(())
}

#[test]
fn test_update_reports_failure_when_fetch_fails_without_remote() -> anyhow::Result<()> {
    let (repo, remote) = TestRepo::with_remote(None)?;
    drop(remote);

    let result = repo::update(repo.path(), |_| {});

    match result.outcome {
        UpdateOutcome::Failed(failure) => {
            assert_eq!(failure.step, UpdateStep::Fetching);
            assert!(failure.error.contains("fetch"));
        }
        UpdateOutcome::Success(_) => anyhow::bail!("expected update to fail without a remote"),
    }
    Ok(())
}

#[test]
fn test_update_workspace_updates_multiple_repos() -> anyhow::Result<()> {
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(&workspace, &[("repo-a", "master"), ("repo-b", "master")])?;

    let repos = repo::find_git_repos(workspace.path());
    assert_eq!(repos.len(), 2);

    let results = repo::update_workspace(&repos, |_| NoOpCallbacks);

    assert_eq!(results.len(), 2);
    assert!(results
        .iter()
        .all(|result| matches!(result.outcome, UpdateOutcome::Success(_))));
    Ok(())
}

#[test]
fn test_workspace_mixed_success_and_failure() -> anyhow::Result<()> {
    let workspace = TempDir::new()?;

    let repo_a = workspace.path().join("repo-a");
    let remote_a = workspace.path().join("repo-a-remote");
    std::fs::create_dir_all(&repo_a)?;
    std::fs::create_dir_all(&remote_a)?;
    init_repo(&repo_a, "master")?;
    git::run_git(&remote_a, &["init", "--bare"])?;
    git::run_git(
        &repo_a,
        &["remote", "add", "origin", remote_a.to_str().unwrap()],
    )?;
    git::run_git(&repo_a, &["push", "-u", "origin", "master"])?;

    let repo_b = workspace.path().join("repo-b");
    std::fs::create_dir_all(&repo_b)?;
    init_repo(&repo_b, "master")?;
    git::run_git(&repo_b, &["remote", "add", "origin", "/nonexistent/path"])?;

    let repos = repo::find_git_repos(workspace.path());
    assert_eq!(repos.len(), 2);

    let results = repo::update_workspace(&repos, |_| NoOpCallbacks);

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
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(
        &workspace,
        &[("repo-clean", "master"), ("repo-dirty", "master")],
    )?;

    let dirty_path = workspace.path().join("repo-dirty");
    std::fs::write(dirty_path.join("README.md"), "# Modified\n")?;
    assert!(git::has_uncommitted_changes(&dirty_path)?);

    let repos = repo::find_git_repos(workspace.path());
    let results = repo::update_workspace(&repos, |_| NoOpCallbacks);

    assert!(results
        .iter()
        .all(|r| matches!(r.outcome, UpdateOutcome::Success(_))));

    assert!(git::has_uncommitted_changes(&dirty_path)?);

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
    let results = repo::update_workspace_with(&repos, callbacks);

    assert_eq!(results.len(), 3);
    assert_eq!(complete_count.load(Ordering::SeqCst), 3);

    let total_steps = step_count.load(Ordering::SeqCst);
    assert!(
        total_steps >= 21,
        "Expected at least 21 step callbacks, got {}",
        total_steps
    );
    Ok(())
}

#[test]
fn test_workspace_repos_on_different_branches() -> anyhow::Result<()> {
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(
        &workspace,
        &[("repo-on-master", "master"), ("repo-on-main", "main")],
    )?;

    let master_repo = workspace.path().join("repo-on-master");
    let main_repo = workspace.path().join("repo-on-main");

    git::run_git(&master_repo, &["branch", "feature-x"])?;
    git::checkout(&master_repo, "feature-x")?;

    git::run_git(&main_repo, &["branch", "develop"])?;
    git::checkout(&main_repo, "develop")?;

    let repos = repo::find_git_repos(workspace.path());
    let results = repo::update_workspace(&repos, |_| NoOpCallbacks);

    assert_eq!(results.len(), 2);
    assert!(results
        .iter()
        .all(|r| matches!(r.outcome, UpdateOutcome::Success(_))));

    assert_eq!(git::get_current_branch(&master_repo)?, "feature-x");
    assert_eq!(git::get_current_branch(&main_repo)?, "develop");

    for result in &results {
        match &result.outcome {
            UpdateOutcome::Success(s) => {
                if result.path.ends_with("repo-on-master") {
                    assert_eq!(s.original_branch, "feature-x");
                    assert_eq!(s.master_branch, "master");
                } else if result.path.ends_with("repo-on-main") {
                    assert_eq!(s.original_branch, "develop");
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
    let workspace = TempDir::new()?;
    let repos = repo::find_git_repos(workspace.path());
    assert!(repos.is_empty());

    let results = repo::update_workspace(&repos, |_| NoOpCallbacks);
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
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(
        &workspace,
        &[("alpha", "master"), ("beta", "master"), ("gamma", "master")],
    )?;

    let repos = repo::find_git_repos(workspace.path());
    let expected_names: HashSet<&str> = ["alpha", "beta", "gamma"].into_iter().collect();

    for _ in 0..3 {
        let results = repo::update_workspace(&repos, |_| NoOpCallbacks);
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
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(&workspace, &[("repo-untracked", "master")])?;

    let repo_path = workspace.path().join("repo-untracked");
    std::fs::write(repo_path.join("untracked.txt"), "untracked content\n")?;
    assert!(git::has_uncommitted_changes(&repo_path)?);

    let repos = repo::find_git_repos(workspace.path());
    let results = repo::update_workspace(&repos, |_| NoOpCallbacks);

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
