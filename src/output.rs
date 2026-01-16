//! Progress bars, colored output, and summary formatting.
//!
//! This module provides visual feedback during repository updates including
//! spinners, progress bars, and colored summary output.

use crate::config::Config;
use crate::constants::{DEFAULT_REPO_NAME, MAX_VISIBLE_COMPLETIONS, PROGRESS_TICK_MS};
use crate::repo::{UpdateCallbacks, UpdateOutcome, UpdateResult, UpdateStep};
use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// No-op callbacks for when progress tracking is not needed.
/// This is the null object pattern for UpdateCallbacks - use it when
/// you don't need any output or progress tracking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoOpCallbacks;

impl UpdateCallbacks for NoOpCallbacks {
    fn on_step(&self, _step: &UpdateStep) {}
    fn on_complete(&self, _result: &UpdateResult) {}
}

/// Prints a repository header in verbose mode.
pub fn print_repo_header(config: &Config, repo_name: &str) {
    if !config.is_verbose() {
        return;
    }
    eprintln!("\n{}", format!("[{}]", repo_name).white().bold());
}

/// Prints a step progress message in verbose mode.
pub fn print_step(config: &Config, step: &UpdateStep) {
    if !config.is_verbose() {
        return;
    }
    eprintln!("  {}...", step.to_string().dimmed());
}

/// Prints completion status (verbose mode only).
pub fn print_completion_status(config: &Config, success: bool, error: Option<&str>) {
    if !config.is_verbose() {
        return;
    }
    if success {
        eprintln!("  {} completed successfully", "✓".green());
    } else if let Some(err) = error {
        eprintln!("  {} failed: {}", "✗".red(), err);
    }
}

/// Progress wrapper for single repository updates.
/// Displays a spinner with step-by-step status messages.
/// Uses `Option` to avoid allocation when progress is hidden (quiet/verbose modes).
pub struct SingleRepoProgress {
    spinner: Option<ProgressBar>,
}

impl SingleRepoProgress {
    pub fn update(&self, step: &UpdateStep) {
        if let Some(spinner) = &self.spinner {
            let message = format_step_message(step);
            spinner.set_message(message);
        }
    }

    pub fn finish_success(&self, repo_name: &str) {
        if let Some(spinner) = &self.spinner {
            spinner.finish_with_message(format!(
                "{} {} updated successfully",
                "✓".green(),
                repo_name
            ));
        }
    }

    pub fn finish_failed(&self, repo_name: &str, error: &str) {
        if let Some(spinner) = &self.spinner {
            spinner.finish_with_message(format!("{} {} failed: {}", "✗".red(), repo_name, error));
        }
    }
}

/// Callbacks for single repository updates.
/// Combines progress bar updates with verbose output handling.
pub struct SingleRepoCallbacks {
    progress: SingleRepoProgress,
    config: Config,
}

impl SingleRepoCallbacks {
    pub fn new(progress: SingleRepoProgress, config: Config) -> Self {
        Self { progress, config }
    }

    /// Finish the progress bar with success/failure message.
    pub fn finish(&self, result: &UpdateResult) {
        let repo_name = result
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(DEFAULT_REPO_NAME);

        match &result.outcome {
            UpdateOutcome::Success(_) => {
                self.progress.finish_success(repo_name);
            }
            UpdateOutcome::Failed(failure) => {
                self.progress.finish_failed(repo_name, &failure.error);
            }
        }
    }
}

impl UpdateCallbacks for SingleRepoCallbacks {
    fn on_update_start(&self, repo_name: &str) {
        print_repo_header(&self.config, repo_name);
    }

    fn on_step(&self, step: &UpdateStep) {
        self.progress.update(step);
    }

    fn on_step_execute(&self, step: &UpdateStep) {
        print_step(&self.config, step);
    }

    fn on_complete(&self, _result: &UpdateResult) {
        // Completion is handled by main.rs using the result
    }

    fn on_completion_status(&self, success: bool, error: Option<&str>) {
        print_completion_status(&self.config, success, error);
    }
}

/// Consolidated state for workspace progress tracking.
/// Combining these fields reduces lock contention by acquiring a single lock
/// instead of multiple separate locks for related data.
struct CompletionState {
    /// Recently completed repos for display (bounded by MAX_VISIBLE_COMPLETIONS)
    repos: VecDeque<(String, bool)>,
    /// Count of failed repos for status message
    failed_count: usize,
    /// Total completed for determining ellipsis display
    total_completed: usize,
}

/// Thread-safe progress tracker for workspace mode.
/// Shows a progress bar with the completion count and recent results.
#[derive(Clone)]
pub struct WorkspaceProgress {
    _multi: Arc<MultiProgress>,
    main_bar: ProgressBar,
    completion_slots: Vec<ProgressBar>,
    state: Arc<Mutex<CompletionState>>,
}

impl WorkspaceProgress {
    pub fn create_repo_tracker(&self, repo_name: &str, config: Config) -> RepoProgressTracker {
        RepoProgressTracker {
            repo_name: repo_name.to_string(),
            workspace: self.clone(),
            config,
        }
    }

