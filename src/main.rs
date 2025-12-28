#[allow(unused_imports)]
use git_daily_rust::{git, output, repo};

fn main() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    output::print_working_dir(&cwd);

    let is_git_repo = repo::is_git_repo(&cwd);
    if is_git_repo {
        println!("Git repo");
    } else {
        println!("Not a git repo");
    }

    Ok(())
}
