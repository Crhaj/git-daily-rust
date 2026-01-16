mod common;

use common::{TestRepo, test_config};
use git_daily_rust::git::{self, no_op_logger};
use std::path::PathBuf;
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
    assert!(!git::has_uncommitted_changes(
        repo.path(),
        &config,
        logger()
    )?);
    repo.make_dirty()?;
    assert!(git::has_uncommitted_changes(
        repo.path(),
        &config,
        logger()
    )?);
    Ok(())
}

#[test]
fn test_make_untracked() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;
    assert!(!repo.file_exists("untracked.txt"));
    repo.make_untracked()?;
    assert!(repo.file_exists("untracked.txt"));
    assert!(git::has_uncommitted_changes(
        repo.path(),
        &config,
        logger()
    )?);
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
fn test_list_branches_with_upstream_includes_tracking() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(None)?;
    repo.create_branch("feature")?;

    let output = git::list_branches_with_upstream(repo.path(), &config, logger())?;

    assert!(output.lines().any(|line| line == "master|origin/master"));
    assert!(output.lines().any(|line| line == "feature|"));
    Ok(())
}

#[test]
fn test_remote_ref_exists_for_origin_master() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(None)?;

    assert!(git::remote_ref_exists(
        repo.path(),
        &config,
        "origin/master",
        logger()
    )?);
    Ok(())
}

#[test]
fn test_delete_branch_removes_branch() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;
    repo.create_branch("feature")?;

    git::delete_branch(repo.path(), &config, "feature", logger())?;
    let output = git::run_git(repo.path(), &config, &["branch", "--list", "feature"])?;
    assert!(output.trim().is_empty());
    Ok(())
}

#[test]
fn test_merge_base_and_merge_tree() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;
    let base_commit = git::get_current_commit(repo.path(), &config, logger())?;

    git::run_git(repo.path(), &config, &["checkout", "-b", "feature"])?;
    std::fs::write(repo.path().join("README.md"), "# Feature\n")?;
    git::run_git(repo.path(), &config, &["add", "README.md"])?;
    git::run_git(repo.path(), &config, &["commit", "-m", "Feature commit"])?;

    let merge_base = git::merge_base(repo.path(), &config, "master", "feature", logger())?;
    assert_eq!(merge_base, base_commit);

    let output = git::merge_tree(
        repo.path(),
        &config,
        &merge_base,
        "master",
        "feature",
        logger(),
    )?;
    assert!(!output.trim().is_empty());
    Ok(())
}

#[test]
fn test_list_merged_branches_includes_feature() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;

    git::run_git(repo.path(), &config, &["checkout", "-b", "feature"])?;
    std::fs::write(repo.path().join("README.md"), "# Feature\n")?;
    git::run_git(repo.path(), &config, &["add", "README.md"])?;
    git::run_git(repo.path(), &config, &["commit", "-m", "Feature commit"])?;

    git::run_git(repo.path(), &config, &["checkout", "master"])?;
    git::run_git(repo.path(), &config, &["merge", "feature"])?;

    let output = git::list_merged_branches(repo.path(), &config, "master", logger())?;
    let merged_names: Vec<&str> = output
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim())
        .collect();
    assert!(merged_names.contains(&"feature"));
    Ok(())
}

#[test]
fn test_remote_ref_exists_returns_false_for_missing_ref() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::with_remote(None)?;

    assert!(!git::remote_ref_exists(
        repo.path(),
        &config,
        "origin/does-not-exist",
        logger()
    )?);
    Ok(())
}

#[test]
fn test_remote_ref_exists_with_non_origin_remote() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;
    let upstream = TempDir::new()?;

    git::run_git(upstream.path(), &config, &["init", "--bare"])?;
    git::run_git(
        repo.path(),
        &config,
        &[
            "remote",
            "add",
            "upstream",
            upstream.path().to_str().unwrap(),
        ],
    )?;
    git::run_git(repo.path(), &config, &["push", "-u", "upstream", "master"])?;

    assert!(git::remote_ref_exists(
        repo.path(),
        &config,
        "upstream/master",
        logger()
    )?);
    Ok(())
}

#[test]
fn test_delete_branch_force_removes_unmerged_branch() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;

    git::run_git(repo.path(), &config, &["checkout", "-b", "feature"])?;
    std::fs::write(repo.path().join("README.md"), "# Feature\n")?;
    git::run_git(repo.path(), &config, &["add", "README.md"])?;
    git::run_git(repo.path(), &config, &["commit", "-m", "Feature commit"])?;
    git::run_git(repo.path(), &config, &["checkout", "master"])?;

    git::delete_branch_force(repo.path(), &config, "feature", logger())?;
    let output = git::run_git(repo.path(), &config, &["branch", "--list", "feature"])?;
    assert!(output.trim().is_empty());
    Ok(())
}

#[test]
fn test_list_branches_with_upstream_without_remote() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;
    repo.create_branch("feature")?;

    let output = git::list_branches_with_upstream(repo.path(), &config, logger())?;

    assert!(output.lines().any(|line| line == "master|"));
    assert!(output.lines().any(|line| line == "feature|"));
    Ok(())
}

#[test]
fn test_remote_ref_exists_rejects_invalid_remote_ref() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;

    let result = git::remote_ref_exists(repo.path(), &config, "-bad/branch", logger());
    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_merge_tree_errors_on_missing_ref() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;
    let base =
        git::get_current_commit(repo.path(), &config, logger()).expect("failed to get base commit");

    let result = git::merge_tree(
        repo.path(),
        &config,
        &base,
        "master",
        "does-not-exist",
        logger(),
    );
    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_merge_base_errors_on_invalid_ref_name() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;

    let result = git::merge_base(repo.path(), &config, "master", "bad;name", logger());
    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_run_git_reports_failure_for_unknown_ref() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;

    let result = git::run_git(repo.path(), &config, &["rev-parse", "does-not-exist"]);
    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_run_git_reports_spawn_failure_for_missing_repo_path() {
    let config = test_config();
    let missing_path = PathBuf::from("/no/such/repo/for/test");

    let result = git::run_git(&missing_path, &config, &["status"]);
    assert!(result.is_err());
    let message = result.unwrap_err().to_string();
    assert!(message.contains("Failed to spawn git command"));
}

#[test]
fn test_list_merged_branches_rejects_invalid_target() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;

    let result = git::list_merged_branches(repo.path(), &config, "bad;name", logger());
    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_merge_base_errors_on_missing_ref() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;

    let result = git::merge_base(repo.path(), &config, "master", "does-not-exist", logger());
    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_merge_tree_rejects_invalid_branch_name() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;
    let base =
        git::get_current_commit(repo.path(), &config, logger()).expect("failed to get base commit");

    let result = git::merge_tree(repo.path(), &config, &base, "master", "bad;name", logger());
    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_delete_branch_fails_on_unmerged_branch() -> anyhow::Result<()> {
    let config = test_config();
    let repo = TestRepo::new()?;

    git::run_git(repo.path(), &config, &["checkout", "-b", "feature"])?;
    std::fs::write(repo.path().join("README.md"), "# Feature\n")?;
    git::run_git(repo.path(), &config, &["add", "README.md"])?;
    git::run_git(repo.path(), &config, &["commit", "-m", "Feature commit"])?;
    git::run_git(repo.path(), &config, &["checkout", "master"])?;

    let result = git::delete_branch(repo.path(), &config, "feature", logger());
    assert!(result.is_err());
    Ok(())
}
