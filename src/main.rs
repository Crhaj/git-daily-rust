#[allow(unused_imports)]
use git_daily_rust::{git, output, repo};

fn main() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    output::print_working_dir(&cwd);

    Ok(())
}
