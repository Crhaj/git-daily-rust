// Repository detection, update logic, result types

use std::path::{Path, PathBuf};

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
// TODO: UpdateStep enum
// TODO: UpdateResult struct
// TODO: UpdateOutcome enum
