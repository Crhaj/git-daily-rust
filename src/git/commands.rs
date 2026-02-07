//! Git command wrappers.
//!
//! High-level functions for common git operations like checkout, stash, and branch management.

use std::path::Path;

use anyhow::Context;

use crate::config::Config;

use super::runner::{run_git_output, run_git_with_logger};
use super::{validate_branch_name, validate_remote_ref, GitLogger};

#[must_use = "query function returns data that should be used"]
pub fn get_current_branch(
    repo: &Path,
    config: &Config,
    logger: GitLogger,
) -> anyhow::Result<String> {
    run_git_with_logger(repo, config, &["rev-parse", "--abbrev-ref", "HEAD"], logger)
        .context("Failed to get current branch")
}

#[must_use = "query function returns data that should be used"]
pub fn get_current_commit(
    repo: &Path,
    config: &Config,
    logger: GitLogger,
) -> anyhow::Result<String> {
    run_git_with_logger(repo, config, &["rev-parse", "HEAD"], logger)
        .context("Failed to get current commit")
}

/// Returns true if the remote tracking ref exists.
///
/// `remote_ref` must be in `<remote>/<branch>` form (for example, `origin/feature-x`),
/// not a full `refs/remotes/...` path.
#[must_use = "query function returns data that should be used"]
pub fn remote_ref_exists(
    repo: &Path,
    config: &Config,
    remote_ref: &str,
    logger: GitLogger,
) -> anyhow::Result<bool> {
    validate_remote_ref(remote_ref)?;
    let ref_path = format!("refs/remotes/{}", remote_ref);
    let output = run_git_output(
        repo,
        config,
        &["rev-parse", "--verify", ref_path.as_str()],
        logger,
    )?;
    Ok(output.status.success())
}

#[must_use = "query function returns data that should be used"]
pub fn has_uncommitted_changes(
    repo: &Path,
    config: &Config,
    logger: GitLogger,
) -> anyhow::Result<bool> {
    run_git_with_logger(repo, config, &["status", "--porcelain"], logger)
        .map(|output| !output.is_empty())
        .context("Failed to check for uncommitted changes")
}

pub fn stash(repo: &Path, config: &Config, logger: GitLogger) -> anyhow::Result<bool> {
    let output =
        run_git_with_logger(repo, config, &["stash"], logger).context("Failed to stash changes")?;
    Ok(!output.contains("No local changes to save"))
}

pub fn stash_pop(repo: &Path, config: &Config, logger: GitLogger) -> anyhow::Result<()> {
    run_git_with_logger(repo, config, &["stash", "pop"], logger).context("Failed to pop stash")?;
    Ok(())
}

pub fn checkout(
    repo: &Path,
    config: &Config,
    branch: &str,
    logger: GitLogger,
) -> anyhow::Result<()> {
    validate_branch_name(branch)?;
    run_git_with_logger(repo, config, &["checkout", branch], logger)
        .with_context(|| format!("Failed to checkout branch '{}'", branch))?;
    Ok(())
}

pub fn pull(repo: &Path, config: &Config, branch: &str, logger: GitLogger) -> anyhow::Result<()> {
    validate_branch_name(branch)?;
    run_git_with_logger(
        repo,
        config,
        &["pull", "--ff-only", "origin", branch],
        logger,
    )
    .with_context(|| format!("Failed to pull '{}' from origin", branch))?;
    Ok(())
}

/// Fetches from origin and prunes deleted remote branches.
///
/// This ensures local tracking refs are up-to-date before branch analysis.
pub fn fetch_prune(repo: &Path, config: &Config, logger: GitLogger) -> anyhow::Result<()> {
    run_git_with_logger(repo, config, &["fetch", "--prune"], logger)
        .context("Failed to fetch and prune")?;
    Ok(())
}

/// Lists local branches with their upstream tracking refs.
pub fn list_branches_with_upstream(
    repo: &Path,
    config: &Config,
    logger: GitLogger,
) -> anyhow::Result<String> {
    run_git_with_logger(
        repo,
        config,
        &[
            "for-each-ref",
            "--format=%(refname:short)|%(upstream:short)",
            "refs/heads/",
        ],
        logger,
    )
    .context("Failed to get branch names with upstream info")
}

