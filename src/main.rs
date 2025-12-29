use git_daily_rust::repo::UpdateOutcome;
use git_daily_rust::{output, repo};

fn main() -> anyhow::Result<()> {
    let start = std::time::Instant::now();

    let cwd = std::env::current_dir()?;
    output::print_working_dir(&cwd);

    let results: Vec<_> = if repo::is_git_repo(&cwd) {
        vec![repo::update(&cwd, |_| {})]
    } else {
        let sub_dirs = repo::find_git_repos(&cwd);
        output::print_workspace_start(sub_dirs.len());
        sub_dirs
            .into_iter()
            .map(|dir| repo::update(&dir, |_| {}))
            .collect()
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
