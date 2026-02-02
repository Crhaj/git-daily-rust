//! Progress bars, colored output, and summary formatting.
//!
//! This module provides visual feedback during repository updates and cleanups,
//! including spinners, progress bars, and colored summary output.

mod cleanup;
mod summary;
mod update;

// Re-export cleanup output functions and callbacks
pub use cleanup::{
    BranchListWidths, TerminalCleanupCallbacks, format_branch_line, print_analyzing_branches,
    print_branch_list, print_branch_list_header, print_cleanup_summary, print_deleting_header,
    print_deletion_result, print_no_branches_to_clean,
};

// Re-export summary functions
pub use summary::print_summary;

// Re-export update progress tracking
pub use update::{
    NoOpCallbacks, RepoProgressTracker, SingleRepoCallbacks, SingleRepoProgress, WorkspaceProgress,
    create_single_repo_progress, create_workspace_progress, print_completion_status,
    print_repo_header, print_step, print_working_dir, print_workspace_start,
};

use std::time::Duration;

/// Formats a duration as seconds with two decimal places.
///
/// Used by both update summary and cleanup summary formatting.
pub(crate) fn format_duration(duration: Duration) -> String {
    format!("{:.2}s", duration.as_secs_f32())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(1)), "1.00s");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1.50s");
        assert_eq!(format_duration(Duration::from_millis(123)), "0.12s");
    }
}
