//! Git command wrappers.
//!
//! This module provides a thin wrapper around git CLI commands,
//! handling command execution and error formatting.

use anyhow::Context;
use std::path::Path;

fn run_git(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = std::process::Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .context("Failed to execute git command")?;

    if output.status.success() {
        let result = String::from_utf8_lossy(&output.stdout);
        Ok(result.as_ref().trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {} failed: {}", args.join(" "), stderr)
    }
}

fn validate_branch_name(branch: &str) -> anyhow::Result<()> {
    if branch.contains('\0') || branch.contains('\n') || branch.is_empty() {
        anyhow::bail!("Invalid branch name: {:?}", branch);
    }
    Ok(())
}

pub fn get_current_branch(repo: &Path) -> anyhow::Result<String> {
    run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"]).context("Failed to get current branch")
}

pub fn has_uncommitted_changes(repo: &Path) -> anyhow::Result<bool> {
    run_git(repo, &["status", "--porcelain"])
        .map(|output| !output.is_empty())
        .context("Failed to check for uncommitted changes")
}

pub fn stash(repo: &Path) -> anyhow::Result<bool> {
    let output = run_git(repo, &["stash"]).context("Failed to stash changes")?;
    Ok(!output.contains("No local changes to save"))
}

pub fn stash_pop(repo: &Path) -> anyhow::Result<()> {
    run_git(repo, &["stash", "pop"]).context("Failed to pop stash")?;
    Ok(())
}

pub fn checkout(repo: &Path, branch: &str) -> anyhow::Result<()> {
    validate_branch_name(branch)?;
    run_git(repo, &["checkout", branch])
        .with_context(|| format!("Failed to checkout branch '{}'", branch))?;
    Ok(())
}

pub fn fetch_prune(repo: &Path) -> anyhow::Result<()> {
    run_git(repo, &["fetch", "--prune"]).context("Failed to fetch from remote")?;
    Ok(())
}
