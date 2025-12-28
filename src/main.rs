use git_daily_rust::{output, repo};

fn main() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    output::print_working_dir(&cwd);

    let sub_dirs = repo::find_git_repos(&cwd);
    if sub_dirs.is_empty() {
        println!("No git repos found in this directory");
    } else {
        output::print_workspace_start(sub_dirs.len());
    }

    Ok(())
}
