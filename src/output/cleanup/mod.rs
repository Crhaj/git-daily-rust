//! Branch cleanup output - listing branches and reporting deletion results.
//!
//! # Module Structure
//!
//! - `formatting` - Column width calculation and branch line formatting
//! - `printing` - Terminal output for branch lists, results, and summaries

mod formatting;
mod printing;

pub use formatting::{BranchListWidths, format_branch_line, format_branch_selection_item};
pub use printing::{
    print_analyzing_branches, print_branch_list, print_branch_list_header, print_cleanup_summary,
    print_deleting_header, print_deletion_result, print_no_branches_to_clean,
};

use colored::Colorize;

use crate::cleanup::{BranchInfo, CleanupCallbacks, DeletionResult};
use crate::config::Config;

/// Terminal-based cleanup callbacks.
///
/// Implements [`CleanupCallbacks`] with colored terminal output.
/// Respects quiet mode by suppressing non-error output.
#[derive(Debug, Clone, Copy)]
pub struct TerminalCleanupCallbacks {
    config: Config,
}

impl TerminalCleanupCallbacks {
    /// Creates a new terminal callbacks instance.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl CleanupCallbacks for TerminalCleanupCallbacks {
    fn on_fetching(&self) {
        if !self.config.is_quiet() {
            eprintln!("Fetching latest changes...");
        }
    }

    fn on_analyzing(&self) {
        print_analyzing_branches(&self.config);
    }

    fn on_detached_head(&self) {
        if !self.config.is_quiet() {
            eprintln!(
                "{}: Repository is in detached HEAD state. All branches can be selected.",
                "Note".cyan()
            );
        }
    }

    fn on_no_branches(&self) {
        print_no_branches_to_clean(&self.config);
    }

    fn on_current_branch_selected(&self, branch_name: &str) {
        if !self.config.is_quiet() {
            eprintln!(
                "\nYou selected '{}', which is your current branch.",
                branch_name
            );
        }
    }

    fn on_switched_branch(&self, to_branch: &str) {
        if !self.config.is_quiet() {
            eprintln!("Switched to '{}'.", to_branch);
        }
    }

    fn on_unclear_warning(&self) {
        if !self.config.is_quiet() {
            eprintln!(
                "\n{}: Some branches have 'unclear' status (could not verify merge status).",
                "Note".yellow()
            );
        }
    }

    fn on_deleting(&self) {
        print_deleting_header(&self.config);
    }

    fn on_deletion_result(&self, result: &DeletionResult) {
        print_deletion_result(result, &self.config);
    }

    fn on_cancelled(&self) {
        if !self.config.is_quiet() {
            eprintln!("Cancelled.");
        }
    }

    fn on_dry_run(&self, branches: &[&BranchInfo]) {
        if !self.config.is_quiet() {
            eprintln!("\n{}", "Dry run - no branches deleted.".yellow());
            for branch in branches {
                eprintln!("  Would delete: {}", branch.name);
            }
        }
    }
}