    pub fn mark_completed(&self, repo_name: &str, success: bool) {
        self.main_bar.inc(1);

        let mut state = self
            .state
            .lock()
            .expect("WorkspaceProgress state mutex poisoned");

        if !success {
            state.failed_count += 1;
            self.main_bar
                .set_message(format!("│ {} failed", state.failed_count).red().to_string());
        }

        state.total_completed += 1;
        state.repos.push_back((repo_name.to_string(), success));

        while state.repos.len() > MAX_VISIBLE_COMPLETIONS {
            state.repos.pop_front();
        }

        self.redraw_completions(&state);
    }

    pub fn finish(&self) {
        self.main_bar.finish_and_clear();
        for slot in &self.completion_slots {
            slot.finish_and_clear();
        }
    }

    fn redraw_completions(&self, state: &CompletionState) {
        let show_ellipsis = state.total_completed > MAX_VISIBLE_COMPLETIONS;

        for (i, slot) in self.completion_slots.iter().enumerate() {
            if i == 0 && show_ellipsis {
                slot.set_message("...".dimmed().to_string());
            } else {
                let idx = if show_ellipsis { i - 1 } else { i };
                if idx < state.repos.len() {
                    let (name, success) = &state.repos[idx];
                    let symbol = if *success { "✓".green() } else { "✗".red() };
                    slot.set_message(format!("{} {}", symbol, name));
                } else {
                    slot.set_message("");
                }
            }
        }
    }
}

/// Per-repository progress tracker for workspace mode.
/// Implements `UpdateCallbacks` to receive completion notifications.
#[derive(Clone)]
pub struct RepoProgressTracker {
    repo_name: String,
    workspace: WorkspaceProgress,
    config: Config,
}

impl UpdateCallbacks for RepoProgressTracker {
    fn on_update_start(&self, repo_name: &str) {
        print_repo_header(&self.config, repo_name);
    }

    fn on_step(&self, _step: &UpdateStep) {}

    fn on_step_execute(&self, step: &UpdateStep) {
        print_step(&self.config, step);
    }

    fn on_complete(&self, result: &UpdateResult) {
        let success = matches!(result.outcome, UpdateOutcome::Success(_));
        self.workspace.mark_completed(&self.repo_name, success);
    }

    fn on_completion_status(&self, success: bool, error: Option<&str>) {
        print_completion_status(&self.config, success, error);
    }
}

/// Creates a spinner-based progress tracker for single repository updates.
/// Returns `None` in quiet or verbose mode to avoid allocation.
#[must_use]
pub fn create_single_repo_progress(config: &Config) -> SingleRepoProgress {
    let spinner = if config.is_quiet() || config.is_verbose() {
        None
    } else {
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
                .template("{spinner:.cyan} {msg}")
                .unwrap(),
        );
        spinner.enable_steady_tick(Duration::from_millis(PROGRESS_TICK_MS));
        Some(spinner)
    };

    SingleRepoProgress { spinner }
}

/// Creates a progress bar for workspace updates showing completion count.
/// Returns hidden progress bars in quiet or verbose mode.
#[must_use]
pub fn create_workspace_progress(total: usize, config: &Config) -> WorkspaceProgress {
    let multi = Arc::new(MultiProgress::new());
    let hide_progress = config.is_quiet() || config.is_verbose();

    let main_bar = if hide_progress {
        ProgressBar::hidden()
    } else {
        let bar = multi.add(ProgressBar::new(total as u64));
        bar.set_style(
            ProgressStyle::default_bar()
                .template("{bar:40.cyan/blue} {pos}/{len} completed {spinner:.cyan} {msg}")
                .unwrap()
                .progress_chars("█░"),
        );
        bar.enable_steady_tick(Duration::from_millis(PROGRESS_TICK_MS));
        bar
    };

    let completion_slots: Vec<ProgressBar> = if hide_progress {
        vec![]
    } else {
        (0..MAX_VISIBLE_COMPLETIONS)
            .map(|_| {
                let slot = multi.add(ProgressBar::new_spinner());
                slot.set_style(
                    ProgressStyle::default_spinner()
                        .template("  {msg}")
                        .unwrap(),
                );
                slot
            })
            .collect()
    };

    WorkspaceProgress {
        _multi: multi,
        main_bar,
        completion_slots,
        state: Arc::new(Mutex::new(CompletionState {
            repos: VecDeque::new(),
            failed_count: 0,
            total_completed: 0,
        })),
    }
}

pub fn print_working_dir(path: &Path, config: &Config) {
    if config.is_quiet() {
        return;
    }
    println!(
        "{} {}",
        "Working in:".cyan(),
        path.display().to_string().white().bold()
    )
}

pub fn print_workspace_start(count: usize, config: &Config) {
    if config.is_quiet() {
        return;
    }
    if count == 0 {
        print_no_repos()
    } else {
        println!(
            "{}",
            format!("Starting in workspace mode with {} repositories", count).dimmed()
        )
    }
}

