#[allow(unused_imports)]
use git_daily_rust::{git, output, repo};

fn main() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    output::print_working_dir(&cwd);

    if repo::is_git_repo(&cwd) {
        println!("Git repo");
    } else {
        println!("Not a git repo, checking subdirectories...\n");

        let sub_dirs = repo::find_git_repos(&cwd);
        output::print_workspace_start(sub_dirs.len());
    }

    println!("Current branch: {}", git::get_current_branch(&cwd)?);
    println!("Has uncommitted changes: {}", git::has_uncommitted_changes(&cwd)?);
    git::stash(&cwd)?;
    git::stash_pop(&cwd)?;
    git::fetch_prune(&cwd)?;

    println!("Done!");
    Ok(())
}
