mod common;

use common::TestRepo;
use git_daily_rust::git;
use git_daily_rust::repo::{self, UpdateOutcome};

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

    // Still on master
    let branch = git::get_current_branch(repo.path())?;
    assert_eq!(branch, "master");

    // Can checkout the new branch
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
    // Untracked files also show up in has_uncommitted_changes
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
    // Ensure the remote is gone before fetch runs.
    drop(remote);

    let result = repo::update(repo.path(), |_| {});

    match result.outcome {
        UpdateOutcome::Failed(failure) => {
            assert!(matches!(failure.step, repo::UpdateStep::Fetching));
            assert!(failure.error.contains("fetch"));
        }
        UpdateOutcome::Success(_) => anyhow::bail!("expected update to fail without a remote"),
    }

    Ok(())
}
