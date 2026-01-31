//! Progress bars, colored output, and summary formatting.
//!
//! This module provides visual feedback during repository updates including
//! spinners, progress bars, and colored summary output.

use crate::cleanup::{
    BranchInfo, CleanupResult, DeletionOutcome, DeletionResult, MergeStatus, TrackingStatus,
};
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
    println!("{}", build_working_dir_line(path));
}

pub fn print_workspace_start(count: usize, config: &Config) {
    if config.is_quiet() {
        return;
    }
    println!("{}", build_workspace_start_line(count));
}

pub fn print_summary(results: &[UpdateResult], duration: Duration, config: &Config) {
    if config.is_quiet() {
        print_quiet_summary(results);
    } else {
        print_normal_summary(results, duration);
    }
}

fn print_quiet_summary(results: &[UpdateResult]) {
    let (stdout_line, stderr_lines) = build_quiet_summary(results);
    println!("{}", stdout_line);
    for line in stderr_lines {
        eprintln!("{}", line);
    }
}

fn print_normal_summary(results: &[UpdateResult], duration: Duration) {
    let output = build_normal_summary(results, duration);
    print!("{}", output);
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
        build_no_repos_line()
    } else {
        format!("Starting in workspace mode with {} repositories", count)
            .dimmed()
            .to_string()
    }
}

fn build_no_repos_line() -> String {
    "No git repositories found".yellow().bold().to_string()
}

fn build_quiet_summary(results: &[UpdateResult]) -> (String, Vec<String>) {
    let (successes, failures): (Vec<_>, Vec<_>) = results
        .iter()
        .partition(|r| matches!(r.outcome, UpdateOutcome::Success(_)));

    let stdout_line = format!("{}/{} repositories updated", successes.len(), results.len());
    let stderr_lines = failures
        .iter()
        .filter_map(|result| match &result.outcome {
            UpdateOutcome::Failed(failure) => Some(format!(
                "error: {}: {}",
                result.path.display(),
                failure.error
            )),
            _ => None,
        })
        .collect();

    (stdout_line, stderr_lines)
}

fn build_normal_summary(results: &[UpdateResult], duration: Duration) -> String {
    let mut output = String::new();
    output.push_str(&build_section("Summary"));

    let (successes, failures): (Vec<_>, Vec<_>) = results
        .iter()
        .partition(|r| matches!(r.outcome, UpdateOutcome::Success(_)));

    output.push_str(&build_success_lines(&successes));
    output.push_str(&build_failure_lines(&failures));
    output.push_str(&format!(
        "{}: {}/{} repos in {}",
        "Total".white().bold(),
        successes.len(),
        results.len(),
        format_duration(duration)
    ));
    output.push('\n');

    output
}

fn format_duration(duration: Duration) -> String {
    format!("{:.2}s", duration.as_secs_f32())
}

fn build_section(title: &str) -> String {
    let line = "=".repeat(50).cyan().dimmed();
    let padding = (50 - title.len()) / 2;
    let centered = format!("{:>width$}", title, width = padding + title.len());
    format!("\n{}\n{}\n{}\n\n", line, centered.cyan().bold(), line)
}

fn build_success_lines(successes: &[&UpdateResult]) -> String {
    let mut output = String::new();
    if successes.is_empty() {
        return output;
    }
    output.push_str(&format!(
        "{}",
        format!("Succeeded ({}):", successes.len()).green().bold()
    ));
    output.push('\n');

    for result in successes {
        if let UpdateOutcome::Success(success) = &result.outcome {
            let stash_msg = if success.had_stash {
                " (stash restored)".yellow()
            } else {
                "".normal()
            };
            output.push_str(&format!(
                "  {} {} {} {} in {}",
                "OK".green().bold(),
                result.path.display().to_string().white(),
                success.original_head.display().cyan(),
                stash_msg,
                format_duration(result.duration).dimmed(),
            ));
            output.push('\n');
        }
    }
    output.push('\n');
    output
}

