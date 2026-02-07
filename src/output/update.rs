//! Real-time progress tracking for repository updates.
//!
//! Provides spinners and progress bars for single-repo and workspace modes.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::config::Config;
use crate::constants::{
    DEFAULT_REPO_NAME, MAX_VISIBLE_ACTIVE, MAX_VISIBLE_COMPLETIONS, PROGRESS_TICK_MS,
};
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
    completed_repos: VecDeque<(String, bool)>,
    /// Count of failed repos for status message.
    failed_count: usize,
    /// Total completed for determining ellipsis display.
    total_completed: usize,
    /// Active repos with their current step (repo_name -> step).
    active_repos: HashMap<String, UpdateStep>,
}

/// Thread-safe progress tracker for workspace mode.
///
/// Shows a progress bar with the completion count and recent results.
/// Designed for high concurrency (51+ parallel repos) with minimal lock contention.
#[derive(Clone)]
pub struct WorkspaceProgress {
    /// Owns the MultiProgress container that manages all progress bars.
    /// Must be kept alive for child progress bars to render correctly.
    /// Not accessed directly but required for indicatif's internal bookkeeping.
    #[allow(dead_code)]
    multi: Arc<MultiProgress>,
    main_bar: ProgressBar,
    activity_slots: Vec<ProgressBar>,
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

    /// Updates the current step for a repository.
    pub fn update_step(&self, repo_name: &str, step: &UpdateStep) {
        let mut state = self
            .state
            .lock()
            .expect("WorkspaceProgress state mutex poisoned");

        state.active_repos.insert(repo_name.to_string(), *step);
        self.update_message(&state);
        self.redraw_activity(&state);
    }

    /// Marks a repository as completed.
    pub fn mark_completed(&self, repo_name: &str, success: bool) {
        self.main_bar.inc(1);

        let mut state = self
            .state
            .lock()
            .expect("WorkspaceProgress state mutex poisoned");

        state.active_repos.remove(repo_name);

        if !success {
            state.failed_count += 1;
        }

        self.update_message(&state);
        self.redraw_activity(&state);

        state.total_completed += 1;
        state
            .completed_repos
            .push_back((repo_name.to_string(), success));

        while state.completed_repos.len() > MAX_VISIBLE_COMPLETIONS {
            state.completed_repos.pop_front();
        }

        self.redraw_completions(&state);
    }

    /// Updates the progress bar message with current status.
    fn update_message(&self, state: &CompletionState) {
        let active_count = state.active_repos.len();
        let mut parts = Vec::new();

        if active_count > 0 {
            parts.push(format!("{} active", active_count).cyan().to_string());
        }

        if state.failed_count > 0 {
            parts.push(format!("{} failed", state.failed_count).red().to_string());
        }

        if parts.is_empty() {
            self.main_bar.set_message("");
        } else {
            self.main_bar.set_message(format!("│ {}", parts.join(", ")));
        }
    }

    /// Clears and finishes the progress display.
    pub fn finish(&self) {
        self.main_bar.finish_and_clear();
        for slot in &self.activity_slots {
            slot.finish_and_clear();
        }
        for slot in &self.completion_slots {
            slot.finish_and_clear();
        }
    }

    /// Redraws the activity slots showing repos currently being updated.
    fn redraw_activity(&self, state: &CompletionState) {
        // Prioritize repos in later phases (pulling > fetching > others).
        // Use repo name as secondary sort key for stable ordering - prevents
        // UI flickering when multiple repos are in the same phase.
        let mut active: Vec<_> = state.active_repos.iter().collect();
        active.sort_by(|(name_a, step_a), (name_b, step_b)| {
            step_priority(step_a)
                .cmp(&step_priority(step_b))
                .then_with(|| name_a.cmp(name_b))
        });

        for (i, slot) in self.activity_slots.iter().enumerate() {
            if i < active.len() {
                let (name, step) = active[i];
                let icon = step_icon(step);
                let step_name = step_short_name(step);
                slot.set_message(format!(
                    "  {}  {} {}",
                    truncate_name(name, 20),
                    step_name.dimmed(),
                    icon
                ));
            } else {
                slot.set_message("");
            }
        }
    }

    fn redraw_completions(&self, state: &CompletionState) {
        let show_ellipsis = state.total_completed > MAX_VISIBLE_COMPLETIONS;

        for (i, slot) in self.completion_slots.iter().enumerate() {
            if i == 0 && show_ellipsis {
                slot.set_message("...".dimmed().to_string());
            } else {
                let idx = if show_ellipsis { i - 1 } else { i };
                if idx < state.completed_repos.len() {
                    let (name, success) = &state.completed_repos[idx];
                    let symbol = if *success { "✓".green() } else { "✗".red() };
                    slot.set_message(format!("{} {}", symbol, name));
                } else {
                    slot.set_message("");
                }
            }
        }
    }
}

/// Returns a priority value for sorting steps (lower = show first, later phases prioritized).
fn step_priority(step: &UpdateStep) -> u8 {
    match step {
        UpdateStep::Pulling => 0,      // Show pulling first (almost done)
        UpdateStep::PoppingStash => 1, // Finishing up
        UpdateStep::RestoringBranch => 2,
        UpdateStep::CheckingOut => 3,
        UpdateStep::Stashing => 4,
        UpdateStep::Fetching => 5, // Most common slow step
        UpdateStep::CheckingChanges => 6,
        UpdateStep::DetectingBranch => 7,
        UpdateStep::Started => 8,
        UpdateStep::Completed => 9,
    }
}

