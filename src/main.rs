//! CLI entry point for git-daily-v2.

use clap::Parser;
use git_daily_rust::config::{Config, Verbosity};
use git_daily_rust::constants::{DEFAULT_REPO_NAME, RAYON_THREAD_COUNT};
use git_daily_rust::repo::UpdateOutcome;
use git_daily_rust::{output, repo};
use std::path::Path;

#[derive(Parser)]
#[command(name = "git-daily-v2")]
#[command(
    about = "Update master/main branches in git repositories. Useful to update everything in your workspace at once."
)]
#[command(version)]
#[command(
    after_help = "EXIT CODES:\n  0  All repositories updated successfully\n  1  Some repositories failed\n  2  All repositories failed"
)]
struct Args {
    /// Show git commands being executed (runs sequentially in workspace mode)
    #[arg(short, long)]
    verbose: bool,

    /// Minimal output (errors only). Ideal for CI/scripts
    #[arg(short, long, conflicts_with = "verbose")]
    quiet: bool,
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

    let start = std::time::Instant::now();
    let cwd = std::env::current_dir()?;

    output::print_working_dir(&cwd, &config);

    let results: Vec<_> = if repo::is_git_repo(&cwd) {
        run_single_repo(&cwd, &config)
    } else {
        run_workspace(&cwd, &config)
    };

    output::print_summary(&results, start.elapsed(), &config);

    std::process::exit(compute_exit_code(&results));
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
