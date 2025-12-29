//! Progress bars, colored output, and summary formatting.
//!
//! This module provides the UI layer for git-daily, including:
//! - Progress indicators for single and multi-repository updates
//! - Colored terminal output for results
//! - Summary formatting for completed operations

use crate::repo::{UpdateOutcome, UpdateResult, UpdateStep};
use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_VISIBLE_COMPLETIONS: usize = 5;

/// Wrapper for single repository progress updates.
pub struct SingleRepoProgress {
    spinner: ProgressBar,
}

impl SingleRepoProgress {
    pub fn update(&self, step: &UpdateStep) {
        let message = format_step_message(step);
        self.spinner.set_message(message);
    }

    pub fn finish_success(&self, repo_name: &str) {
        self.spinner.finish_with_message(format!(
            "{} {} updated successfully",
            "✓".green(),
            repo_name
        ));
    }

    pub fn finish_failed(&self, repo_name: &str, error: &str) {
        self.spinner.finish_with_message(format!(
            "{} {} failed: {}",
            "✗".red(),
            repo_name,
            error
        ));
    }
}

/// Thread-safe wrapper for workspace progress tracking.
#[derive(Clone)]
pub struct WorkspaceProgress {
    _multi: Arc<MultiProgress>,
    main_bar: ProgressBar,
    completion_slots: Vec<ProgressBar>,
    completed_repos: Arc<Mutex<VecDeque<(String, bool)>>>,
    failed_count: Arc<Mutex<usize>>,
    total_completed: Arc<Mutex<usize>>,
}

impl WorkspaceProgress {
    pub fn create_repo_tracker(&self, repo_name: String) -> RepoProgressTracker {
        RepoProgressTracker {
            repo_name,
            workspace: self.clone(),
        }
    }

    pub fn mark_completed(&self, repo_name: &str, success: bool) {
        self.main_bar.inc(1);

        if !success {
            let mut failed = self.failed_count.lock().unwrap();
            *failed += 1;
            self.main_bar
                .set_message(format!("│ {} failed", *failed).red().to_string());
        }

        {
            let mut completed = self.completed_repos.lock().unwrap();
            let mut total = self.total_completed.lock().unwrap();
            *total += 1;

            completed.push_back((repo_name.to_string(), success));

            while completed.len() > MAX_VISIBLE_COMPLETIONS {
                completed.pop_front();
            }

            self.redraw_completions(&completed, *total);
        }
    }

    pub fn finish(&self) {
        self.main_bar.finish_and_clear();
        for slot in &self.completion_slots {
            slot.finish_and_clear();
        }
    }

    fn redraw_completions(&self, completed: &VecDeque<(String, bool)>, total: usize) {
        let show_ellipsis = total > MAX_VISIBLE_COMPLETIONS;

        for (i, slot) in self.completion_slots.iter().enumerate() {
            if i == 0 && show_ellipsis {
                slot.set_message("...".dimmed().to_string());
            } else {
                let idx = if show_ellipsis { i - 1 } else { i };
                if idx < completed.len() {
                    let (name, success) = &completed[idx];
                    let symbol = if *success {
                        "✓".green()
                    } else {
                        "✗".red()
                    };
                    slot.set_message(format!("{} {}", symbol, name));
                } else {
                    slot.set_message("");
                }
            }
        }
    }
}

/// A repository-specific progress tracker that can be moved into rayon workers.
pub struct RepoProgressTracker {
    repo_name: String,
    workspace: WorkspaceProgress,
}

impl RepoProgressTracker {
    pub fn step_callback(&self) -> impl Fn(&UpdateStep) + '_ {
        move |_step: &UpdateStep| {}
    }

    pub fn mark_completed(&self, success: bool) {
        self.workspace.mark_completed(&self.repo_name, success);
    }
}

pub fn create_single_repo_progress() -> SingleRepoProgress {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    spinner.enable_steady_tick(Duration::from_millis(80));

    SingleRepoProgress { spinner }
}

