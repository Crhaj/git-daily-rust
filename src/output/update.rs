//! Real-time progress tracking for repository updates.
//!
//! Provides spinners and progress bars for single-repo and workspace modes.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::config::Config;
use crate::constants::{DEFAULT_REPO_NAME, MAX_VISIBLE_COMPLETIONS, PROGRESS_TICK_MS};
use crate::repo::{UpdateCallbacks, UpdateOutcome, UpdateResult, UpdateStep};

/// No-op callbacks for when progress tracking is not needed.
///
/// This is the null object pattern for `UpdateCallbacks` - use it when
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
    eprintln!("{}", build_repo_header_line(repo_name));
}

/// Prints a step progress message in verbose mode.
pub fn print_step(config: &Config, step: &UpdateStep) {
    if !config.is_verbose() {
        return;
    }
    eprintln!("{}", build_step_line(step));
}

/// Prints completion status (verbose mode only).
pub fn print_completion_status(config: &Config, success: bool, error: Option<&str>) {
    if !config.is_verbose() {
        return;
    }
    if let Some(line) = build_completion_status_line(success, error) {
        eprintln!("{}", line);
    }
}

/// Prints the current working directory.
pub fn print_working_dir(path: &Path, config: &Config) {
    if config.is_quiet() {
        return;
    }
    println!("{}", build_working_dir_line(path));
}

/// Prints the workspace mode start message.
pub fn print_workspace_start(count: usize, config: &Config) {
    if config.is_quiet() {
        return;
    }
    println!("{}", build_workspace_start_line(count));
}

/// Progress wrapper for single repository updates.
///
/// Displays a spinner with step-by-step status messages.
/// Uses `Option` to avoid allocation when progress is hidden (quiet/verbose modes).
pub struct SingleRepoProgress {
    spinner: Option<ProgressBar>,
}

impl SingleRepoProgress {
    /// Updates the spinner with the current step message.
    pub fn update(&self, step: &UpdateStep) {
        if let Some(spinner) = &self.spinner {
            let message = format_step_message(step);
            spinner.set_message(message);
        }
    }

    /// Finishes the spinner with a success message.
    pub fn finish_success(&self, repo_name: &str) {
        if let Some(spinner) = &self.spinner {
            spinner.finish_with_message(format!(
                "{} {} updated successfully",
                "✓".green(),
                repo_name
            ));
        }
    }

    /// Finishes the spinner with a failure message.
    pub fn finish_failed(&self, repo_name: &str, error: &str) {
        if let Some(spinner) = &self.spinner {
            spinner.finish_with_message(format!("{} {} failed: {}", "✗".red(), repo_name, error));
        }
    }
}

/// Callbacks for single repository updates.
///
/// Combines progress bar updates with verbose output handling.
pub struct SingleRepoCallbacks {
    progress: SingleRepoProgress,
    config: Config,
}

impl SingleRepoCallbacks {
    /// Creates new callbacks with the given progress tracker and config.
    pub fn new(progress: SingleRepoProgress, config: Config) -> Self {
        Self { progress, config }
    }

    /// Finishes the progress bar with success/failure message.
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
///
/// Combining these fields reduces lock contention by acquiring a single lock
/// instead of multiple separate locks for related data.
struct CompletionState {
    /// Recently completed repos for display (bounded by MAX_VISIBLE_COMPLETIONS).
    repos: VecDeque<(String, bool)>,
    /// Count of failed repos for status message.
    failed_count: usize,
    /// Total completed for determining ellipsis display.
    total_completed: usize,
}

/// Thread-safe progress tracker for workspace mode.
///
/// Shows a progress bar with the completion count and recent results.
#[derive(Clone)]
pub struct WorkspaceProgress {
    _multi: Arc<MultiProgress>,
    main_bar: ProgressBar,
    completion_slots: Vec<ProgressBar>,
    state: Arc<Mutex<CompletionState>>,
}

impl WorkspaceProgress {
    /// Creates a per-repository progress tracker.
    pub fn create_repo_tracker(&self, repo_name: &str, config: Config) -> RepoProgressTracker {
        RepoProgressTracker {
            repo_name: repo_name.to_string(),
            workspace: self.clone(),
            config,
        }
    }

    /// Marks a repository as completed.
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

