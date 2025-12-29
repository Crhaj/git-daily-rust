//! Test infrastructure for git-daily-rust integration tests.

use anyhow::Result;
use git_daily_rust::git::run_git;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A temporary git repository for testing.
/// Automatically cleaned up when dropped.
pub struct TestRepo {
    _temp_dir: TempDir,
    path: PathBuf,
}

impl TestRepo {
    fn new_with_branch(branch: &str) -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let path = temp_dir.path().to_path_buf();

        run_git(&path, &["init", "-b", branch])?;

        run_git(&path, &["config", "user.email", "test@example.com"])?;
        run_git(&path, &["config", "user.name", "Test User"])?;

        std::fs::write(path.join("README.md"), "# Test Repo\n")?;
        run_git(&path, &["add", "README.md"])?;
        run_git(&path, &["commit", "-m", "Initial commit"])?;

        Ok(Self {
            _temp_dir: temp_dir,
            path,
        })
    }

    /// Creates a new test repository with an initial commit on the master branch.
    pub fn new() -> Result<Self> {
        Self::new_with_branch("master")
    }

    /// Creates a test repository with a configured remote.
    /// Returns the repo and the remote TempDir (must be kept alive).
    pub fn with_remote(branch: Option<&str>) -> Result<(Self, TempDir)> {
        let branch = branch.unwrap_or("master");
        let remote_dir = TempDir::new()?;
        run_git(remote_dir.path(), &["init", "--bare"])?;

        let local = Self::new_with_branch(branch)?;

        run_git(
            &local.path,
            &["remote", "add", "origin", remote_dir.path().to_str().unwrap()],
        )?;
        run_git(&local.path, &["push", "-u", "origin", branch])?;

        Ok((local, remote_dir))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Creates a new branch without checking it out.
    pub fn create_branch(&self, name: &str) -> Result<()> {
        run_git(&self.path, &["branch", name])?;
        Ok(())
    }

    /// Creates a modified tracked file (uncommitted changes).
    pub fn make_dirty(&self) -> Result<()> {
        std::fs::write(self.path.join("README.md"), "# Modified\n")?;
        Ok(())
    }

    /// Creates an untracked file.
    pub fn make_untracked(&self) -> Result<()> {
        std::fs::write(self.path.join("untracked.txt"), "untracked content\n")?;
        Ok(())
    }

    /// Returns true if there's an active stash.
    pub fn has_stash(&self) -> Result<bool> {
        let output = run_git(&self.path, &["stash", "list"])?;
        Ok(!output.is_empty())
    }

    /// Returns true if a file exists in the working directory.
    pub fn file_exists(&self, name: &str) -> bool {
        self.path.join(name).exists()
    }
}
