// Repository detection, update logic, result types

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
struct UpdateError {
    source: anyhow::Error,
    step: UpdateStep,
}

#[derive(Debug)]
struct UpdateSuccess {
    original_branch: String,
    master_branch: String,
    had_stash: bool,
}

#[derive(Debug)]
struct UpdateFailure {
    error: String,
    step: UpdateStep,
}

#[derive(Debug)]
pub enum UpdateOutcome {
    Success(UpdateSuccess),
    Failed(UpdateFailure),
}

fn is_git_repo(path: &Path) -> bool {
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

fn at_step<T>(step: UpdateStep, result: anyhow::Result<T>) -> Result<T, UpdateError> {
    result.map_err(|e| UpdateError { source: e, step })
}

pub fn update<F>(path: &Path, on_step: F) -> UpdateResult
where
    F: Fn(UpdateStep),
{
    UpdateResult {
        path: path.to_path_buf(),
        outcome: UpdateOutcome::Success(do_update(path, &on_step).unwrap()),
        duration: Duration::from_secs(0),
    }
}

fn do_update<F>(path: &Path, on_step: &F) -> Result<UpdateSuccess, UpdateError>
where
    F: Fn(UpdateStep),
{
    on_step(UpdateStep::Started);

    on_step(UpdateStep::DetectingBranch);
    let original_branch = at_step(UpdateStep::DetectingBranch, git::get_current_branch(path))?;

    on_step(UpdateStep::CheckingChanges);
    let is_dirty = at_step(
        UpdateStep::CheckingChanges,
        git::has_uncommitted_changes(path),
    )?;

    if is_dirty {
        on_step(UpdateStep::Stashing);
        at_step(UpdateStep::Stashing, git::stash(path))?;
    }

    on_step(UpdateStep::CheckingOut {
        branch: MASTER_BRANCH.to_string(),
    });
    let master_or_main_branch = match git::checkout(path, MASTER_BRANCH) {
        Ok(_) => MASTER_BRANCH,
        Err(_) => {
            on_step(UpdateStep::CheckingOut {
                branch: MAIN_BRANCH.to_string(),
            });
            at_step(
                UpdateStep::CheckingOut {
                    branch: MAIN_BRANCH.to_string(),
                },
                git::checkout(path, MAIN_BRANCH),
            )?;
            MAIN_BRANCH
        }
    };

    on_step(UpdateStep::Fetching);
    at_step(UpdateStep::Fetching, git::fetch_prune(path))?;

    on_step(UpdateStep::RestoringBranch {
        branch: original_branch.clone(),
    });
    at_step(
        UpdateStep::RestoringBranch {
            branch: original_branch.clone(),
        },
        git::checkout(path, &original_branch),
    )?;

    if is_dirty {
        on_step(UpdateStep::PoppingStash);
        at_step(UpdateStep::PoppingStash, git::stash_pop(path))?;
    }

    on_step(UpdateStep::Completed);

    Ok(UpdateSuccess {
        original_branch,
        master_branch: master_or_main_branch.to_string(),
        had_stash: is_dirty,
    })
}
