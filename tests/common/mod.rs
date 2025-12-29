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
    /// Creates a new test repository with an initial commit on the master branch.
    pub fn new() -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let path = temp_dir.path().to_path_buf();

        run_git(&path, &["init", "-b", "master"])?;

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

    /// Creates a test repository with a configured remote.
    /// Returns the repo and the remote TempDir (must be kept alive).
    pub fn with_remote() -> Result<(Self, TempDir)> {
        let remote_dir = TempDir::new()?;
        run_git(remote_dir.path(), &["init", "--bare"])?;

        let local = Self::new()?;

        run_git(
            &local.path,
            &["remote", "add", "origin", remote_dir.path().to_str().unwrap()],
        )?;
        run_git(&local.path, &["push", "-u", "origin", "master"])?;

        Ok((local, remote_dir))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
