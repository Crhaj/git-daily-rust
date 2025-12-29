//! Git repository updater library.
//!
//! This crate provides functionality to update git repositories by:
//! - Detecting the current branch
//! - Stashing uncommitted changes
//! - Checking out main/master
//! - Fetching updates with prune
//! - Restoring the original branch and stash

pub mod git;
pub mod output;
pub mod repo;
