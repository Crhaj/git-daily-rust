//! Repository detection, update logic, and result types.
//!
//! This module provides the core update functionality for git repositories,
//! including detecting branches, stashing changes, and fetching updates.

use crate::git;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MASTER_BRANCH: &str = "master";
const MAIN_BRANCH: &str = "main";
const GIT_DIR: &str = ".git";

/// Represents a step in the repository update process.
///
/// Each variant represents a distinct phase of the update operation.
/// Callbacks receive these to track progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStep {
    Started,
    DetectingBranch,
    CheckingChanges,
    Stashing,
    CheckingOut { branch: String },
    Fetching,
    RestoringBranch { branch: String },
    PoppingStash,
    Completed,
}

#[derive(Debug)]
pub struct UpdateResult {
    pub path: PathBuf,
    pub outcome: UpdateOutcome,
    pub duration: Duration,
}

/// Callbacks for monitoring repository update progress.
///
/// Implement this trait to receive notifications during the update process.
/// For simple cases, use [`NoOpCallbacks`] or implement the trait on your own type.
///
/// # Example
///
/// ```ignore
/// use git_daily_rust::repo::{UpdateCallbacks, UpdateStep, UpdateResult};
///
/// struct MyCallbacks;
///
/// impl UpdateCallbacks for MyCallbacks {
///     fn on_step(&self, step: &UpdateStep) {
///         println!("Step: {:?}", step);
///     }
///     fn on_complete(&self, result: &UpdateResult) {
///         println!("Done: {:?}", result.path);
///     }
/// }
/// ```
pub trait UpdateCallbacks: Send + Sync {
    /// Called when an update step begins.
    fn on_step(&self, step: &UpdateStep);

    /// Called when the update completes (success or failure).
    fn on_complete(&self, result: &UpdateResult);
}

/// Default no-op callbacks for when progress tracking is not needed.
///
/// This is a zero-cost abstraction - no heap allocations or dynamic dispatch
/// when used directly (monomorphized).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOpCallbacks;

impl UpdateCallbacks for NoOpCallbacks {
    #[inline]
    fn on_step(&self, _step: &UpdateStep) {}

    #[inline]
    fn on_complete(&self, _result: &UpdateResult) {}
}

/// Blanket implementation for closure tuples.
///
/// Allows using a tuple of two closures as callbacks:
///
/// ```ignore
/// let callbacks = (
///     |step: &UpdateStep| println!("{:?}", step),
///     |result: &UpdateResult| println!("done"),
/// );
/// ```
impl<F1, F2> UpdateCallbacks for (F1, F2)
where
    F1: Fn(&UpdateStep) + Send + Sync,
    F2: Fn(&UpdateResult) + Send + Sync,
{
    fn on_step(&self, step: &UpdateStep) {
        (self.0)(step);
    }
    fn on_complete(&self, result: &UpdateResult) {
        (self.1)(result);
    }
}

#[derive(Debug)]
pub struct UpdateSuccess {
    pub original_branch: String,
    pub master_branch: String,
    pub had_stash: bool,
}

#[derive(Debug)]
pub struct UpdateFailure {
    pub error: String,
    pub step: UpdateStep,
}

#[derive(Debug)]
pub enum UpdateOutcome {
    Success(UpdateSuccess),
    Failed(UpdateFailure),
}

#[derive(Debug)]
struct UpdateError {
    source: anyhow::Error,
    step: UpdateStep,
}

pub fn is_git_repo(path: &Path) -> bool {
    path.join(GIT_DIR).is_dir()
}

pub fn find_git_repos(path: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir() && is_git_repo(&e.path()))
        .map(|e| e.path())
        .collect()
}

pub fn update<F>(path: &Path, on_step: F) -> UpdateResult
where
    F: Fn(&UpdateStep),
{
    on_step(&UpdateStep::Started);

    let start = std::time::Instant::now();
    let result = do_update(path, &on_step);
    let duration = start.elapsed();

    on_step(&UpdateStep::Completed);

    match result {
        Ok(success) => UpdateResult {
            path: path.to_path_buf(),
            outcome: UpdateOutcome::Success(success),
            duration,
        },
        Err(error) => UpdateResult {
            path: path.to_path_buf(),
            outcome: UpdateOutcome::Failed(UpdateFailure {
                error: error.source.to_string(),
                step: error.step,
            }),
            duration,
        },
    }
}

