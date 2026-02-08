//! Branch cleanup domain logic.
//!
//! Core types and functions for analyzing and deleting stale local branches.
//! Supports detection of both regular merges and squash merges.
//!
//! # Architecture
//!
//! This module is organized into submodules:
//! - `types`: Core data structures (`BranchInfo`, `MergeStatus`, `CleanupResult`, etc.)
//! - `operations`: Git operations for branch analysis and deletion
//! - `interactive`: Interactive orchestration via [`run_interactive`]
//!
//! The interactive flow is decoupled from terminal I/O via the [`Prompter`] trait,
//! enabling full testability with [`MockPrompter`].
//!
//! [`Prompter`]: crate::prompt::Prompter
//! [`MockPrompter`]: crate::prompt::MockPrompter

mod interactive;
mod operations;
mod types;

// Re-export types for public API
pub use types::{
    BranchInfo, CleanupCallbacks, CleanupResult, DeletionMode, DeletionOutcome, DeletionResult,
    InteractiveResult, MergeStatus, NoOpCleanupCallbacks, TrackingStatus,
};

// Re-export operations for public API
pub use operations::{
    check_tracking_status, delete_branches, delete_single_branch, detect_main_branch,
    is_detached_head, list_branches,
};

// Re-export interactive flow
pub use interactive::run_interactive;
