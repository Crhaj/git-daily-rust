mod common;

use common::TestRepo;
use git_daily_rust::git;

#[test]
fn test_repo_creation() -> anyhow::Result<()> {
    let repo = TestRepo::new()?;

    // Verify we're on master
    let branch = git::get_current_branch(repo.path())?;
    assert_eq!(branch, "master");

    Ok(())
}

#[test]
fn test_repo_with_remote() -> anyhow::Result<()> {
    let (repo, _remote) = TestRepo::with_remote()?;

    // Verify we're on master
    let branch = git::get_current_branch(repo.path())?;
    assert_eq!(branch, "master");

    // Verify fetch works (has remote configured)
    git::fetch_prune(repo.path())?;

    Ok(())
}
