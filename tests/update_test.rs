mod common;

use common::{TestRepo, test_config};
use git_daily_rust::git::{self, no_op_logger};
use git_daily_rust::output::NoOpCallbacks;
use git_daily_rust::repo::{self, OriginalHead, UpdateOutcome, UpdateStep};
use tempfile::TempDir;

/// Shorthand for the test logger (no-op for tests)
fn logger() -> git::GitLogger {
    no_op_logger
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
    assert!(git::has_uncommitted_changes(
        repo.path(),
        &config,
        logger()
    )?);

    let result = repo::update(repo.path(), &NoOpCallbacks, &config);

    assert!(matches!(result.outcome, UpdateOutcome::Success(_)));
    assert!(git::has_uncommitted_changes(
        repo.path(),
        &config,
        logger()
    )?);
    assert!(!repo.has_stash()?);
    Ok(())
}

#[test]
fn test_update_untracked_only_no_pop() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(None)?;
    repo.make_untracked()?;
    assert!(!repo.has_stash()?);
    assert!(git::has_uncommitted_changes(
        repo.path(),
        &config,
        logger()
    )?);

    let result = repo::update(repo.path(), &NoOpCallbacks, &config);

    assert!(matches!(result.outcome, UpdateOutcome::Success(_)));
    assert!(!repo.has_stash()?);
    assert!(git::has_uncommitted_changes(
        repo.path(),
        &config,
        logger()
    )?);
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
            assert_eq!(
                success.original_head,
                OriginalHead::Branch("feature".to_string())
            );
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

    let result1 = repo::update(repo.path(), &NoOpCallbacks, &config);
    assert!(matches!(result1.outcome, UpdateOutcome::Success(_)));

    let result2 = repo::update(repo.path(), &NoOpCallbacks, &config);
    assert!(matches!(result2.outcome, UpdateOutcome::Success(_)));

    assert_eq!(
        git::get_current_branch(repo.path(), &config, logger())?,
        "feature"
    );
    Ok(())
}

#[test]
fn test_update_handles_detached_head() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(None)?;

    let original_commit = git::get_current_commit(repo.path(), &config, logger())?;
    git::run_git(repo.path(), &config, &["checkout", "--detach", "HEAD"])?;

    let branch = git::get_current_branch(repo.path(), &config, logger())?;
    assert_eq!(branch, "HEAD", "Expected detached HEAD state");

    let result = repo::update(repo.path(), &NoOpCallbacks, &config);

    match &result.outcome {
        UpdateOutcome::Success(success) => {
            assert!(
                success.original_head.is_detached(),
                "Expected detached HEAD state"
            );
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

    let current_commit = git::get_current_commit(repo.path(), &config, logger())?;
    assert_eq!(current_commit, original_commit);

    Ok(())
}

#[test]
fn test_update_clean_repo_reports_no_stash() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(None)?;

    let result = repo::update(repo.path(), &NoOpCallbacks, &config);
    match result.outcome {
        UpdateOutcome::Success(success) => {
            assert!(!success.had_stash);
        }
        UpdateOutcome::Failed(failure) => anyhow::bail!("update failed: {}", failure.error),
    }
    Ok(())
}

#[test]
fn test_update_fails_when_no_master_or_main() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    let repo_path = workspace.path().join("dev-repo");
    let remote_path = workspace.path().join("dev-remote");

    std::fs::create_dir_all(&repo_path)?;
    std::fs::create_dir_all(&remote_path)?;
    git::run_git(&remote_path, &config, &["init", "--bare"])?;
    git::run_git(&repo_path, &config, &["init", "-b", "dev"])?;
    git::run_git(
        &repo_path,
        &config,
        &["config", "user.email", "test@example.com"],
    )?;
    git::run_git(&repo_path, &config, &["config", "user.name", "Test User"])?;
    std::fs::write(repo_path.join("README.md"), "# Test Repo\n")?;
    git::run_git(&repo_path, &config, &["add", "README.md"])?;
    git::run_git(&repo_path, &config, &["commit", "-m", "Initial commit"])?;
    git::run_git(
        &repo_path,
        &config,
        &["remote", "add", "origin", remote_path.to_str().unwrap()],
    )?;
    git::run_git(&repo_path, &config, &["push", "-u", "origin", "dev"])?;

    let result = repo::update(&repo_path, &NoOpCallbacks, &config);
    match result.outcome {
        UpdateOutcome::Failed(failure) => {
            assert_eq!(failure.step, UpdateStep::CheckingOut);
        }
        UpdateOutcome::Success(_) => anyhow::bail!("expected update to fail without master/main"),
    }
    Ok(())
}

#[test]
fn test_update_handles_empty_repo() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    let repo_path = workspace.path().join("empty-repo");

    std::fs::create_dir_all(&repo_path)?;
    git::run_git(&repo_path, &config, &["init", "-b", "master"])?;
    git::run_git(
        &repo_path,
        &config,
        &["config", "user.email", "test@example.com"],
    )?;
    git::run_git(&repo_path, &config, &["config", "user.name", "Test User"])?;

    let result = repo::update(&repo_path, &NoOpCallbacks, &config);
    match result.outcome {
        UpdateOutcome::Failed(failure) => {
            assert_eq!(failure.step, UpdateStep::DetectingBranch);
        }
        UpdateOutcome::Success(_) => anyhow::bail!("expected update to fail for empty repo"),
    }
    Ok(())
}
