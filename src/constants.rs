//! Application-wide constants.
//!
//! Centralized configuration values to avoid magic numbers throughout the codebase.

use std::time::Duration;

/// Default timeout for individual git operations (in seconds).
const DEFAULT_GIT_TIMEOUT_SECS: u64 = 30;

/// Returns the git command timeout.
///
/// Can be customized via the GIT_DAILY_TIMEOUT environment variable (in seconds).
/// Falls back to 30 seconds if not set or invalid.
///
/// Example: `GIT_DAILY_TIMEOUT=60 git-daily-v2`
pub fn git_timeout() -> Duration {
    std::env::var("GIT_DAILY_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(DEFAULT_GIT_TIMEOUT_SECS))
}

/// Number of threads for parallel repository updates.
/// Higher than CPU count because git operations are I/O-bound (network, disk).
pub const RAYON_THREAD_COUNT: usize = 60;

/// Progress bar tick interval in milliseconds.
/// Controls how often the spinner/bar animates.
pub const PROGRESS_TICK_MS: u64 = 80;

/// Maximum number of completed repositories to show in the workspace progress display.
pub const MAX_VISIBLE_COMPLETIONS: usize = 5;

/// Default branch names to try when checking out the main branch.
pub const MASTER_BRANCH: &str = "master";
pub const MAIN_BRANCH: &str = "main";

/// Git directory name used to detect repositories.
pub const GIT_DIR: &str = ".git";

/// Default name used when a repository name cannot be determined from its path.
pub const DEFAULT_REPO_NAME: &str = "repository";
