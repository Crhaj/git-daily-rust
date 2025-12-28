use std::path::Path;

// Progress bars, colored output, summary formatting

pub fn print_working_dir(path: &Path) {
    println!("Working in: {}", path.display())
}

fn print_no_repos() {
    println!("No git repositories found")
}

pub fn print_workspace_start(count: usize) {
    if count == 0 {
        print_no_repos()
    } else {
        println!("Starting in workspace mode with {} repositories", count)
    }
}

// TODO: create_repo_progress()
// TODO: create_workspace_progress(count)
// TODO: update_progress(pb, step)
// TODO: print_summary(results, duration)
