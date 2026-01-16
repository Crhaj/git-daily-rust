//! Repository detection, update logic, and result types.
//!
//! This module provides the core update functionality for git repositories,
//! including detecting branches, stashing changes, and fetching updates.

use crate::config::Config;
use crate::constants::{DEFAULT_REPO_NAME, GIT_DIR, MAIN_BRANCH, MASTER_BRANCH};
use crate::git;
use rayon::prelude::*;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Callbacks for monitoring repository update progress and output.
///
/// Implement this trait to receive notifications during the update process.
/// Use [`output::NoOpCallbacks`] when progress tracking is not needed.
///
/// This trait decouples domain logic from presentation concerns, allowing
/// different output strategies (verbose, quiet, progress bars, etc.).
///
/// # Required Methods
///
/// - [`on_step`]: Called for each major step - implement for progress tracking
/// - [`on_complete`]: Called when update finishes - implement for result handling
///
/// # Optional Methods (have default no-op implementations)
///
/// - [`on_update_start`]: Called before update begins - use for repo-level setup
/// - [`on_step_execute`]: Called just before step executes - use for verbose logging
/// - [`on_completion_status`]: Called with final status - use for success/error messages
///
/// [`on_step`]: UpdateCallbacks::on_step
/// [`on_complete`]: UpdateCallbacks::on_complete
/// [`on_update_start`]: UpdateCallbacks::on_update_start
/// [`on_step_execute`]: UpdateCallbacks::on_step_execute
/// [`on_completion_status`]: UpdateCallbacks::on_completion_status
/// [`output::NoOpCallbacks`]: crate::output::NoOpCallbacks
pub trait UpdateCallbacks: Send + Sync {
    /// Called when a repository update begins.
    ///
    /// Optional - default implementation does nothing.
    fn on_update_start(&self, _repo_name: &str) {}

    /// Called when an update step begins (for progress tracking).
    ///
    /// Required - you must implement this method.
    fn on_step(&self, step: &UpdateStep);

    /// Called when an update step is about to execute (for verbose output).
    ///
    /// Optional - default implementation does nothing.
    fn on_step_execute(&self, _step: &UpdateStep) {}

    /// Called when the update completes (success or failure).
    ///
    /// Required - you must implement this method.
    fn on_complete(&self, result: &UpdateResult);

    /// Called with completion status for verbose output.
    ///
    /// Optional - default implementation does nothing.
    fn on_completion_status(&self, _success: bool, _error: Option<&str>) {}
}

/// Represents a step in the repository update process.
///
/// This enum is marked `#[non_exhaustive]` because new steps may be added
/// in future versions. When matching on `UpdateStep`, use a wildcard pattern
/// to handle unknown variants gracefully.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStep {
    Started,
    DetectingBranch,
    CheckingChanges,
    Fetching,
    Stashing,
    CheckingOut,
    Pulling,
    RestoringBranch,
    PoppingStash,
    Completed,
}

impl fmt::Display for UpdateStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            UpdateStep::Started => "Starting",
            UpdateStep::DetectingBranch => "Detecting branch",
            UpdateStep::CheckingChanges => "Checking changes",
            UpdateStep::Fetching => "Fetching",
            UpdateStep::Stashing => "Stashing",
            UpdateStep::CheckingOut => "Checking out",
            UpdateStep::Pulling => "Pulling",
            UpdateStep::RestoringBranch => "Restoring branch",
            UpdateStep::PoppingStash => "Popping stash",
            UpdateStep::Completed => "Completed",
        };
        write!(f, "{}", name)
    }
}

/// Result of a repository update operation.
#[derive(Debug, Clone)]
pub struct UpdateResult {
    pub path: PathBuf,
    pub outcome: UpdateOutcome,
    pub duration: Duration,
}

/// Outcome of an update: success or failure.
#[derive(Debug, Clone)]
pub enum UpdateOutcome {
    Success(UpdateSuccess),
    Failed(UpdateFailure),
}

/// The original state of HEAD before an update operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginalHead {
    /// HEAD was on a named branch (e.g., "feature-x").
    Branch(String),
    /// HEAD was detached at a specific commit SHA.
    DetachedAt(String),
}

impl OriginalHead {
    /// Returns the git reference to checkout (branch name or commit SHA).
    #[must_use]
    pub fn git_ref(&self) -> &str {
        match self {
            OriginalHead::Branch(name) => name,
            OriginalHead::DetachedAt(sha) => sha,
        }
    }

    /// Returns true if HEAD was detached.
    #[must_use]
    pub fn is_detached(&self) -> bool {
        matches!(self, OriginalHead::DetachedAt(_))
    }

    /// Returns a display-friendly representation for summaries.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            OriginalHead::Branch(name) => format!("[{}]", name),
            OriginalHead::DetachedAt(sha) => {
                let short = if sha.len() > 7 { &sha[..7] } else { sha };
                format!("[{}...detached]", short)
            }
        }
    }
}

/// Details of a successful update.
#[derive(Debug, Clone)]
pub struct UpdateSuccess {
    pub original_head: OriginalHead,
    pub master_branch: &'static str,
    pub had_stash: bool,
}

/// Details of a failed update.
#[derive(Debug, Clone)]
pub struct UpdateFailure {
    pub error: String,
    pub step: UpdateStep,
}

impl fmt::Display for UpdateFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed at {:?}: {}", self.step, self.error)
    }
}

struct UpdateError {
    source: anyhow::Error,
    step: UpdateStep,
}

