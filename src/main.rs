//! CLI entry point for git-daily-v2.

use clap::{Parser, Subcommand};
use git_daily_rust::cleanup;
use git_daily_rust::config::{Config, Verbosity};
use git_daily_rust::constants::{DEFAULT_REPO_NAME, RAYON_THREAD_COUNT};
use git_daily_rust::prompt::TerminalPrompter;
use git_daily_rust::repo::UpdateOutcome;
use git_daily_rust::{output, repo};
use std::path::Path;

#[derive(Parser)]
#[command(name = "git-daily-v2")]
#[command(about = "Keep git repositories up to date and clean up stale branches.")]
#[command(version)]
#[command(after_help = "EXIT CODES:\n  0  Success\n  1  Partial failure\n  2  Complete failure")]
struct Args {
    /// Show git commands being executed
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Minimal output (errors only)
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Update master/main branches (default when no subcommand given)
    Update,

    /// Interactively clean up stale local branches
    Cleanup {
        /// Show what would be deleted without making changes
        #[arg(long)]
        dry_run: bool,
    },
}

impl Args {
    fn to_config(&self) -> Config {
        let verbosity = if self.quiet {
            Verbosity::Quiet
        } else if self.verbose {
            Verbosity::Verbose
        } else {
            Verbosity::Normal
        };
        Config { verbosity }
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = args.to_config();

    // High thread count is fine for I/O-bound git operations
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(RAYON_THREAD_COUNT)
        .build_global();

    let cwd = std::env::current_dir()?;

    match args.command {
        None | Some(Command::Update) => run_update(&cwd, &config),
        Some(Command::Cleanup { dry_run }) => run_cleanup(&cwd, dry_run, &config),
    }
}

fn run_update(cwd: &Path, config: &Config) -> anyhow::Result<()> {
    let start = std::time::Instant::now();

    output::print_working_dir(cwd, config);

    let results: Vec<_> = if repo::is_git_repo(cwd) {
        run_single_repo(cwd, config)
    } else {
        run_workspace(cwd, config)
    };

    output::print_summary(&results, start.elapsed(), config);

    std::process::exit(compute_exit_code(&results));
}

/// Runs the interactive branch cleanup flow.
///
/// This function is purely wiring - it creates the necessary dependencies
/// and delegates to the domain layer's `cleanup::run_interactive`.
fn run_cleanup(cwd: &Path, dry_run: bool, config: &Config) -> anyhow::Result<()> {
    if !repo::is_git_repo(cwd) {
        anyhow::bail!("Not a git repository. Cleanup only works inside a git repo.");
    }

    let logger = config.git_logger();
    let prompter = TerminalPrompter;
    let callbacks = output::TerminalCleanupCallbacks::new(*config);

    // Delegate to domain layer - all business logic lives there
    let result = cleanup::run_interactive(cwd, dry_run, &prompter, &callbacks, config, logger)?;

    // Print summary if we have results (not cancelled, not dry-run)
    if let Some(interactive_result) = result
        && !interactive_result.dry_run
    {
        output::print_cleanup_summary(
            &interactive_result.result,
            &interactive_result.remaining,
            config,
        );
    }

    Ok(())
}

fn run_single_repo(path: &Path, config: &Config) -> Vec<repo::UpdateResult> {
    let progress = output::create_single_repo_progress(config);
    let callbacks = output::SingleRepoCallbacks::new(progress, *config);
    let result = repo::update(path, &callbacks, config);
    callbacks.finish(&result);

    vec![result]
}

fn run_workspace(path: &Path, config: &Config) -> Vec<repo::UpdateResult> {
    let sub_dirs = repo::find_git_repos(path);
    output::print_workspace_start(sub_dirs.len(), config);

    if sub_dirs.is_empty() {
        return vec![];
    }

    let workspace_progress = output::create_workspace_progress(sub_dirs.len(), config);
    let results = repo::update_workspace(
        &sub_dirs,
        |dir| workspace_progress.create_repo_tracker(get_repo_name(dir), *config),
        config,
    );

    workspace_progress.finish();
    results
}

fn compute_exit_code(results: &[repo::UpdateResult]) -> i32 {
    if results.is_empty() {
        return 0;
    }

    let failure_count = results
        .iter()
        .filter(|r| matches!(r.outcome, UpdateOutcome::Failed(_)))
        .count();

    if failure_count == results.len() {
        2 // All failed
    } else if failure_count > 0 {
        1 // Partial failure
    } else {
        0 // All success
    }
}

fn get_repo_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(DEFAULT_REPO_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_daily_rust::repo::{UpdateFailure, UpdateResult, UpdateSuccess};
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn test_args_to_config_respects_quiet_and_verbose() {
        let quiet = Args::parse_from(["git-daily-v2", "--quiet"]);
        assert!(quiet.to_config().is_quiet());

        let verbose = Args::parse_from(["git-daily-v2", "--verbose"]);
        assert!(verbose.to_config().is_verbose());

        let normal = Args::parse_from(["git-daily-v2"]);
        assert!(!normal.to_config().is_quiet());
        assert!(!normal.to_config().is_verbose());
    }

    #[test]
    fn test_args_rejects_conflicting_flags() {
        let result = Args::try_parse_from(["git-daily-v2", "--quiet", "--verbose"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_args_rejects_unknown_flag() {
        let result = Args::try_parse_from(["git-daily-v2", "--nope"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_exit_code_all_success() {
        let results = vec![UpdateResult {
            path: PathBuf::from("/repo"),
            outcome: UpdateOutcome::Success(UpdateSuccess {
                original_head: repo::OriginalHead::Branch("main".to_string()),
                master_branch: "main",
                had_stash: false,
            }),
            duration: Duration::from_secs(1),
        }];
        assert_eq!(compute_exit_code(&results), 0);
    }

    #[test]
    fn test_compute_exit_code_partial_failure() {
        let success = UpdateResult {
            path: PathBuf::from("/repo-success"),
            outcome: UpdateOutcome::Success(UpdateSuccess {
                original_head: repo::OriginalHead::Branch("main".to_string()),
                master_branch: "main",
                had_stash: false,
            }),
            duration: Duration::from_secs(1),
        };
        let failure = UpdateResult {
            path: PathBuf::from("/repo-fail"),
            outcome: UpdateOutcome::Failed(UpdateFailure {
                error: "boom".to_string(),
                step: repo::UpdateStep::Fetching,
            }),
            duration: Duration::from_secs(1),
        };
        assert_eq!(compute_exit_code(&[success, failure]), 1);
    }

    #[test]
    fn test_compute_exit_code_all_failed() {
        let failure = UpdateResult {
            path: PathBuf::from("/repo-fail"),
            outcome: UpdateOutcome::Failed(UpdateFailure {
                error: "boom".to_string(),
                step: repo::UpdateStep::Fetching,
            }),
            duration: Duration::from_secs(1),
        };
        assert_eq!(compute_exit_code(&[failure]), 2);
    }

    #[test]
    fn test_compute_exit_code_empty() {
        assert_eq!(compute_exit_code(&[]), 0);
    }

    #[test]
    fn test_get_repo_name_falls_back_to_default() {
        let name = get_repo_name(Path::new("/"));
        assert_eq!(name, DEFAULT_REPO_NAME);
    }

    #[test]
    fn test_get_repo_name_uses_last_component() {
        let name = get_repo_name(Path::new("/tmp/my-repo"));
        assert_eq!(name, "my-repo");
    }
}