    /// Clears and finishes the progress display.
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
///
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
///
/// Returns a hidden spinner in quiet or verbose mode to avoid allocation.
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
///
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

fn build_repo_header_line(repo_name: &str) -> String {
    format!("\n{}", format!("[{}]", repo_name).white().bold())
}

fn build_step_line(step: &UpdateStep) -> String {
    format!("  {}...", step.to_string().dimmed())
}

fn build_completion_status_line(success: bool, error: Option<&str>) -> Option<String> {
    if success {
        Some(format!("  {} completed successfully", "✓".green()))
    } else {
        error.map(|err| format!("  {} failed: {}", "✗".red(), err))
    }
}

fn build_working_dir_line(path: &Path) -> String {
    format!(
        "{} {}",
        "Working in:".cyan(),
        path.display().to_string().white().bold()
    )
}

fn build_workspace_start_line(count: usize) -> String {
    if count == 0 {
        "No git repositories found".yellow().bold().to_string()
    } else {
        format!("Starting in workspace mode with {} repositories", count)
    }
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
    use crate::config::Verbosity;

    fn make_config(verbosity: Verbosity) -> Config {
        Config { verbosity }
    }

    #[test]
    fn test_no_op_callbacks_is_default() {
        let _: NoOpCallbacks = Default::default();
    }

    #[test]
    fn test_single_repo_progress_smoke() {
        let config = make_config(Verbosity::Normal);
        let progress = create_single_repo_progress(&config);
        progress.update(&UpdateStep::Fetching);
        progress.finish_success("test-repo");
    }

    #[test]
    fn test_single_repo_progress_quiet_mode_no_spinner() {
        let config = make_config(Verbosity::Quiet);
        let progress = create_single_repo_progress(&config);
        // Should not panic even without a spinner
        progress.update(&UpdateStep::Fetching);
        progress.finish_success("test-repo");
    }

    #[test]
    fn test_workspace_progress_mark_completed_smoke() {
        let config = make_config(Verbosity::Normal);
        let progress = create_workspace_progress(5, &config);

        progress.mark_completed("repo-1", true);
        progress.mark_completed("repo-2", false);
        progress.finish();
    }

    #[test]
    fn test_workspace_progress_tracker_and_capacity() {
        let config = make_config(Verbosity::Normal);
        let workspace = create_workspace_progress(10, &config);

        // Mark more than MAX_VISIBLE_COMPLETIONS to test bounded deque
        for i in 0..8 {
            workspace.mark_completed(&format!("repo-{}", i), i % 2 == 0);
        }

        let state = workspace.state.lock().unwrap();
        assert!(state.repos.len() <= MAX_VISIBLE_COMPLETIONS);
        assert_eq!(state.total_completed, 8);

        drop(state);
        workspace.finish();
    }

    #[test]
    fn test_build_repo_header_line_format() {
        let line = build_repo_header_line("my-repo");
        assert!(line.contains("my-repo"));
    }

    #[test]
    fn test_build_step_line_format() {
        let line = build_step_line(&UpdateStep::Fetching);
        assert!(line.contains("Fetching"));
    }

    #[test]
    fn test_build_completion_status_line_success() {
        let line = build_completion_status_line(true, None);
        assert!(line.is_some());
        assert!(line.unwrap().contains("successfully"));
    }

    #[test]
    fn test_build_completion_status_line_failure() {
        let line = build_completion_status_line(false, Some("network error"));
        assert!(line.is_some());
        assert!(line.unwrap().contains("network error"));
    }

    #[test]
    fn test_build_completion_status_line_failure_no_error() {
        let line = build_completion_status_line(false, None);
        assert!(line.is_none());
    }

    #[test]
    fn test_build_working_dir_line_format() {
        let line = build_working_dir_line(Path::new("/tmp/my-project"));
        assert!(line.contains("my-project"));
    }

    #[test]
    fn test_build_workspace_start_line_with_repos() {
        let line = build_workspace_start_line(5);
        assert!(line.contains("5"));
        assert!(line.contains("repositories"));
    }

    #[test]
    fn test_build_workspace_start_line_no_repos() {
        let line = build_workspace_start_line(0);
        assert!(line.contains("No git repositories"));
    }

    #[test]
    fn test_format_step_message_all_steps() {
        // Ensure all steps have messages (compile-time exhaustiveness)
        let steps = [
            UpdateStep::Started,
            UpdateStep::DetectingBranch,
            UpdateStep::CheckingChanges,
            UpdateStep::Fetching,
            UpdateStep::Stashing,
            UpdateStep::CheckingOut,
            UpdateStep::Pulling,
            UpdateStep::RestoringBranch,
            UpdateStep::PoppingStash,
            UpdateStep::Completed,
        ];

        for step in steps {
            let msg = format_step_message(&step);
            assert!(!msg.is_empty());
        }
    }
}