/// Updates multiple repositories in parallel with per-repository callbacks.
///
/// This function iterates over the provided repositories in parallel using rayon,
/// calling `make_callbacks` to create a callbacks instance for each repository.
/// This allows per-repo customization of progress tracking.
///
/// # Arguments
///
/// * `repos` - Slice of repository paths to update
/// * `make_callbacks` - Factory function that creates callbacks for each repository
///
/// # Example
///
/// ```ignore
/// use git_daily_rust::repo::{update_workspace, NoOpCallbacks};
///
/// let results = update_workspace(&repos, |_path| NoOpCallbacks);
/// ```
pub fn update_workspace<F, C>(repos: &[PathBuf], make_callbacks: F) -> Vec<UpdateResult>
where
    F: Fn(&Path) -> C + Sync,
    C: UpdateCallbacks,
{
    repos
        .par_iter()
        .map(|path| {
            let callbacks = make_callbacks(path);
            let result = update(path, |step| callbacks.on_step(step));
            callbacks.on_complete(&result);
            result
        })
        .collect()
}

/// Updates multiple repositories in parallel with shared callbacks.
///
/// A convenience wrapper around [`update_workspace`] when the same callbacks
/// instance can be cloned and shared across all repositories.
///
/// # Example
///
/// ```ignore
/// use git_daily_rust::repo::{update_workspace_with, NoOpCallbacks};
///
/// let results = update_workspace_with(&repos, NoOpCallbacks);
/// ```
pub fn update_workspace_with<C>(repos: &[PathBuf], callbacks: C) -> Vec<UpdateResult>
where
    C: UpdateCallbacks + Clone,
{
    update_workspace(repos, |_| callbacks.clone())
}

fn run_step<T, F>(
    step: UpdateStep,
    on_progress: &F,
    operation: impl FnOnce() -> anyhow::Result<T>,
) -> Result<T, UpdateError>
where
    F: Fn(&UpdateStep),
{
    on_progress(&step);
    operation().map_err(|e| UpdateError { source: e, step })
}

fn checkout_master_or_main_branch<F>(path: &Path, on_step: &F) -> Result<&'static str, UpdateError>
where
    F: Fn(&UpdateStep),
{
    match run_step(
        UpdateStep::CheckingOut {
            branch: MASTER_BRANCH.to_string(),
        },
        on_step,
        || git::checkout(path, MASTER_BRANCH),
    ) {
        Ok(_) => Ok(MASTER_BRANCH),
        Err(_) => {
            run_step(
                UpdateStep::CheckingOut {
                    branch: MAIN_BRANCH.to_string(),
                },
                on_step,
                || git::checkout(path, MAIN_BRANCH),
            )?;
            Ok(MAIN_BRANCH)
        }
    }
}

fn do_update<F>(path: &Path, on_step: &F) -> Result<UpdateSuccess, UpdateError>
where
    F: Fn(&UpdateStep),
{
    let original_branch = run_step(UpdateStep::DetectingBranch, on_step, || {
        git::get_current_branch(path)
    })?;

    let is_dirty = run_step(UpdateStep::CheckingChanges, on_step, || {
        git::has_uncommitted_changes(path)
    })?;

    let had_stash = if is_dirty {
        run_step(UpdateStep::Stashing, on_step, || git::stash(path))?
    } else {
        false
    };
    let master_or_main_branch = checkout_master_or_main_branch(path, on_step)?;

    run_step(UpdateStep::Fetching, on_step, || git::fetch_prune(path))?;
    run_step(
        UpdateStep::RestoringBranch {
            branch: original_branch.clone(),
        },
        on_step,
        || git::checkout(path, &original_branch),
    )?;

    if had_stash {
        run_step(UpdateStep::PoppingStash, on_step, || git::stash_pop(path))?;
    }

    Ok(UpdateSuccess {
        original_branch,
        master_branch: master_or_main_branch.to_string(),
        had_stash,
    })
}
