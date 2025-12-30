//! CLI entry point for git-daily-v2.

use git_daily_rust::repo::UpdateOutcome;
use git_daily_rust::{output, repo};
use std::path::Path;

fn main() -> anyhow::Result<()> {
    // High thread count is fine for I/O-bound git operations
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(100)
        .build_global();

    let start = std::time::Instant::now();

    let cwd = std::env::current_dir()?;
    output::print_working_dir(&cwd);

    let results: Vec<_> = if repo::is_git_repo(&cwd) {
        // Single repository mode - use spinner with step updates
        let progress = output::create_single_repo_progress();
        let result = repo::update(&cwd, |step| {
            progress.update(step);
        });
        match &result.outcome {
            UpdateOutcome::Success(_) => {
                progress.finish_success(get_repo_name(&cwd));
            }
            UpdateOutcome::Failed(failure) => {
                progress.finish_failed(get_repo_name(&cwd), &failure.error);
            }
        }

        vec![result]
    } else {
        // Workspace mode - use progress bar with parallel execution
        let sub_dirs = repo::find_git_repos(&cwd);
        output::print_workspace_start(sub_dirs.len());

        if sub_dirs.is_empty() {
            vec![]
        } else {
            let workspace_progress = output::create_workspace_progress(sub_dirs.len());
            let results = repo::update_workspace(&sub_dirs, |dir| {
                workspace_progress.create_repo_tracker(get_repo_name(dir))
            });

            workspace_progress.finish();
            results
        }
    };

    output::print_summary(&results, start.elapsed());

    if results
        .iter()
        .any(|r| matches!(r.outcome, UpdateOutcome::Failed(_)))
    {
        std::process::exit(1);
    }

    Ok(())
}

fn get_repo_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repository")
}
