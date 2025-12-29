use crate::repo::{UpdateOutcome, UpdateResult};
use std::path::Path;
use std::time::Duration;
use colored::Colorize;

// Progress bars, colored output, summary formatting

pub fn print_working_dir(path: &Path) {
    println!("{} {}", "Working in:".cyan(), path.display().to_string().white().bold())
}

fn print_no_repos() {
    println!("{}", "No git repositories found".yellow().bold())
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

    println!("{}", format!("Succeeded ({}):", successes.len()).green().bold());

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

// TODO: create_repo_progress()
// TODO: create_workspace_progress(count)
// TODO: update_progress(pb, step)
