use std::path::Path;

fn run_git(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = std::process::Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()?;

    if output.status.success() {
        let result = String::from_utf8_lossy(&output.stdout);
        Ok(result.as_ref().trim().to_string())
    } else {
        anyhow::bail!("git failed: {}", String::from_utf8_lossy(&output.stderr))
    }
}

pub fn get_current_branch(repo: &Path) -> anyhow::Result<String> {
    run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
}

pub fn has_uncommitted_changes(repo: &Path) -> anyhow::Result<bool> {
    run_git(repo, &["status", "--porcelain"]).map(|output| !output.is_empty())
}

pub fn stash(repo: &Path) -> anyhow::Result<()> {
    run_git(repo, &["stash"])?;
    Ok(())
}

pub fn stash_pop(repo: &Path) -> anyhow::Result<()> {
    run_git(repo, &["stash", "pop"])?;
    Ok(())
}

pub fn checkout(repo: &Path, branch: &str) -> anyhow::Result<()> {
    run_git(repo, &["checkout", branch])?;
    Ok(())
}

pub fn fetch_prune(repo: &Path) -> anyhow::Result<()> {
    run_git(repo, &["fetch", "--prune"])?;
    Ok(())
}

