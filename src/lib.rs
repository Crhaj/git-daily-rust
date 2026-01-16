//! Git repository updater library.
//!
//! This crate provides functionality to update git repositories by:
//! - Detecting the current branch
//! - Stashing uncommitted changes
//! - Checking out main/master
//! - Pulling updates (fast-forward only)
//! - Restoring the original branch and stash
//!
//! # Usage
//!
//! ## Update a single repository
//!
//! ```no_run
//! use git_daily_rust::{repo, output, config::Config};
//! use std::path::Path;
//!
//! let config = Config::default();
//! let result = repo::update(Path::new("/path/to/repo"), &output::NoOpCallbacks, &config);
//!
//! match result.outcome {
//!     repo::UpdateOutcome::Success(s) => println!("Updated from {}", s.original_head.display()),
//!     repo::UpdateOutcome::Failed(f) => eprintln!("Failed: {}", f),
//! }
//! ```
//!
//! ## Update multiple repositories in parallel
//!
//! ```no_run
//! use git_daily_rust::{repo, output, config::Config};
//! use std::path::PathBuf;
//!
//! let config = Config::default();
//! let repos: Vec<PathBuf> = repo::find_git_repos(std::path::Path::new("/workspace"));
//!
//! let results = repo::update_workspace(&repos, |_path| output::NoOpCallbacks, &config);
//! let succeeded = results.iter().filter(|r| matches!(r.outcome, repo::UpdateOutcome::Success(_))).count();
//! println!("{}/{} repositories updated", succeeded, results.len());
//! ```

pub mod config;
pub mod constants;
pub mod git;
pub mod output;
pub mod repo;
pub mod cleanup;
pub mod prompt;