/// Deletes a local branch safely (fails if not fully merged).
pub fn delete_branch(
    repo: &Path,
    config: &Config,
    name: &str,
    logger: GitLogger,
) -> anyhow::Result<()> {
    validate_branch_name(name)?;
    run_git_with_logger(repo, config, &["branch", "-d", name], logger)
        .with_context(|| format!("Failed to delete branch '{}'", name))?;
    Ok(())
}

/// Force deletes a local branch.
pub fn delete_branch_force(
    repo: &Path,
    config: &Config,
    name: &str,
    logger: GitLogger,
) -> anyhow::Result<()> {
    validate_branch_name(name)?;
    run_git_with_logger(repo, config, &["branch", "-D", name], logger)
        .with_context(|| format!("Failed to force delete branch '{}'", name))?;
    Ok(())
}

/// Lists local branches merged into the specified target branch.
pub fn list_merged_branches(
    repo: &Path,
    config: &Config,
    target: &str,
    logger: GitLogger,
) -> anyhow::Result<String> {
    validate_branch_name(target)?;
    run_git_with_logger(repo, config, &["branch", "--merged", target], logger)
        .with_context(|| format!("Failed to list branches merged into '{}'", target))
}

/// Returns the merge-base SHA between two refs.
pub fn merge_base(
    repo: &Path,
    config: &Config,
    ref1: &str,
    ref2: &str,
    logger: GitLogger,
) -> anyhow::Result<String> {
    run_git_with_logger(repo, config, &["merge-base", ref1, ref2], logger)
        .with_context(|| format!("Failed to run merge-base for '{}' and '{}'", ref1, ref2))
}

/// Checks if all commits in a branch have been applied to the target branch.
///
/// Uses `git cherry target branch` which compares commits by patch-id (content hash).
/// Returns true if ALL commits show `-` prefix (meaning they're in target).
/// Returns false if ANY commit shows `+` prefix (meaning it's NOT in target).
///
/// Note: This works for single-commit squash merges but NOT for multi-commit
/// squash merges (the combined patch differs from individual patches).
#[must_use = "query function returns data that should be used"]
pub fn is_branch_merged_by_cherry(
    repo: &Path,
    config: &Config,
    target: &str,
    branch: &str,
    logger: GitLogger,
) -> anyhow::Result<bool> {
    validate_branch_name(target)?;
    validate_branch_name(branch)?;
    let output = run_git_with_logger(repo, config, &["cherry", target, branch], logger)
        .with_context(|| format!("Failed to run git cherry {} {}", target, branch))?;

    // Empty output means no commits unique to branch (it's at the same point or behind)
    if output.trim().is_empty() {
        return Ok(true);
    }

    // All lines must start with '-' (commit is in target) for branch to be fully merged
    // Any '+' means there's a commit not in target
    Ok(output.lines().all(|line| line.starts_with('-')))
}

