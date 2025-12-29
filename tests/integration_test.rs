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
    let (repo, _remote) = TestRepo::with_remote()?;

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
    let (repo, _remote) = TestRepo::with_remote()?;

    // Create and checkout a feature branch
    repo.create_branch("feature")?;
    git::checkout(repo.path(), "feature")?;

    // Run update
    let result = repo::update(repo.path(), |_| {});

    // Verify success
    assert!(matches!(result.outcome, UpdateOutcome::Success(_)));

    // Verify we're back on feature branch
    let branch = git::get_current_branch(repo.path())?;
    assert_eq!(branch, "feature");

    Ok(())
}