/// Formats the full error chain from an anyhow error.
fn format_error_chain(error: &anyhow::Error) -> String {
    let mut chain: Vec<String> = vec![error.to_string()];
    let mut current = error.source();
    while let Some(cause) = current {
        chain.push(cause.to_string());
        current = cause.source();
    }
    chain.join(": ")
}

/// Returns true if the given path contains a `.git` directory.
#[must_use]
pub fn is_git_repo(path: &Path) -> bool {
    path.join(GIT_DIR).is_dir()
}

/// Finds all immediate child directories that are git repositories.
/// Does not search recursively into nested directories.
#[must_use]
pub fn find_git_repos(path: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir() && is_git_repo(&e.path()))
        .map(|e| e.path())
        .collect()
}

/// Updates a single repository with callbacks for progress and output.
pub fn update<C>(path: &Path, callbacks: &C, config: &Config) -> UpdateResult
where
    C: UpdateCallbacks,
{
    callbacks.on_step(&UpdateStep::Started);

    let repo_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(DEFAULT_REPO_NAME);
    callbacks.on_update_start(repo_name);

    let start = std::time::Instant::now();
    let result = do_update(path, callbacks, config);
    let duration = start.elapsed();

    callbacks.on_step(&UpdateStep::Completed);

    match result {
        Ok(success) => {
            callbacks.on_completion_status(true, None);
            UpdateResult {
                path: path.to_path_buf(),
                outcome: UpdateOutcome::Success(success),
                duration,
            }
        }
        Err(error) => {
            // Format full error chain for better debugging
            let error_chain = format_error_chain(&error.source);
            callbacks.on_completion_status(false, Some(&error_chain));
            UpdateResult {
                path: path.to_path_buf(),
                outcome: UpdateOutcome::Failed(UpdateFailure {
                    error: error_chain,
                    step: error.step,
                }),
                duration,
            }
        }
    }
}

/// Updates multiple repositories in parallel with per-repository callbacks.
/// In verbose mode, runs sequentially for readable output.
pub fn update_workspace<F, C>(
    repos: &[PathBuf],
    make_callbacks: F,
    config: &Config,
) -> Vec<UpdateResult>
where
    F: Fn(&Path) -> C + Sync,
    C: UpdateCallbacks,
{
    let process_repo = |path: &PathBuf| {
        let callbacks = make_callbacks(path);
        let result = update(path, &callbacks, config);
        callbacks.on_complete(&result);
        result
    };

    if config.is_verbose() {
        // Sequential for readable verbose output
        repos.iter().map(process_repo).collect()
    } else {
        // Parallel for performance
        repos.par_iter().map(process_repo).collect()
    }
}

fn run_step<T, C>(
    step: UpdateStep,
    path: &Path,
    callbacks: &C,
    operation: impl FnOnce() -> anyhow::Result<T>,
) -> Result<T, UpdateError>
where
    C: UpdateCallbacks,
{
    use anyhow::Context;
    callbacks.on_step(&step);
    callbacks.on_step_execute(&step);
    operation()
        .with_context(|| format!("in repository '{}'", path.display()))
        .map_err(|e| UpdateError { source: e, step })
}

/// Checks out the master branch, falling back to main if master doesn't exist.
fn checkout_master_or_main_branch<C>(
    path: &Path,
    callbacks: &C,
    config: &Config,
) -> Result<&'static str, UpdateError>
where
    C: UpdateCallbacks,
{
    let logger = config.git_logger();
    match run_step(UpdateStep::CheckingOut, path, callbacks, || {
        git::checkout(path, config, MASTER_BRANCH, logger)
    }) {
        Ok(_) => Ok(MASTER_BRANCH),
        Err(_) => {
            run_step(UpdateStep::CheckingOut, path, callbacks, || {
                git::checkout(path, config, MAIN_BRANCH, logger)
            })?;
            Ok(MAIN_BRANCH)
        }
    }
}

/// Core update logic: stash, checkout main, fetch, restore branch, pop stash.
fn do_update<C>(path: &Path, callbacks: &C, config: &Config) -> Result<UpdateSuccess, UpdateError>
where
    C: UpdateCallbacks,
{
    let logger = config.git_logger();

    let branch_name = run_step(UpdateStep::DetectingBranch, path, callbacks, || {
        git::get_current_branch(path, config, logger)
    })?;

    // Handle detached HEAD: store commit SHA instead of "HEAD"
    let original_head = if branch_name == "HEAD" {
        let commit = run_step(UpdateStep::DetectingBranch, path, callbacks, || {
            git::get_current_commit(path, config, logger)
        })?;
        OriginalHead::DetachedAt(commit)
    } else {
        OriginalHead::Branch(branch_name)
    };

    let is_dirty = run_step(UpdateStep::CheckingChanges, path, callbacks, || {
        git::has_uncommitted_changes(path, config, logger)
    })?;

    run_step(UpdateStep::Fetching, path, callbacks, || {
        git::fetch_prune(path, config, logger)
    })?;

    let had_stash = if is_dirty {
        run_step(UpdateStep::Stashing, path, callbacks, || {
            git::stash(path, config, logger)
        })?
    } else {
        false
    };

    let master_branch = checkout_master_or_main_branch(path, callbacks, config)?;

    run_step(UpdateStep::Pulling, path, callbacks, || {
        git::pull(path, config, master_branch, logger)
    })?;

    run_step(UpdateStep::RestoringBranch, path, callbacks, || {
        git::checkout(path, config, original_head.git_ref(), logger)
    })?;

    if had_stash {
        run_step(UpdateStep::PoppingStash, path, callbacks, || {
            git::stash_pop(path, config, logger)
        })?;
    }

    Ok(UpdateSuccess {
        original_head,
        master_branch,
        had_stash,
    })
}
