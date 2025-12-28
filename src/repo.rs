// Repository detection, update logic, result types

use std::path::Path;

pub fn is_git_repo(path: &Path) -> bool {
    path.join(".git").is_dir()
}

// TODO: update(path, on_step)
// TODO: UpdateStep enum
// TODO: UpdateResult struct
// TODO: UpdateOutcome enum
