// Repository detection, update logic, result types

use std::path::{Path, PathBuf};
use std::time::Duration;

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
pub enum UpdateOutcome {
    Success {
        original_branch: String,
        main_branch: String,
        had_stash: bool,
    },
    Failed {
        error: String,
        step: UpdateStep,
    },
}

pub fn is_git_repo(path: &Path) -> bool {
    path.join(".git").is_dir()
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

// TODO: update(path, on_step)
