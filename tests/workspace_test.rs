mod common;

use common::{CountingCallbacks, init_repo, setup_workspace_with_repos, test_config};
use git_daily_rust::config::Verbosity;
use git_daily_rust::git;
use git_daily_rust::output::NoOpCallbacks;
use git_daily_rust::repo::{self, UpdateCallbacks, UpdateOutcome, UpdateStep};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tempfile::TempDir;

#[test]
fn test_update_workspace_updates_multiple_repos() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(&workspace, &[("repo-a", "master"), ("repo-b", "master")])?;

    let repos = repo::find_git_repos(workspace.path());
    let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);

    assert_eq!(results.len(), 2);
    Ok(())
}

#[test]
fn test_workspace_mixed_success_and_failure() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(
        &workspace,
        &[("repo-ok", "master"), ("repo-fail", "master")],
    )?;

    let broken_path = workspace.path().join("repo-fail");
    git::run_git(
        &broken_path,
        &config,
        &["remote", "set-url", "origin", "/nope"],
    )?;

    let repos = repo::find_git_repos(workspace.path());
    let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);

    assert_eq!(results.len(), 2);
    assert!(
        results
            .iter()
            .any(|r| matches!(r.outcome, UpdateOutcome::Failed(_)))
    );
    assert!(
        results
            .iter()
            .any(|r| matches!(r.outcome, UpdateOutcome::Success(_)))
    );
    Ok(())
}

#[test]
fn test_workspace_with_dirty_repos() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(&workspace, &[("repo-dirty", "master")])?;

    let repo_path = workspace.path().join("repo-dirty");
    std::fs::write(repo_path.join("README.md"), "# Modified\n")?;
    assert!(repo::is_git_repo(&repo_path));

    let repos = repo::find_git_repos(workspace.path());
    let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);

    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn test_workspace_callbacks_called_for_each_repo() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(&workspace, &[("repo-a", "master"), ("repo-b", "master")])?;

    let repos = repo::find_git_repos(workspace.path());
    let (callbacks, step_count, complete_count) = CountingCallbacks::new();
    let results = repo::update_workspace(&repos, |_| callbacks.clone(), &config);

    assert_eq!(results.len(), 2);
    assert!(step_count.load(std::sync::atomic::Ordering::SeqCst) > 0);
    assert_eq!(complete_count.load(std::sync::atomic::Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn test_workspace_repos_on_different_branches() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(&workspace, &[("repo-a", "master"), ("repo-b", "main")])?;

    let master_repo = workspace.path().join("repo-a");
    repo::update(&master_repo, &NoOpCallbacks, &config);

    let main_repo = workspace.path().join("repo-b");
    repo::update(&main_repo, &NoOpCallbacks, &config);

    let repos = repo::find_git_repos(workspace.path());
    let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);

    assert_eq!(results.len(), 2);
    Ok(())
}

#[test]
fn test_workspace_empty_directory() -> anyhow::Result<()> {
    let workspace = TempDir::new()?;
    let repos = repo::find_git_repos(workspace.path());
    assert_eq!(repos.len(), 0);
    Ok(())
}

#[test]
fn test_workspace_nested_repos_not_discovered() -> anyhow::Result<()> {
    let workspace = TempDir::new()?;
    let outer_repo = workspace.path().join("outer-repo");
    let nested_repo = outer_repo.join("nested-repo");

    std::fs::create_dir_all(&outer_repo)?;
    std::fs::create_dir_all(&nested_repo)?;
    init_repo(&outer_repo, "master")?;
    init_repo(&nested_repo, "master")?;

    let repos = repo::find_git_repos(workspace.path());
    assert_eq!(repos.len(), 1);
    assert!(repos[0].ends_with("outer-repo"));
    Ok(())
}

#[test]
fn test_workspace_order_independence() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(
        &workspace,
        &[("alpha", "master"), ("beta", "master"), ("gamma", "master")],
    )?;

    let repos = repo::find_git_repos(workspace.path());
    let expected_names: HashSet<&str> = ["alpha", "beta", "gamma"].into_iter().collect();

    for _ in 0..3 {
        let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);
        assert_eq!(results.len(), 3);

        let result_names: HashSet<&str> = results
            .iter()
            .filter_map(|r| r.path.file_name())
            .filter_map(|n| n.to_str())
            .collect();

        assert_eq!(result_names, expected_names);
    }
    Ok(())
}

#[test]
fn test_workspace_with_untracked_files_only() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    setup_workspace_with_repos(&workspace, &[("repo-untracked", "master")])?;

    let repo_path = workspace.path().join("repo-untracked");
    std::fs::write(repo_path.join("untracked.txt"), "untracked content\n")?;
    assert!(repo::is_git_repo(&repo_path));

    let repos = repo::find_git_repos(workspace.path());
    let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);

    assert_eq!(results.len(), 1);
    match &results[0].outcome {
        UpdateOutcome::Success(s) => {
            assert!(!s.had_stash);
        }
        UpdateOutcome::Failed(f) => panic!("Expected success, got failure: {}", f.error),
    }

    assert!(repo_path.join("untracked.txt").exists());
    Ok(())
}

#[test]
fn test_workspace_ignores_non_git_subdirs() -> anyhow::Result<()> {
    let config = test_config();
    let workspace = TempDir::new()?;
    let non_repo_path = workspace.path().join("notes");

    std::fs::create_dir_all(&non_repo_path)?;
    setup_workspace_with_repos(&workspace, &[("repo-a", "master")])?;

    let repos = repo::find_git_repos(workspace.path());
    assert_eq!(repos.len(), 1);
    assert!(repos[0].ends_with("repo-a"));

    let results = repo::update_workspace(&repos, |_| NoOpCallbacks, &config);
    assert_eq!(results.len(), 1);
    assert!(matches!(results[0].outcome, UpdateOutcome::Success(_)));
    Ok(())
}

#[derive(Clone)]
struct ConcurrencyCallbacks {
    active: Arc<AtomicUsize>,
    saw_concurrent: Arc<AtomicBool>,
}

impl UpdateCallbacks for ConcurrencyCallbacks {
    fn on_step(&self, _step: &UpdateStep) {}

    fn on_complete(&self, _result: &repo::UpdateResult) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }

    fn on_update_start(&self, _repo_name: &str) {
        let previous = self.active.fetch_add(1, Ordering::SeqCst);
        if previous > 0 {
            self.saw_concurrent.store(true, Ordering::SeqCst);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[test]
fn test_workspace_verbose_runs_sequentially() -> anyhow::Result<()> {
    let mut config = test_config();
    config.verbosity = Verbosity::Verbose;

    let workspace = TempDir::new()?;
    setup_workspace_with_repos(&workspace, &[("repo-a", "master"), ("repo-b", "master")])?;

    let active = Arc::new(AtomicUsize::new(0));
    let saw_concurrent = Arc::new(AtomicBool::new(false));

    let repos = repo::find_git_repos(workspace.path());
    let results = repo::update_workspace(
        &repos,
        |_| ConcurrencyCallbacks {
            active: Arc::clone(&active),
            saw_concurrent: Arc::clone(&saw_concurrent),
        },
        &config,
    );

    assert_eq!(results.len(), 2);
    assert!(!saw_concurrent.load(Ordering::SeqCst));
    Ok(())
}