pub fn create_workspace_progress(total: usize) -> WorkspaceProgress {
    let multi = Arc::new(MultiProgress::new());
    let main_bar = multi.add(ProgressBar::new(total as u64));

    main_bar.set_style(
        ProgressStyle::default_bar()
            .template("{bar:40.cyan/blue} {pos}/{len} completed {spinner:.cyan} {msg}")
            .unwrap()
            .progress_chars("█░"),
    );
    main_bar.enable_steady_tick(Duration::from_millis(80));

    let completion_slots: Vec<ProgressBar> = (0..MAX_VISIBLE_COMPLETIONS)
        .map(|_| {
            let slot = multi.add(ProgressBar::new_spinner());
            slot.set_style(
                ProgressStyle::default_spinner()
                    .template("  {msg}")
                    .unwrap(),
            );
            slot
        })
        .collect();

    WorkspaceProgress {
        _multi: multi,
        main_bar,
        completion_slots,
        completed_repos: Arc::new(Mutex::new(VecDeque::new())),
        failed_count: Arc::new(Mutex::new(0)),
        total_completed: Arc::new(Mutex::new(0)),
    }
}

pub fn print_working_dir(path: &Path) {
    println!(
        "{} {}",
        "Working in:".cyan(),
        path.display().to_string().white().bold()
    )
}

pub fn print_workspace_start(count: usize) {
    if count == 0 {
        print_no_repos()
    } else {
        println!(
            "{}",
            format!("Starting in workspace mode with {} repositories", count).dimmed()
        )
    }
}

pub fn print_summary(results: &[UpdateResult], duration: Duration) {
    print_section("Summary");
    let (successes, failures): (Vec<_>, Vec<_>) = results
        .iter()
        .partition(|r| matches!(r.outcome, UpdateOutcome::Success(_)));

    print_successes(&successes);
    print_failures(&failures);

    println!(
        "{}: {}/{} repos in {}",
        "Total".white().bold(),
        successes.len(),
        results.len(),
        format_duration(duration)
    );
}

fn print_no_repos() {
    println!("{}", "No git repositories found".yellow().bold())
}

fn format_duration(duration: Duration) -> String {
    format!("{:.2}s", duration.as_secs_f32())
}

fn print_section(title: &str) {
    let line = "=".repeat(50).cyan().dimmed();
    let padding = (50 - title.len()) / 2;
    let centered = format!("{:>width$}", title, width = padding + title.len());
    println!("\n{}\n{}\n{}\n", line, centered.cyan().bold(), line);
}

fn print_successes(successes: &[&UpdateResult]) {
    if successes.is_empty() {
        return;
    }
    println!(
        "{}",
        format!("Succeeded ({}):", successes.len()).green().bold()
    );

    for result in successes {
        if let UpdateOutcome::Success(success) = &result.outcome {
            let stash_msg = if success.had_stash {
                " (stash restored)".yellow()
            } else {
                "".normal()
            };
            println!(
                "  {} {} {} {} in {}",
                "OK".green().bold(),
                result.path.display().to_string().white(),
                format!("[{}]", success.original_branch).cyan(),
                stash_msg,
                format_duration(result.duration).dimmed(),
            );
        }
    }
    println!();
}

fn print_failures(failures: &[&UpdateResult]) {
    if failures.is_empty() {
        return;
    }

    println!("{}", format!("Failed ({}):", failures.len()).red().bold());

    for result in failures {
        if let UpdateOutcome::Failed(failure) = &result.outcome {
            println!(
                "  {} {} {} in {}",
                "FAIL".red().bold(),
                result.path.display().to_string().white(),
                format!("at {:?}: {}", failure.step, failure.error).red(),
                format_duration(result.duration).dimmed(),
            );
        }
    }
    println!();
}

fn format_step_message(step: &UpdateStep) -> String {
    match step {
        UpdateStep::Started => "Starting update...".to_string(),
        UpdateStep::DetectingBranch => "Detecting current branch...".to_string(),
        UpdateStep::CheckingChanges => "Checking for uncommitted changes...".to_string(),
        UpdateStep::Stashing => "Stashing uncommitted changes...".to_string(),
        UpdateStep::CheckingOut { branch } => format!("Checking out {}...", branch),
        UpdateStep::Fetching => "Fetching from origin...".to_string(),
        UpdateStep::RestoringBranch { branch } => format!("Restoring branch {}...", branch),
        UpdateStep::PoppingStash => "Restoring stashed changes...".to_string(),
        UpdateStep::Completed => "Completed".to_string(),
    }
}
