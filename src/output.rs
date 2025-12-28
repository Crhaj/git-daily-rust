use std::path::Path;

// Progress bars, colored output, summary formatting

pub fn print_working_dir(path: &Path) {
    println!("Working in: {}", path.display())
}
// TODO: create_repo_progress()
// TODO: create_workspace_progress(count)
// TODO: update_progress(pb, step)
// TODO: print_summary(results, duration)
// TODO: print_no_repos()
// TODO: print_workspace_start(count)