/// Gets files added by a branch since it diverged from another branch.
///
/// Returns a list of file paths that the branch introduced (not just modified).
/// This is used to check if a squash-merged branch's additions are in the target.
#[must_use = "query function returns data that should be used"]
pub fn get_files_added_by_branch(
    repo: &Path,
    config: &Config,
    target: &str,
    branch: &str,
    logger: GitLogger,
) -> anyhow::Result<Vec<String>> {
    validate_branch_name(target)?;
    validate_branch_name(branch)?;

    // Get merge-base first
    let merge_base = run_git_with_logger(repo, config, &["merge-base", target, branch], logger)
        .with_context(|| format!("Failed to find merge-base for {} and {}", target, branch))?;
    let merge_base = merge_base.trim();

    // Get files added by branch since merge-base
    let output = run_git_with_logger(
        repo,
        config,
        &["diff", "--name-only", "--diff-filter=A", merge_base, branch],
        logger,
    )
    .with_context(|| format!("Failed to get files added by {}", branch))?;

    Ok(output
        .lines()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

/// Checks if specific files exist in a branch.
pub fn files_exist_in_branch(
    repo: &Path,
    config: &Config,
    branch: &str,
    files: &[String],
    logger: GitLogger,
) -> anyhow::Result<bool> {
    validate_branch_name(branch)?;

    for file in files {
        // Use ls-tree to check if file exists in branch
        // ls-tree returns empty output for non-existent files, error for invalid refs
        let output = run_git_with_logger(
            repo,
            config,
            &["ls-tree", "--name-only", branch, "--", file],
            logger,
        )
        .with_context(|| format!("Failed to check if {} exists in {}", file, branch))?;

        if output.trim().is_empty() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Checks if a branch's modifications are present in the target branch.
///
/// For branches that only modify existing files (no additions), this checks
/// if those modifications have been incorporated into the target. Uses
/// `git diff target branch -- <modified files>` scoped to only the files
/// the branch touched.
///
/// Returns true if the branch has no unique content (modifications are in target).
pub fn branch_changes_in_target(
    repo: &Path,
    config: &Config,
    target: &str,
    branch: &str,
    logger: GitLogger,
) -> anyhow::Result<bool> {
    validate_branch_name(target)?;
    validate_branch_name(branch)?;

    // Get merge-base
    let merge_base = run_git_with_logger(repo, config, &["merge-base", target, branch], logger)
        .with_context(|| format!("Failed to find merge-base for {} and {}", target, branch))?;
    let merge_base = merge_base.trim();

    // Get files modified by branch since merge-base (not added, just modified)
    let modified_output = run_git_with_logger(
        repo,
        config,
        &["diff", "--name-only", "--diff-filter=M", merge_base, branch],
        logger,
    )
    .with_context(|| format!("Failed to get files modified by {}", branch))?;

    let modified_files: Vec<&str> = modified_output.lines().collect();

    // If no files were modified, branch only deleted files or is empty
    // Fall back to checking if there are any differences
    if modified_files.is_empty() {
        let diff = run_git_with_logger(repo, config, &["diff", target, branch], logger)
            .with_context(|| format!("Failed to diff {} {}", target, branch))?;
        return Ok(diff.trim().is_empty());
    }

    // Check if the branch's version of modified files matches target's version
    // Use git diff with specific file paths
    let mut args = vec!["diff", "--quiet", target, branch, "--"];
    args.extend(modified_files);

    // Use run_git_output directly to distinguish between:
    // - Exit 0: no differences
    // - Exit 1: differences exist (not an error)
    // - Other: actual errors (timeout, invalid ref)
    let output = run_git_output(repo, config, &args, logger)
        .with_context(|| format!("Failed to diff {} vs {} for modified files", target, branch))?;

    if output.status.success() {
        Ok(true) // No differences
    } else if output.status.code() == Some(1) {
        Ok(false) // Files differ (expected behavior for git diff --quiet)
    } else {
        // Unexpected exit code - treat as error
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "git diff --quiet failed with unexpected exit code {:?}: {}",
            output.status.code(),
            stderr
        )
    }
}

/// Returns numstat diff output showing line changes between two branches.
///
/// Uses `git diff --numstat from_branch to_branch` which outputs:
/// `<added>\t<removed>\t<filename>` for each changed file.
///
/// This is used to determine if a branch's content is fully contained in another:
/// - If diffing FROM feature TO main shows only additions (removed=0), feature is merged
/// - If there are any removals, feature has content not in main
///
/// Binary files show `-` instead of numbers.
///
/// Note: Empty files that exist in from_branch but not to_branch show as `0\t0\tfilename`.
/// Use [`has_deleted_files`] to catch this edge case.
pub fn diff_numstat(
    repo: &Path,
    config: &Config,
    from_branch: &str,
    to_branch: &str,
    logger: GitLogger,
) -> anyhow::Result<String> {
    validate_branch_name(from_branch)?;
    validate_branch_name(to_branch)?;
    run_git_with_logger(
        repo,
        config,
        &["diff", "--numstat", from_branch, to_branch],
        logger,
    )
    .with_context(|| format!("Failed to diff --numstat {} {}", from_branch, to_branch))
}

/// Checks if there are files that exist in from_branch but not in to_branch.
///
/// Uses `git diff --diff-filter=D --name-only from_branch to_branch` which lists
/// files that would be deleted when going from `from_branch` to `to_branch`.
///
/// This catches edge cases that numstat misses, such as empty files unique to the branch.
#[must_use = "query function returns data that should be used"]
pub fn has_deleted_files(
    repo: &Path,
    config: &Config,
    from_branch: &str,
    to_branch: &str,
    logger: GitLogger,
) -> anyhow::Result<bool> {
    validate_branch_name(from_branch)?;
    validate_branch_name(to_branch)?;
    let output = run_git_with_logger(
        repo,
        config,
        &[
            "diff",
            "--diff-filter=D",
            "--name-only",
            from_branch,
            to_branch,
        ],
        logger,
    )
    .with_context(|| {
        format!(
            "Failed to check deleted files {} {}",
            from_branch, to_branch
        )
    })?;
    Ok(!output.trim().is_empty())
}