pub fn print_summary(results: &[UpdateResult], duration: Duration, config: &Config) {
    if config.is_quiet() {
        print_quiet_summary(results);
    } else {
        print_normal_summary(results, duration);
    }
}

fn print_quiet_summary(results: &[UpdateResult]) {
    let (successes, failures): (Vec<_>, Vec<_>) = results
        .iter()
        .partition(|r| matches!(r.outcome, UpdateOutcome::Success(_)));

    // Always print count to stdout
    println!("{}/{} repositories updated", successes.len(), results.len());

    // Print failures to stderr
    for result in &failures {
        if let UpdateOutcome::Failed(failure) = &result.outcome {
            eprintln!("error: {}: {}", result.path.display(), failure.error);
        }
    }
}

fn print_normal_summary(results: &[UpdateResult], duration: Duration) {
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
                success.original_head.display().cyan(),
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

fn format_step_message(step: &UpdateStep) -> &'static str {
    match step {
        UpdateStep::Started => "Starting update...",
        UpdateStep::DetectingBranch => "Detecting current branch...",
        UpdateStep::CheckingChanges => "Checking for uncommitted changes...",
        UpdateStep::Fetching => "Fetching from origin...",
        UpdateStep::Stashing => "Stashing uncommitted changes...",
        UpdateStep::CheckingOut => "Checking out master branch...",
        UpdateStep::Pulling => "Pulling changes from origin...",
        UpdateStep::RestoringBranch => "Restoring original branch...",
        UpdateStep::PoppingStash => "Restoring stashed changes...",
        UpdateStep::Completed => "Completed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::{OriginalHead, UpdateFailure, UpdateSuccess};
    use std::path::PathBuf;

    #[test]
    fn test_format_duration_rounds_to_two_decimals() {
        assert_eq!(format_duration(Duration::from_millis(1234)), "1.23s");
        assert_eq!(format_duration(Duration::from_millis(5678)), "5.68s");
        assert_eq!(format_duration(Duration::from_secs(42)), "42.00s");
    }

    #[test]
    fn test_format_step_message_covers_all_known_steps() {
        // Ensure all known steps have meaningful messages
        assert_eq!(
            format_step_message(&UpdateStep::Started),
            "Starting update..."
        );
        assert_eq!(
            format_step_message(&UpdateStep::DetectingBranch),
            "Detecting current branch..."
        );
        assert_eq!(
            format_step_message(&UpdateStep::CheckingChanges),
            "Checking for uncommitted changes..."
        );
        assert_eq!(
            format_step_message(&UpdateStep::Fetching),
            "Fetching from origin..."
        );
        assert_eq!(
            format_step_message(&UpdateStep::Stashing),
            "Stashing uncommitted changes..."
        );
        assert_eq!(
            format_step_message(&UpdateStep::CheckingOut),
            "Checking out master branch..."
        );
        assert_eq!(
            format_step_message(&UpdateStep::Pulling),
            "Pulling changes from origin..."
        );
        assert_eq!(
            format_step_message(&UpdateStep::RestoringBranch),
            "Restoring original branch..."
        );
        assert_eq!(
            format_step_message(&UpdateStep::PoppingStash),
            "Restoring stashed changes..."
        );
        assert_eq!(format_step_message(&UpdateStep::Completed), "Completed");
    }

    #[test]
    fn test_no_op_callbacks_implements_all_required_methods() {
        let callbacks = NoOpCallbacks;
        let result = UpdateResult {
            path: PathBuf::from("/test/repo"),
            outcome: UpdateOutcome::Success(UpdateSuccess {
                original_head: OriginalHead::Branch("main".to_string()),
                master_branch: "main",
                had_stash: false,
            }),
            duration: Duration::from_secs(1),
        };

        // These should not panic
        callbacks.on_update_start("test");
        callbacks.on_step(&UpdateStep::Started);
        callbacks.on_step_execute(&UpdateStep::Fetching);
        callbacks.on_complete(&result);
        callbacks.on_completion_status(true, None);
    }

    #[test]
    fn test_quiet_summary_format() {
        // This is more of a smoke test - we can't easily test stderr output
        // but we can ensure it doesn't panic with various inputs
        let success = UpdateResult {
            path: PathBuf::from("/test/success"),
            outcome: UpdateOutcome::Success(UpdateSuccess {
                original_head: OriginalHead::Branch("feature".to_string()),
                master_branch: "master",
                had_stash: true,
            }),
            duration: Duration::from_secs(2),
        };

        let failure = UpdateResult {
            path: PathBuf::from("/test/failure"),
            outcome: UpdateOutcome::Failed(UpdateFailure {
                error: "test error".to_string(),
                step: UpdateStep::Fetching,
            }),
            duration: Duration::from_millis(500),
        };

        // Should not panic
        print_quiet_summary(std::slice::from_ref(&success));
        print_quiet_summary(std::slice::from_ref(&failure));
        print_quiet_summary(&[success, failure]);
        print_quiet_summary(&[]);
    }
}
