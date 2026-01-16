//! Test infrastructure for git-daily-rust integration tests.
//!
//! Provides test fixtures and utilities for creating temporary git repositories.

use anyhow::Result;
use git_daily_rust::config::Config;
use git_daily_rust::git::run_git;
use git_daily_rust::repo::{UpdateCallbacks, UpdateResult, UpdateStep};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

/// Default config for tests (normal verbosity, no special options).
pub fn test_config() -> Config {
    Config::default()
}

/// A temporary git repository for testing.
/// Automatically cleaned up when dropped.
pub struct TestRepo {
    // These fields must be kept alive to prevent temp directory cleanup
    #[allow(dead_code)]
    temp_dir: TempDir,
    #[allow(dead_code)]
    remote_dir: Option<TempDir>,
    path: PathBuf,
}

impl TestRepo {
    fn new_with_branch(branch: &str) -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let path = temp_dir.path().to_path_buf();
        init_repo(&path, branch)?;
        Ok(Self {
            temp_dir,
            remote_dir: None,
            path,
        })
    }

    /// Creates a new test repository with an initial commit on master.
    pub fn new() -> Result<Self> {
        Self::new_with_branch("master")
    }

    /// Creates a test repository with a configured remote.
    pub fn with_remote(branch: Option<&str>) -> Result<Self> {
        let branch = branch.unwrap_or("master");
        let config = test_config();
        let remote_dir = TempDir::new()?;
        run_git(remote_dir.path(), &config, &["init", "--bare"])?;

        let temp_dir = TempDir::new()?;
        let path = temp_dir.path().to_path_buf();
        init_repo(&path, branch)?;

        run_git(
            &path,
            &config,
            &[
                "remote",
                "add",
                "origin",
                remote_dir.path().to_str().unwrap(),
            ],
        )?;
        run_git(&path, &config, &["push", "-u", "origin", branch])?;

        Ok(Self {
            temp_dir,
            remote_dir: Some(remote_dir),
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Creates a new branch without checking it out.
    pub fn create_branch(&self, name: &str) -> Result<()> {
        run_git(&self.path, &test_config(), &["branch", name])?;
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
        let output = run_git(&self.path, &test_config(), &["stash", "list"])?;
        Ok(!output.is_empty())
    }

    /// Returns true if a file exists in the working directory.
    pub fn file_exists(&self, name: &str) -> bool {
        self.path.join(name).exists()
    }

    /// Removes the remote directory, simulating a broken remote.
    /// Used for testing failure scenarios.
    pub fn remove_remote(&mut self) {
        self.remote_dir = None;
    }
}

/// Callbacks that count invocations for testing.
#[derive(Clone)]
pub struct CountingCallbacks {
    step_count: Arc<AtomicUsize>,
    complete_count: Arc<AtomicUsize>,
}

impl CountingCallbacks {
    pub fn new() -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let step_count = Arc::new(AtomicUsize::new(0));
        let complete_count = Arc::new(AtomicUsize::new(0));
        (
            Self {
                step_count: Arc::clone(&step_count),
                complete_count: Arc::clone(&complete_count),
            },
            step_count,
            complete_count,
        )
    }
}

impl UpdateCallbacks for CountingCallbacks {
    fn on_step(&self, _step: &UpdateStep) {
        self.step_count.fetch_add(1, Ordering::SeqCst);
    }

    fn on_complete(&self, _result: &UpdateResult) {
        self.complete_count.fetch_add(1, Ordering::SeqCst);
    }
}

/// Initializes a git repository at the given path with an initial commit.
pub fn init_repo(path: &Path, branch: &str) -> Result<()> {
    let config = test_config();
    run_git(path, &config, &["init", "-b", branch])?;
    run_git(path, &config, &["config", "user.email", "test@example.com"])?;
    run_git(path, &config, &["config", "user.name", "Test User"])?;
    std::fs::write(path.join("README.md"), "# Test Repo\n")?;
    run_git(path, &config, &["add", "README.md"])?;
    run_git(path, &config, &["commit", "-m", "Initial commit"])?;
    Ok(())
}

/// Sets up a workspace with multiple repos and their remotes.
pub fn setup_workspace_with_repos(
    workspace: &TempDir,
    repo_configs: &[(&str, &str)],
) -> Result<()> {
    let config = test_config();
    for (name, branch) in repo_configs {
        let repo_path = workspace.path().join(name);
        let remote_path = workspace.path().join(format!("{}-remote", name));

        std::fs::create_dir_all(&repo_path)?;
        std::fs::create_dir_all(&remote_path)?;

        init_repo(&repo_path, branch)?;
        run_git(&remote_path, &config, &["init", "--bare"])?;
        run_git(
            &repo_path,
            &config,
            &["remote", "add", "origin", remote_path.to_str().unwrap()],
        )?;
        run_git(&repo_path, &config, &["push", "-u", "origin", branch])?;
    }
    Ok(())
}