/// Returns a short icon for each step.
fn step_icon(step: &UpdateStep) -> &'static str {
    match step {
        UpdateStep::Fetching => "⟳",
        UpdateStep::Pulling => "↓",
        UpdateStep::Stashing | UpdateStep::PoppingStash => "📦",
        UpdateStep::CheckingOut | UpdateStep::RestoringBranch => "⎇",
        _ => "·",
    }
}

/// Returns a short name for each step.
fn step_short_name(step: &UpdateStep) -> &'static str {
    match step {
        UpdateStep::Started => "starting",
        UpdateStep::DetectingBranch => "detecting",
        UpdateStep::CheckingChanges => "checking",
        UpdateStep::Fetching => "fetching",
        UpdateStep::Stashing => "stashing",
        UpdateStep::CheckingOut => "checkout",
        UpdateStep::Pulling => "pulling",
        UpdateStep::RestoringBranch => "restoring",
        UpdateStep::PoppingStash => "unstashing",
        UpdateStep::Completed => "done",
    }
}

/// Truncates a name to fit in a fixed width, padding with spaces.
///
/// Handles Unicode correctly by counting characters, not bytes.
fn truncate_name(name: &str, max_len: usize) -> String {
    let char_count = name.chars().count();
    if char_count <= max_len {
        format!("{:<width$}", name, width = max_len)
    } else {
        let truncated: String = name.chars().take(max_len - 1).collect();
        format!("{}…", truncated)
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

    fn on_step(&self, step: &UpdateStep) {
        self.workspace.update_step(&self.repo_name, step);
    }

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

    // Activity slots show currently active repos with their step
    let activity_slots: Vec<ProgressBar> = if hide_progress {
        vec![]
    } else {
        (0..MAX_VISIBLE_ACTIVE)
            .map(|_| {
                let slot = multi.add(ProgressBar::new_spinner());
                slot.set_style(ProgressStyle::default_spinner().template("{msg}").unwrap());
                slot
            })
            .collect()
    };

    // Completion slots show recently completed repos
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
        multi,
        main_bar,
        activity_slots,
        completion_slots,
        state: Arc::new(Mutex::new(CompletionState {
            completed_repos: VecDeque::new(),
            failed_count: 0,
            total_completed: 0,
            active_repos: HashMap::new(),
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
        Config {
            verbosity,
            debug: false,
        }
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
        assert!(state.completed_repos.len() <= MAX_VISIBLE_COMPLETIONS);
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

    #[test]
    fn test_step_priority_orders_later_phases_first() {
        // Pulling should come before Fetching (lower priority = shown first)
        assert!(step_priority(&UpdateStep::Pulling) < step_priority(&UpdateStep::Fetching));
        assert!(step_priority(&UpdateStep::Fetching) < step_priority(&UpdateStep::DetectingBranch));
        assert!(step_priority(&UpdateStep::PoppingStash) < step_priority(&UpdateStep::Stashing));
    }

    #[test]
    fn test_step_icon_returns_icons_for_slow_steps() {
        assert_eq!(step_icon(&UpdateStep::Fetching), "⟳");
        assert_eq!(step_icon(&UpdateStep::Pulling), "↓");
        assert_eq!(step_icon(&UpdateStep::Stashing), "📦");
        assert_eq!(step_icon(&UpdateStep::PoppingStash), "📦");
    }

    #[test]
    fn test_step_short_name_returns_short_names() {
        assert_eq!(step_short_name(&UpdateStep::Fetching), "fetching");
        assert_eq!(step_short_name(&UpdateStep::Pulling), "pulling");
        assert_eq!(step_short_name(&UpdateStep::DetectingBranch), "detecting");
    }

    #[test]
    fn test_truncate_name_pads_short_names() {
        let result = truncate_name("foo", 10);
        assert_eq!(result, "foo       ");
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn test_truncate_name_truncates_long_names() {
        let result = truncate_name("very-long-repo-name", 10);
        assert_eq!(result, "very-long…");
    }

    #[test]
    fn test_truncate_name_handles_exact_length() {
        let result = truncate_name("exactly-10", 10);
        assert_eq!(result, "exactly-10");
    }

    #[test]
    fn test_truncate_name_handles_unicode() {
        // Japanese characters (3 bytes each in UTF-8)
        let result = truncate_name("日本語リポジトリ", 5);
        assert!(result.ends_with('…'));
        assert_eq!(result.chars().count(), 5); // 4 chars + ellipsis
    }

    #[test]
    fn test_workspace_progress_update_step() {
        let config = make_config(Verbosity::Normal);
        let workspace = create_workspace_progress(5, &config);

        workspace.update_step("repo-1", &UpdateStep::Fetching);
        workspace.update_step("repo-2", &UpdateStep::Pulling);

        let state = workspace.state.lock().unwrap();
        assert_eq!(state.active_repos.len(), 2);
        assert_eq!(
            state.active_repos.get("repo-1"),
            Some(&UpdateStep::Fetching)
        );
        assert_eq!(state.active_repos.get("repo-2"), Some(&UpdateStep::Pulling));

        drop(state);
        workspace.finish();
    }

    #[test]
    fn test_workspace_progress_removes_completed_from_active() {
        let config = make_config(Verbosity::Normal);
        let workspace = create_workspace_progress(5, &config);

        workspace.update_step("repo-1", &UpdateStep::Fetching);
        workspace.mark_completed("repo-1", true);

        let state = workspace.state.lock().unwrap();
        assert!(state.active_repos.is_empty());
        assert_eq!(state.completed_repos.len(), 1);

        drop(state);
        workspace.finish();
    }
}