fn build_failure_lines(failures: &[&UpdateResult]) -> String {
    let mut output = String::new();
    if failures.is_empty() {
        return output;
    }

    output.push_str(&format!(
        "{}",
        format!("Failed ({}):", failures.len()).red().bold()
    ));
    output.push('\n');

    for result in failures {
        if let UpdateOutcome::Failed(failure) = &result.outcome {
            output.push_str(&format!(
                "  {} {} {} in {}",
                "FAIL".red().bold(),
                result.path.display().to_string().white(),
                format!("at {:?}: {}", failure.step, failure.error).red(),
                format_duration(result.duration).dimmed(),
            ));
            output.push('\n');
        }
    }
    output.push('\n');
    output
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

/// Width of the summary box divider line.
const SUMMARY_BOX_WIDTH: usize = 60;

/// Column widths for aligned branch list display.
///
/// Use [`BranchListWidths::from_branches`] to calculate optimal widths
/// based on the actual branch names in your list.
#[derive(Debug, Clone, Copy)]
pub struct BranchListWidths {
    name: usize,
    status: usize,
    remote: usize,
}

impl BranchListWidths {
    const MIN_NAME_WIDTH: usize = 6;
    const MAX_NAME_WIDTH: usize = 60;
    const STATUS_WIDTH: usize = 16; // "(current branch)" is 16 chars
    const REMOTE_WIDTH: usize = 7; // "unknown" is 7 chars

    /// Calculates optimal column widths based on branch data.
    ///
    /// Name width is clamped between `MIN_NAME_WIDTH` (6) and `MAX_NAME_WIDTH` (60)
    /// to ensure readable output even with very long branch names.
    #[must_use]
    pub fn from_branches(branches: &[BranchInfo]) -> Self {
        let max_name = branches
            .iter()
            .map(|b| b.name.len())
            .max()
            .unwrap_or(Self::MIN_NAME_WIDTH)
            .clamp(Self::MIN_NAME_WIDTH, Self::MAX_NAME_WIDTH);

        Self {
            name: max_name,
            status: Self::STATUS_WIDTH,
            remote: Self::REMOTE_WIDTH,
        }
    }

    fn total_width(&self) -> usize {
        self.name + self.status + self.remote + 4 // 4 for spacing between columns
    }
}

/// Formats a single branch line for display.
///
/// The output includes the branch name, merge status, and tracking status
/// with appropriate coloring based on safety.
///
/// # Example
///
/// ```
/// use git_daily_rust::cleanup::{BranchInfo, MergeStatus, TrackingStatus};
/// use git_daily_rust::output::{BranchListWidths, format_branch_line};
///
/// let branch = BranchInfo {
///     name: "feature/login".to_string(),
///     is_current: false,
///     merge_status: MergeStatus::Merged,
///     tracking_status: TrackingStatus::RemoteGone,
/// };
/// let widths = BranchListWidths::from_branches(&[branch.clone()]);
/// let line = format_branch_line(&branch, &widths);
/// assert!(line.contains("feature/login"));
/// ```
#[must_use]
pub fn format_branch_line(branch: &BranchInfo, widths: &BranchListWidths) -> String {
    let prefix = if branch.is_current { "*" } else { " " };
    let name = &branch.name;
    let status = format_branch_status(branch);
    let remote = format_tracking_status(&branch.tracking_status);
    let warning = format_branch_warning(branch);

    format!(
        "{} {:<name_width$}  {:<status_width$}  {}{}",
        prefix,
        name,
        status,
        remote,
        warning,
        name_width = widths.name,
        status_width = widths.status,
    )
}

/// Prints the header for the branch list.
pub fn print_branch_list_header(widths: &BranchListWidths, config: &Config) {
    if config.is_quiet() {
        return;
    }
    let header = build_branch_list_header(widths);
    eprintln!("{}", header);
}

/// Prints the complete branch list with header.
pub fn print_branch_list(branches: &[BranchInfo], config: &Config) {
    if config.is_quiet() {
        return;
    }

    let widths = BranchListWidths::from_branches(branches);
    print_branch_list_header(&widths, config);

    for branch in branches {
        eprintln!("{}", format_branch_line(branch, &widths));
    }
}

/// Prints a single deletion result (real-time feedback during deletion).
pub fn print_deletion_result(result: &DeletionResult, config: &Config) {
    if config.is_quiet() {
        return;
    }
    let line = build_deletion_result_line(result);
    eprintln!("{}", line);
}

/// Prints the final cleanup summary.
pub fn print_cleanup_summary(result: &CleanupResult, remaining: &[BranchInfo], config: &Config) {
    if config.is_quiet() {
        print_quiet_cleanup_summary(result);
    } else {
        print_normal_cleanup_summary(result, remaining);
    }
}

/// Prints the "Analyzing branches..." message.
pub fn print_analyzing_branches(config: &Config) {
    if config.is_quiet() {
        return;
    }
    eprintln!("{}", "Analyzing branches...".dimmed());
}

/// Prints "No branches available for cleanup" message.
pub fn print_no_branches_to_clean(config: &Config) {
    if config.is_quiet() {
        return;
    }
    eprintln!("{}", "No branches available for cleanup.".yellow().bold());
    eprintln!(
        "{}",
        "All branches are either current or protected.".dimmed()
    );
}

/// Prints "Deleting branches..." header.
pub fn print_deleting_header(config: &Config) {
    if config.is_quiet() {
        return;
    }
    eprintln!("\n{}", "Deleting branches...".cyan());
}

fn format_branch_status(branch: &BranchInfo) -> String {
    if branch.is_current {
        "(current branch)".cyan().to_string()
    } else {
        format_merge_status(&branch.merge_status)
    }
}

/// Formats merge status with semantic color coding.
/// Uses semantic methods to avoid duplicating variant knowledge.
fn format_merge_status(status: &MergeStatus) -> String {
    let label = status.to_string();
    if status.is_safely_deletable() {
        label.green().to_string()
    } else if status.requires_caution() {
        label.yellow().to_string()
    } else {
        // Uncertain status (unclear)
        label.magenta().to_string()
    }
}

/// Formats tracking status with semantic color coding.
/// Active remotes are normal, inactive are dimmed.
fn format_tracking_status(status: &TrackingStatus) -> String {
    let label = status.to_string();
    if status.is_active() {
        label
    } else {
        label.dimmed().to_string()
    }
}

fn format_branch_warning(branch: &BranchInfo) -> String {
    if branch.is_current {
        return String::new();
    }

    let status = &branch.merge_status;
    if status.is_uncertain() {
        format!("   {}", "may be squash-merged".yellow())
    } else if status.requires_caution() {
        format!("   {}", "unmerged".yellow())
    } else {
        String::new()
    }
}

fn build_branch_list_header(widths: &BranchListWidths) -> String {
    let header_line = format!(
        "  {:<name_width$}  {:<status_width$}  {}",
        "BRANCH",
        "STATUS",
        "REMOTE",
        name_width = widths.name,
        status_width = widths.status,
    );

    let separator = "─".repeat(widths.total_width());

    format!("{}\n  {}", header_line.white().bold(), separator.dimmed())
}

fn build_deletion_result_line(result: &DeletionResult) -> String {
    match &result.outcome {
        DeletionOutcome::Deleted => {
            format!("  {} {}", "✓".green(), result.branch)
        }
        DeletionOutcome::ForceDeleted => {
            format!("  {} {} {}", "✓".green(), result.branch, "(force)".dimmed())
        }
        DeletionOutcome::Skipped { reason } => {
            format!("  {} {}: {}", "○".dimmed(), result.branch, reason.dimmed())
        }
        DeletionOutcome::Failed { error } => {
            format!("  {} {}: {}", "✗".red(), result.branch, error.red())
        }
    }
}

fn print_quiet_cleanup_summary(result: &CleanupResult) {
    // Single pass: count deletions and collect failed errors
    let mut deleted_count = 0;
    let mut failed_errors: Vec<(&str, &str)> = Vec::new();

    for d in &result.deletions {
        match &d.outcome {
            DeletionOutcome::Deleted | DeletionOutcome::ForceDeleted => {
                deleted_count += 1;
            }
            DeletionOutcome::Failed { error } => {
                failed_errors.push((&d.branch, error));
            }
            DeletionOutcome::Skipped { .. } => {}
        }
    }

    println!(
        "{}/{} branches deleted",
        deleted_count,
        result.deletions.len()
    );

    for (branch, error) in &failed_errors {
        eprintln!("error: {}: {}", branch, error);
    }

    if !failed_errors.is_empty() {
        eprintln!("{} failed", failed_errors.len());
    }
}

fn print_normal_cleanup_summary(result: &CleanupResult, remaining: &[BranchInfo]) {
    let output = build_cleanup_summary(result, remaining);
    eprint!("{}", output);
}

fn build_cleanup_summary(result: &CleanupResult, remaining: &[BranchInfo]) -> String {
    let mut output = String::new();

    // Header - center plain text first, then apply color to avoid ANSI code width issues
    let line = "═".repeat(SUMMARY_BOX_WIDTH);
    let centered_title = format!("{:^width$}", "Cleanup Complete", width = SUMMARY_BOX_WIDTH);
    output.push_str(&format!("\n{}\n", line.cyan()));
    output.push_str(&format!("{}\n", centered_title.cyan().bold()));
    output.push_str(&format!("{}\n\n", line.cyan()));

    // Partition results
    let (deleted, rest): (Vec<_>, Vec<_>) = result.deletions.iter().partition(|d| {
        matches!(
            d.outcome,
            DeletionOutcome::Deleted | DeletionOutcome::ForceDeleted
        )
    });
    let (skipped, failed): (Vec<_>, Vec<_>) = rest
        .into_iter()
        .partition(|d| matches!(d.outcome, DeletionOutcome::Skipped { .. }));

    // Deleted section
    if !deleted.is_empty() {
        output.push_str(&format!(
            "{}\n",
            format!("Deleted ({}):", deleted.len()).green().bold()
        ));
        for d in &deleted {
            let suffix = if matches!(d.outcome, DeletionOutcome::ForceDeleted) {
                " (force)"
            } else {
                ""
            };
            output.push_str(&format!(
                "  {} {}{}\n",
                "✓".green(),
                d.branch,
                suffix.dimmed()
            ));
        }
        output.push('\n');
    }

    // Skipped section
    if !skipped.is_empty() {
        output.push_str(&format!(
            "{}\n",
            format!("Skipped ({}):", skipped.len()).yellow().bold()
        ));
        for d in &skipped {
            if let DeletionOutcome::Skipped { reason } = &d.outcome {
                output.push_str(&format!(
                    "  {} {}: {}\n",
                    "○".dimmed(),
                    d.branch,
                    reason.dimmed()
                ));
            }
        }
        output.push('\n');
    }

    // Failed section
    if !failed.is_empty() {
        output.push_str(&format!(
            "{}\n",
            format!("Failed ({}):", failed.len()).red().bold()
        ));
        for d in &failed {
            if let DeletionOutcome::Failed { error } = &d.outcome {
                output.push_str(&format!("  {} {}\n", "✗".red(), d.branch));
                output.push_str(&format!("    {}: {}\n", "Error".red(), error));
                output.push_str(&format!(
                    "    {}: Use 'git branch -D {}' to force delete\n",
                    "Hint".dimmed(),
                    d.branch
                ));
            }
        }
        output.push('\n');
    }

    // Remaining branches
    if !remaining.is_empty() {
        output.push_str(&format!(
            "{}: {}\n",
            "Remaining branches".white().bold(),
            remaining.len()
        ));
        for branch in remaining {
            let suffix = if branch.is_current {
                " (current)".cyan().to_string()
            } else {
                String::new()
            };
            output.push_str(&format!("  - {}{}\n", branch.name, suffix));
        }
    }

    output
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
        colored::control::set_override(false);
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

        let (stdout_line, stderr_lines) = build_quiet_summary(&[success.clone(), failure.clone()]);
        assert_eq!(stdout_line, "1/2 repositories updated");
        assert_eq!(stderr_lines.len(), 1);
        assert!(stderr_lines[0].contains("/test/failure"));

        let (stdout_line, stderr_lines) = build_quiet_summary(&[success]);
        assert_eq!(stdout_line, "1/1 repositories updated");
        assert!(stderr_lines.is_empty());

        let (stdout_line, stderr_lines) = build_quiet_summary(&[]);
        assert_eq!(stdout_line, "0/0 repositories updated");
        assert!(stderr_lines.is_empty());
    }

    #[test]
    fn test_print_summary_quiet_and_normal_modes() {
        colored::control::set_override(false);
        let success = UpdateResult {
            path: PathBuf::from("/test/success"),
            outcome: UpdateOutcome::Success(UpdateSuccess {
                original_head: OriginalHead::Branch("main".to_string()),
                master_branch: "main",
                had_stash: false,
            }),
            duration: Duration::from_secs(1),
        };

        let failure = UpdateResult {
            path: PathBuf::from("/test/failure"),
            outcome: UpdateOutcome::Failed(UpdateFailure {
                error: "test error".to_string(),
                step: UpdateStep::Pulling,
            }),
            duration: Duration::from_millis(200),
        };

        let quiet_config = Config {
            verbosity: crate::config::Verbosity::Quiet,
        };
        let normal_config = Config {
            verbosity: crate::config::Verbosity::Normal,
        };

        let (stdout_line, stderr_lines) = build_quiet_summary(&[success.clone(), failure.clone()]);
        assert_eq!(stdout_line, "1/2 repositories updated");
        assert_eq!(stderr_lines.len(), 1);

        let output =
            build_normal_summary(&[success.clone(), failure.clone()], Duration::from_secs(2));
        assert!(output.contains("Summary"));
        assert!(output.contains("Total"));

        print_summary(
            &[success.clone(), failure.clone()],
            Duration::from_secs(2),
            &quiet_config,
        );
        print_summary(&[success, failure], Duration::from_secs(2), &normal_config);
    }

    #[test]
    fn test_build_normal_summary_success_only() {
        colored::control::set_override(false);
        let success = UpdateResult {
            path: PathBuf::from("/test/success"),
            outcome: UpdateOutcome::Success(UpdateSuccess {
                original_head: OriginalHead::Branch("main".to_string()),
                master_branch: "main",
                had_stash: false,
            }),
            duration: Duration::from_secs(1),
        };

        let output = build_normal_summary(&[success], Duration::from_secs(1));
        assert!(output.contains("Succeeded (1):"));
        assert!(!output.contains("Failed ("));
    }

    #[test]
    fn test_build_normal_summary_failure_only() {
        colored::control::set_override(false);
        let failure = UpdateResult {
            path: PathBuf::from("/test/failure"),
            outcome: UpdateOutcome::Failed(UpdateFailure {
                error: "boom".to_string(),
                step: UpdateStep::Fetching,
            }),
            duration: Duration::from_secs(1),
        };

        let output = build_normal_summary(&[failure], Duration::from_secs(1));
        assert!(output.contains("Failed (1):"));
        assert!(!output.contains("Succeeded ("));
    }

    #[test]
    fn test_build_normal_summary_golden_output() {
        colored::control::set_override(false);
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
                error: "boom".to_string(),
                step: UpdateStep::Fetching,
            }),
            duration: Duration::from_millis(500),
        };

        let output = build_normal_summary(&[success, failure], Duration::from_secs(3));
        let expected = [
            "",
            "==================================================",
            "                     Summary",
            "==================================================",
            "",
            "Succeeded (1):",
            "  OK /test/success [feature]  (stash restored) in 2.00s",
            "",
            "Failed (1):",
            "  FAIL /test/failure at Fetching: boom in 0.50s",
            "",
            "Total: 1/2 repos in 3.00s",
            "",
        ]
        .join("\n");

        assert_eq!(output, expected);
    }

    #[test]
    fn test_build_repo_header_line() {
        colored::control::set_override(false);
        let line = build_repo_header_line("repo-a");
        assert!(line.contains("[repo-a]"));
    }

    #[test]
    fn test_build_step_line() {
        colored::control::set_override(false);
        let line = build_step_line(&UpdateStep::Fetching);
        assert!(line.contains("Fetching"));
        assert!(line.ends_with("..."));
    }

    #[test]
    fn test_build_completion_status_line_variants() {
        colored::control::set_override(false);
        let success_line = build_completion_status_line(true, None).expect("missing line");
        assert!(success_line.contains("completed successfully"));

        let failure_line = build_completion_status_line(false, Some("boom")).expect("missing line");
        assert!(failure_line.contains("failed"));
        assert!(failure_line.contains("boom"));

        let none_line = build_completion_status_line(false, None);
        assert!(none_line.is_none());
    }

    #[test]
    fn test_build_working_dir_line() {
        colored::control::set_override(false);
        let line = build_working_dir_line(Path::new("/tmp/repo"));
        assert!(line.contains("Working in:"));
        assert!(line.contains("/tmp/repo"));
    }

    #[test]
    fn test_build_workspace_start_line() {
        colored::control::set_override(false);
        let line = build_workspace_start_line(3);
        assert!(line.contains("workspace mode"));
        assert!(line.contains("3"));

        let empty_line = build_workspace_start_line(0);
        assert!(empty_line.contains("No git repositories found"));
    }

    #[test]
    fn test_print_repo_header_step_and_completion_verbose_only() {
        colored::control::set_override(false);
        let verbose = Config {
            verbosity: crate::config::Verbosity::Verbose,
        };
        let normal = Config {
            verbosity: crate::config::Verbosity::Normal,
        };

        print_repo_header(&normal, "repo-a");
        print_step(&normal, &UpdateStep::Fetching);
        print_completion_status(&normal, true, None);

        print_repo_header(&verbose, "repo-a");
        print_step(&verbose, &UpdateStep::Fetching);
        print_completion_status(&verbose, true, None);
        print_completion_status(&verbose, false, Some("boom"));
    }

    #[test]
    fn test_print_working_dir_and_workspace_start_quiet_and_normal() {
        colored::control::set_override(false);
        let quiet = Config {
            verbosity: crate::config::Verbosity::Quiet,
        };
        let normal = Config {
            verbosity: crate::config::Verbosity::Normal,
        };

        print_working_dir(Path::new("/tmp/repo"), &quiet);
        print_working_dir(Path::new("/tmp/repo"), &normal);

        print_workspace_start(0, &quiet);
        print_workspace_start(0, &normal);
        print_workspace_start(2, &normal);
    }

    #[test]
    fn test_single_repo_callbacks_finish_and_steps() {
        colored::control::set_override(false);
        let config = Config {
            verbosity: crate::config::Verbosity::Normal,
        };
        let progress = create_single_repo_progress(&config);
        let callbacks = SingleRepoCallbacks::new(progress, config);

        let success = UpdateResult {
            path: PathBuf::from("/test/success"),
            outcome: UpdateOutcome::Success(UpdateSuccess {
                original_head: OriginalHead::Branch("main".to_string()),
                master_branch: "main",
                had_stash: false,
            }),
            duration: Duration::from_secs(1),
        };
        callbacks.on_update_start("repo-a");
        callbacks.on_step(&UpdateStep::Started);
        callbacks.on_step_execute(&UpdateStep::Fetching);
        callbacks.on_completion_status(true, None);
        callbacks.on_complete(&success);
        callbacks.finish(&success);

        let failure = UpdateResult {
            path: PathBuf::from("/test/failure"),
            outcome: UpdateOutcome::Failed(UpdateFailure {
                error: "boom".to_string(),
                step: UpdateStep::Fetching,
            }),
            duration: Duration::from_secs(1),
        };
        callbacks.on_completion_status(false, Some("boom"));
        callbacks.on_complete(&failure);
        callbacks.finish(&failure);
    }

    #[test]
    fn test_workspace_progress_tracker_and_capacity() {
        colored::control::set_override(false);
        let config = Config {
            verbosity: crate::config::Verbosity::Normal,
        };
        let progress = create_workspace_progress(MAX_VISIBLE_COMPLETIONS + 2, &config);
        let tracker = progress.create_repo_tracker("repo-a", config);

        for i in 0..(MAX_VISIBLE_COMPLETIONS + 2) {
            let result = UpdateResult {
                path: PathBuf::from(format!("/tmp/repo-{}", i)),
                outcome: UpdateOutcome::Success(UpdateSuccess {
                    original_head: OriginalHead::Branch("main".to_string()),
                    master_branch: "main",
                    had_stash: false,
                }),
                duration: Duration::from_secs(1),
            };
            tracker.on_complete(&result);
        }

        {
            let state = progress
                .state
                .lock()
                .expect("WorkspaceProgress state mutex poisoned");
            assert_eq!(state.repos.len(), MAX_VISIBLE_COMPLETIONS);
        }

        progress.finish();
    }

    #[test]
    fn test_workspace_progress_hidden_when_quiet() {
        colored::control::set_override(false);
        let quiet = Config {
            verbosity: crate::config::Verbosity::Quiet,
        };
        let progress = create_workspace_progress(1, &quiet);
        let tracker = progress.create_repo_tracker("repo-a", quiet);

        let result = UpdateResult {
            path: PathBuf::from("/tmp/repo"),
            outcome: UpdateOutcome::Success(UpdateSuccess {
                original_head: OriginalHead::Branch("main".to_string()),
                master_branch: "main",
                had_stash: false,
            }),
            duration: Duration::from_secs(1),
        };
        tracker.on_complete(&result);
        progress.finish();
    }

    #[test]
    fn test_workspace_progress_mark_completed_smoke() {
        let config = Config {
            verbosity: crate::config::Verbosity::Normal,
        };
        let progress = create_workspace_progress(2, &config);
        progress.mark_completed("repo-a", true);
        progress.mark_completed("repo-b", false);
        progress.finish();
    }

    #[test]
    fn test_single_repo_progress_smoke() {
        let normal_config = Config {
            verbosity: crate::config::Verbosity::Normal,
        };
        let quiet_config = Config {
            verbosity: crate::config::Verbosity::Quiet,
        };

        let normal_progress = create_single_repo_progress(&normal_config);
        normal_progress.update(&UpdateStep::Fetching);
        normal_progress.finish_success("repo-a");
        normal_progress.finish_failed("repo-a", "error");

        let quiet_progress = create_single_repo_progress(&quiet_config);
        quiet_progress.update(&UpdateStep::Fetching);
        quiet_progress.finish_success("repo-b");
        quiet_progress.finish_failed("repo-b", "error");
    }

    use crate::cleanup::{
        BranchInfo, CleanupResult, DeletionOutcome, DeletionResult, MergeStatus, TrackingStatus,
    };

    fn make_branch(
        name: &str,
        is_current: bool,
        merge: MergeStatus,
        tracking: TrackingStatus,
    ) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
            is_current,
            merge_status: merge,
            tracking_status: tracking,
        }
    }

    #[test]
    fn test_column_widths_from_branches() {
        let branches = vec![
            make_branch(
                "short",
                false,
                MergeStatus::Merged,
                TrackingStatus::RemoteGone,
            ),
            make_branch(
                "much-longer-branch-name",
                false,
                MergeStatus::Unmerged,
                TrackingStatus::NoUpstream,
            ),
        ];

        let widths = BranchListWidths::from_branches(&branches);
        assert_eq!(widths.name, 23); // "much-longer-branch-name"
        assert_eq!(widths.status, 16); // "(current branch)" constant
        assert_eq!(widths.remote, 7); // "unknown" constant
    }

    #[test]
    fn test_column_widths_minimum_enforced() {
        let branches = vec![make_branch(
            "a",
            false,
            MergeStatus::Merged,
            TrackingStatus::RemoteGone,
        )];

        let widths = BranchListWidths::from_branches(&branches);
        assert!(widths.name >= 6); // MIN_NAME_WIDTH
    }

    #[test]
    fn test_column_widths_maximum_enforced() {
        // 70-character branch name exceeds MAX_NAME_WIDTH of 60
        let long_name = "a".repeat(70);
        let branches = vec![make_branch(
            &long_name,
            false,
            MergeStatus::Merged,
            TrackingStatus::RemoteGone,
        )];

        let widths = BranchListWidths::from_branches(&branches);
        assert_eq!(widths.name, 60); // MAX_NAME_WIDTH
    }

    #[test]
    fn test_column_widths_empty_branches() {
        let widths = BranchListWidths::from_branches(&[]);
        assert!(widths.name >= 6);
    }

    #[test]
    fn test_format_branch_line_merged_gone() {
        colored::control::set_override(false);
        let branch = make_branch(
            "feature/auth",
            false,
            MergeStatus::Merged,
            TrackingStatus::RemoteGone,
        );
        let widths = BranchListWidths::from_branches(&[branch.clone()]);

        let line = format_branch_line(&branch, &widths);
        assert!(line.contains("feature/auth"));
        assert!(line.contains("merged"));
        assert!(line.contains("gone"));
        assert!(line.starts_with(" ")); // not current
    }

    #[test]
    fn test_format_branch_line_current_branch() {
        colored::control::set_override(false);
        let branch = make_branch(
            "feature/current",
            true,
            MergeStatus::Unmerged,
            TrackingStatus::RemoteExists("origin/feature/current".to_string()),
        );
        let widths = BranchListWidths::from_branches(&[branch.clone()]);

        let line = format_branch_line(&branch, &widths);
        assert!(line.starts_with("*")); // current branch marker
        assert!(line.contains("(current branch)"));
        assert!(line.contains("exists"));
    }

    #[test]
    fn test_format_branch_line_unmerged_shows_warning() {
        colored::control::set_override(false);
        let branch = make_branch(
            "experiment",
            false,
            MergeStatus::Unmerged,
            TrackingStatus::RemoteGone,
        );
        let widths = BranchListWidths::from_branches(&[branch.clone()]);

        let line = format_branch_line(&branch, &widths);
        assert!(line.contains("unmerged"));
    }

    #[test]
    fn test_format_branch_line_unclear_shows_squash_hint() {
        colored::control::set_override(false);
        let branch = make_branch(
            "old-feature",
            false,
            MergeStatus::Unclear,
            TrackingStatus::NoUpstream,
        );
        let widths = BranchListWidths::from_branches(&[branch.clone()]);

        let line = format_branch_line(&branch, &widths);
        assert!(line.contains("unclear"));
        assert!(line.contains("may be squash-merged"));
    }

    #[test]
    fn test_build_branch_list_header() {
        colored::control::set_override(false);
        let branches = vec![make_branch(
            "feature/test",
            false,
            MergeStatus::Merged,
            TrackingStatus::RemoteGone,
        )];
        let widths = BranchListWidths::from_branches(&branches);

        let header = build_branch_list_header(&widths);
        assert!(header.contains("BRANCH"));
        assert!(header.contains("STATUS"));
        assert!(header.contains("REMOTE"));
        assert!(header.contains("─")); // separator line
    }

    #[test]
    fn test_build_deletion_result_line_deleted() {
        colored::control::set_override(false);
        let result = DeletionResult {
            branch: "feature/done".to_string(),
            outcome: DeletionOutcome::Deleted,
        };

        let line = build_deletion_result_line(&result);
        assert!(line.contains("✓"));
        assert!(line.contains("feature/done"));
    }

    #[test]
    fn test_build_deletion_result_line_force_deleted() {
        colored::control::set_override(false);
        let result = DeletionResult {
            branch: "experiment".to_string(),
            outcome: DeletionOutcome::ForceDeleted,
        };

        let line = build_deletion_result_line(&result);
        assert!(line.contains("✓"));
        assert!(line.contains("experiment"));
        assert!(line.contains("(force)"));
    }

    #[test]
    fn test_build_deletion_result_line_skipped() {
        colored::control::set_override(false);
        let result = DeletionResult {
            branch: "feature/current".to_string(),
            outcome: DeletionOutcome::Skipped {
                reason: "current branch".to_string(),
            },
        };

        let line = build_deletion_result_line(&result);
        assert!(line.contains("○"));
        assert!(line.contains("feature/current"));
        assert!(line.contains("current branch"));
    }

    #[test]
    fn test_build_deletion_result_line_failed() {
        colored::control::set_override(false);
        let result = DeletionResult {
            branch: "broken".to_string(),
            outcome: DeletionOutcome::Failed {
                error: "branch not fully merged".to_string(),
            },
        };

        let line = build_deletion_result_line(&result);
        assert!(line.contains("✗"));
        assert!(line.contains("broken"));
        assert!(line.contains("branch not fully merged"));
    }

    #[test]
    fn test_build_cleanup_summary_all_deleted() {
        colored::control::set_override(false);
        let result = CleanupResult {
            main_branch: "master".to_string(),
            deletions: vec![
                DeletionResult {
                    branch: "feature/a".to_string(),
                    outcome: DeletionOutcome::Deleted,
                },
                DeletionResult {
                    branch: "feature/b".to_string(),
                    outcome: DeletionOutcome::ForceDeleted,
                },
            ],
            switched_from: None,
        };
        let remaining = vec![make_branch(
            "master",
            false,
            MergeStatus::Merged,
            TrackingStatus::RemoteExists("origin/master".to_string()),
        )];

        let output = build_cleanup_summary(&result, &remaining);
        assert!(output.contains("Cleanup Complete"));
        assert!(output.contains("Deleted (2):"));
        assert!(output.contains("feature/a"));
        assert!(output.contains("feature/b"));
        assert!(output.contains("(force)"));
        assert!(output.contains("Remaining branches"));
        assert!(output.contains("master"));
    }

    #[test]
    fn test_build_cleanup_summary_with_failures() {
        colored::control::set_override(false);
        let result = CleanupResult {
            main_branch: "main".to_string(),
            deletions: vec![
                DeletionResult {
                    branch: "good".to_string(),
                    outcome: DeletionOutcome::Deleted,
                },
                DeletionResult {
                    branch: "bad".to_string(),
                    outcome: DeletionOutcome::Failed {
                        error: "not fully merged".to_string(),
                    },
                },
            ],
            switched_from: None,
        };

        let output = build_cleanup_summary(&result, &[]);
        assert!(output.contains("Deleted (1):"));
        assert!(output.contains("Failed (1):"));
        assert!(output.contains("bad"));
        assert!(output.contains("not fully merged"));
        assert!(output.contains("Hint"));
        assert!(output.contains("git branch -D bad"));
    }

    #[test]
    fn test_build_cleanup_summary_with_skipped() {
        colored::control::set_override(false);
        let result = CleanupResult {
            main_branch: "master".to_string(),
            deletions: vec![DeletionResult {
                branch: "current-feature".to_string(),
                outcome: DeletionOutcome::Skipped {
                    reason: "current branch".to_string(),
                },
            }],
            switched_from: None,
        };

        let output = build_cleanup_summary(&result, &[]);
        assert!(output.contains("Skipped (1):"));
        assert!(output.contains("current-feature"));
        assert!(output.contains("current branch"));
    }

    #[test]
    fn test_build_cleanup_summary_remaining_with_current() {
        colored::control::set_override(false);
        let result = CleanupResult {
            main_branch: "master".to_string(),
            deletions: vec![],
            switched_from: None,
        };
        let remaining = vec![
            make_branch(
                "master",
                false,
                MergeStatus::Merged,
                TrackingStatus::RemoteExists("origin/master".to_string()),
            ),
            make_branch(
                "feature/wip",
                true,
                MergeStatus::Unmerged,
                TrackingStatus::NoUpstream,
            ),
        ];

        let output = build_cleanup_summary(&result, &remaining);
        assert!(output.contains("Remaining branches: 2"));
        assert!(output.contains("master"));
        assert!(output.contains("feature/wip"));
        assert!(output.contains("(current)"));
    }

    #[test]
    fn test_format_merge_status_colors() {
        colored::control::set_override(false);
        assert_eq!(format_merge_status(&MergeStatus::Merged), "merged");
        assert_eq!(format_merge_status(&MergeStatus::SquashMerged), "merged");
        assert_eq!(format_merge_status(&MergeStatus::Unmerged), "unmerged");
        assert_eq!(format_merge_status(&MergeStatus::Unclear), "unclear");
    }

    #[test]
    fn test_format_tracking_status() {
        colored::control::set_override(false);
        assert_eq!(format_tracking_status(&TrackingStatus::RemoteGone), "gone");
        assert_eq!(
            format_tracking_status(&TrackingStatus::RemoteExists("origin/foo".to_string())),
            "exists"
        );
        assert_eq!(format_tracking_status(&TrackingStatus::NoUpstream), "local");
        assert_eq!(format_tracking_status(&TrackingStatus::Unknown), "unknown");
    }

    #[test]
    fn test_format_branch_warning() {
        let unclear = make_branch(
            "unclear-branch",
            false,
            MergeStatus::Unclear,
            TrackingStatus::NoUpstream,
        );
        let warning = format_branch_warning(&unclear);
        assert!(warning.contains("may be squash-merged"));

        let unmerged = make_branch(
            "unmerged-branch",
            false,
            MergeStatus::Unmerged,
            TrackingStatus::NoUpstream,
        );
        let warning = format_branch_warning(&unmerged);
        assert!(warning.contains("unmerged"));

        let merged = make_branch(
            "merged-branch",
            false,
            MergeStatus::Merged,
            TrackingStatus::NoUpstream,
        );
        let warning = format_branch_warning(&merged);
        assert!(warning.is_empty());

        let current = make_branch(
            "current",
            true,
            MergeStatus::Unmerged,
            TrackingStatus::NoUpstream,
        );
        let warning = format_branch_warning(&current);
        assert!(warning.is_empty()); // current branches don't get warnings
    }

    #[test]
    fn test_print_functions_respect_quiet_mode() {
        let quiet = Config {
            verbosity: crate::config::Verbosity::Quiet,
        };
        let branches = vec![make_branch(
            "test",
            false,
            MergeStatus::Merged,
            TrackingStatus::RemoteGone,
        )];
        let widths = BranchListWidths::from_branches(&branches);

        // These should not panic and should produce no output in quiet mode
        print_branch_list_header(&widths, &quiet);
        print_branch_list(&branches, &quiet);
        print_analyzing_branches(&quiet);
        print_no_branches_to_clean(&quiet);
        print_deleting_header(&quiet);
    }

    #[test]
    fn test_print_functions_normal_mode_smoke() {
        let normal = Config {
            verbosity: crate::config::Verbosity::Normal,
        };
        let branches = vec![make_branch(
            "feature",
            false,
            MergeStatus::Merged,
            TrackingStatus::RemoteGone,
        )];
        let widths = BranchListWidths::from_branches(&branches);
        let result = DeletionResult {
            branch: "feature".to_string(),
            outcome: DeletionOutcome::Deleted,
        };
        let cleanup_result = CleanupResult {
            main_branch: "master".to_string(),
            deletions: vec![result.clone()],
            switched_from: None,
        };

        // Smoke tests - should not panic
        print_branch_list_header(&widths, &normal);
        print_branch_list(&branches, &normal);
        print_deletion_result(&result, &normal);
        print_cleanup_summary(&cleanup_result, &[], &normal);
        print_analyzing_branches(&normal);
        print_no_branches_to_clean(&normal);
        print_deleting_header(&normal);
    }

    #[test]
    fn test_format_branch_line_public_api() {
        colored::control::set_override(false);
        let branch = make_branch(
            "api-test",
            false,
            MergeStatus::Merged,
            TrackingStatus::RemoteGone,
        );
        let widths = BranchListWidths::from_branches(&[branch.clone()]);

        let line = format_branch_line(&branch, &widths);
        assert!(line.contains("api-test"));
    }

    // Color-enabled tests to verify ANSI codes are emitted
    #[test]
    fn test_format_merge_status_emits_green_for_safe() {
        colored::control::set_override(true);
        let result = format_merge_status(&MergeStatus::Merged);
        assert!(result.contains("\x1b[32m"), "Expected green ANSI code");

        let result = format_merge_status(&MergeStatus::SquashMerged);
        assert!(result.contains("\x1b[32m"), "Expected green ANSI code");
    }

    #[test]
    fn test_format_merge_status_emits_yellow_for_caution() {
        colored::control::set_override(true);
        let result = format_merge_status(&MergeStatus::Unmerged);
        assert!(result.contains("\x1b[33m"), "Expected yellow ANSI code");
    }

    #[test]
    fn test_format_merge_status_emits_magenta_for_uncertain() {
        colored::control::set_override(true);
        let result = format_merge_status(&MergeStatus::Unclear);
        assert!(result.contains("\x1b[35m"), "Expected magenta ANSI code");
    }

    #[test]
    fn test_format_tracking_status_emits_dim_for_inactive() {
        colored::control::set_override(true);
        let result = format_tracking_status(&TrackingStatus::RemoteGone);
        assert!(result.contains("\x1b[2m"), "Expected dim ANSI code");

        let result = format_tracking_status(&TrackingStatus::NoUpstream);
        assert!(result.contains("\x1b[2m"), "Expected dim ANSI code");
    }

    #[test]
    fn test_format_tracking_status_no_style_for_active() {
        colored::control::set_override(true);
        let result = format_tracking_status(&TrackingStatus::RemoteExists("origin/foo".into()));
        // Active status should not have ANSI codes
        assert!(
            !result.contains("\x1b["),
            "Expected no ANSI codes for active remote"
        );
    }

    // Invariant tests for BranchListWidths
    #[test]
    fn test_branch_list_widths_invariant_min_bound() {
        // Test with various branch name lengths
        for len in 0..=10 {
            let name = "x".repeat(len);
            let branches = vec![make_branch(
                &name,
                false,
                MergeStatus::Merged,
                TrackingStatus::RemoteGone,
            )];
            let widths = BranchListWidths::from_branches(&branches);
            assert!(
                widths.name >= BranchListWidths::MIN_NAME_WIDTH,
                "Width {} should be >= MIN {}",
                widths.name,
                BranchListWidths::MIN_NAME_WIDTH
            );
        }
    }

    #[test]
    fn test_branch_list_widths_invariant_max_bound() {
        // Test with very long branch names
        for len in [50, 60, 70, 100, 200] {
            let name = "x".repeat(len);
            let branches = vec![make_branch(
                &name,
                false,
                MergeStatus::Merged,
                TrackingStatus::RemoteGone,
            )];
            let widths = BranchListWidths::from_branches(&branches);
            assert!(
                widths.name <= BranchListWidths::MAX_NAME_WIDTH,
                "Width {} should be <= MAX {}",
                widths.name,
                BranchListWidths::MAX_NAME_WIDTH
            );
        }
    }

    #[test]
    fn test_branch_list_widths_tracks_longest_branch() {
        let branches = vec![
            make_branch(
                "short",
                false,
                MergeStatus::Merged,
                TrackingStatus::RemoteGone,
            ),
            make_branch(
                "medium-name",
                false,
                MergeStatus::Merged,
                TrackingStatus::RemoteGone,
            ),
            make_branch(
                "this-is-a-longer-name",
                false,
                MergeStatus::Merged,
                TrackingStatus::RemoteGone,
            ),
        ];
        let widths = BranchListWidths::from_branches(&branches);
        assert_eq!(widths.name, "this-is-a-longer-name".len());
    }

    // Semantic color mapping consistency tests
    #[test]
    fn test_color_mapping_matches_semantic_methods() {
        colored::control::set_override(true);

        // Safe statuses should be green
        for status in [MergeStatus::Merged, MergeStatus::SquashMerged] {
            assert!(status.is_safely_deletable());
            let formatted = format_merge_status(&status);
            assert!(
                formatted.contains("\x1b[32m"),
                "Safe status should be green"
            );
        }

        // Caution status should be yellow
        let status = MergeStatus::Unmerged;
        assert!(status.requires_caution());
        let formatted = format_merge_status(&status);
        assert!(
            formatted.contains("\x1b[33m"),
            "Caution status should be yellow"
        );

        // Uncertain status should be magenta
        let status = MergeStatus::Unclear;
        assert!(status.is_uncertain());
        let formatted = format_merge_status(&status);
        assert!(
            formatted.contains("\x1b[35m"),
            "Uncertain status should be magenta"
        );
    }
}
