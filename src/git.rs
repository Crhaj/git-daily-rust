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

// TODO: get_current_branch()
// TODO: has_uncommitted_changes()
// TODO: stash()
// TODO: stash_pop()
// TODO: checkout()
// TODO: fetch_prune()
