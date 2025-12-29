//! Repository detection, update logic, and result types.
//!
//! This module provides the core update functionality for git repositories,
//! including detecting branches, stashing changes, and fetching updates.

use crate::git;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MASTER_BRANCH: &str = "master";
const MAIN_BRANCH: &str = "main";
const GIT_DIR: &str = ".git";

#[derive(Debug, Clone)]
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

    if is_dirty {
        run_step(UpdateStep::Stashing, on_step, || git::stash(path))?;
    }
    let master_or_main_branch = checkout_master_or_main_branch(path, on_step)?;

    run_step(UpdateStep::Fetching, on_step, || git::fetch_prune(path))?;
    run_step(
        UpdateStep::RestoringBranch {
            branch: original_branch.clone(),
        },
        on_step,
        || git::checkout(path, &original_branch),
    )?;

    if is_dirty {
        run_step(UpdateStep::PoppingStash, on_step, || git::stash_pop(path))?;
    }

    Ok(UpdateSuccess {
        original_branch,
        master_branch: master_or_main_branch.to_string(),
        had_stash: is_dirty,
    })
}
