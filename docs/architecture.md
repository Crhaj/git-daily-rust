# git-daily-rust: Architecture Design

## Purpose

A CLI tool that updates git repositories by stashing changes, switching to the main branch, fetching with prune, and restoring the original state. When run in a non-git directory, it performs this operation on all git repositories in subdirectories (in parallel).

## Project Structure

```
src/
├── main.rs      # Entry point, imports from lib, orchestration
├── lib.rs       # Exports modules for binary and tests
├── output.rs    # Progress bars, colored output, summary formatting
├── git.rs       # Thin wrappers around git binary commands
└── repo.rs      # Repository detection, update logic, result types

tests/
├── common/
│   └── mod.rs   # TestRepo helper for creating temp git repos
└── integration_test.rs  # Integration tests
```

**5 source files + test infrastructure.**

## Dependencies

```toml
[dependencies]
anyhow = "1"           # Error handling with context
rayon = "1"            # Parallel iteration
colored = "2"          # Colored terminal output
indicatif = "0.17"     # Progress bars
clap = { version = "4", features = ["derive"] }  # CLI args

[dev-dependencies]
tempfile = "3"         # Temp directories for tests
```

## Module Responsibilities

### `lib.rs`

Exports modules for use by `main.rs` and integration tests:

```rust
pub mod git;
pub mod repo;
pub mod output;
```

### `main.rs`

- Print a working directory at the start
- Parse CLI args with clap (`-v` for verbose)
- Detect if the current directory is a git repo
- If yes: run a single repo update with a step-by-step progress bar
- If no: discover repos, update in parallel with the repo-count progress bar
- Call `output::print_summary()` at the end
- Exit with code 1 if any failures

### `output.rs`

- `print_working_dir(path)` - prints "Working in: /path"
- `create_single_repo_progress()` - progress bar for single repo
- `create_workspace_progress(count)` - progress bar for workspace
- `print_summary(results, duration)` - colored summary
- `print_workspace_start(count)` - "Found N repositories"

### `git.rs`

- Thin wrappers around `git` binary via `std::process::Command`
- Functions: `run_git()`, `get_current_branch()`, `has_uncommitted_changes()`, `stash()`, `stash_pop()`, `checkout()`, `fetch_prune()`
- Returns `anyhow::Result`

### `repo.rs`

- `is_git_repo(path) -> bool`
- `update(path, on_step) -> UpdateResult` - orchestrates update with progress callback
- `update_workspace(repos, make_callbacks) -> Vec<UpdateResult>` - parallel update with per-repo callbacks
- `update_workspace_with(repos, callbacks) -> Vec<UpdateResult>` - parallel update with shared cloneable callbacks
- Types: `UpdateResult`, `UpdateOutcome`, `UpdateStep`
- Traits: `UpdateCallbacks` - trait for progress callbacks (zero-cost abstraction)
- Helpers: `NoOpCallbacks` - default no-op implementation

## Core Types

Defined in `repo.rs`:

```rust
/// Progress callback trait - zero-cost abstraction for update notifications.
/// Implement this to receive step-by-step progress and completion events.
pub trait UpdateCallbacks: Send + Sync {
    fn on_step(&self, step: &UpdateStep);
    fn on_complete(&self, result: &UpdateResult);
}

/// Default no-op callbacks for when progress tracking is not needed.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOpCallbacks;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStep {
    Started,
    DetectingBranch,
    CheckingChanges,
    Stashing,
    CheckingOut,
    Fetching,
    RestoringBranch,
    PoppingStash,
    Completed,
}

#[derive(Debug, Clone)]
pub struct UpdateResult {
    pub path: PathBuf,
    pub outcome: UpdateOutcome,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub enum UpdateOutcome {
    Success(UpdateSuccess),
    Failed(UpdateFailure),
}

#[derive(Debug, Clone)]
pub struct UpdateSuccess {
    pub original_branch: String,
    pub master_branch: String,
    pub had_stash: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateFailure {
    pub error: String,
    pub step: UpdateStep,
}
```

## Progress Callback Pattern

Two patterns are supported:

### Single Repository (closure-based)

```rust
pub fn update<F>(path: &Path, on_step: F) -> UpdateResult
where
    F: Fn(&UpdateStep),
{
    on_step(&UpdateStep::DetectingBranch);
    // ... work ...
}

// Usage with progress bar
let progress = output::create_single_repo_progress();
repo::update(&path, |step| progress.update(step));
```

### Workspace Mode (trait-based, zero-cost abstraction)

```rust
// With per-repo callbacks factory
repo::update_workspace(&repos, |path| {
    workspace_progress.create_repo_tracker(repo_name)
});

// With shared cloneable callbacks
repo::update_workspace_with(&repos, NoOpCallbacks);

// With custom callbacks
struct MyCallbacks { /* ... */ }
impl UpdateCallbacks for MyCallbacks { /* ... */ }
repo::update_workspace_with(&repos, MyCallbacks::new());
```

The trait-based approach provides zero-cost abstraction through monomorphization
when callbacks are inlined, avoiding heap allocations and dynamic dispatch.

## Execution Flow

```
┌─────────────────────────────────────────────────────────────┐
│  git-daily-v2                                               │
│  Print: "Working in: /path/to/dir"                          │
└─────────────────────────────────────────────────────────────┘
                           │
                           ▼
              ┌────────────────────────┐
              │  Is current dir a      │
              │  git repository?       │
              └────────────────────────┘
                    │           │
                   YES          NO
                    │           │
                    ▼           ▼
          ┌──────────────┐  ┌──────────────────────────┐
          │ Single repo  │  │ Find all git repos in    │
          │ Step progress│  │ subdirectories           │
          │ [====..] 4/7 │  │ "Found 12 repositories"  │
          └──────────────┘  └──────────────────────────┘
                    │           │
                    │           ▼
                    │   ┌──────────────────────────┐
                    │   │ Update repos in parallel │
                    │   │ (rayon .par_iter())      │
                    │   │ [====...] 8/12 repos     │
                    │   └──────────────────────────┘
                    │           │
                    └─────┬─────┘
                          ▼
              ┌────────────────────────┐
              │ Print colored summary  │
              │ OK project-a [main]    │
              │ FAIL project-b: error  │
              │ Total: 11/12 in 3.2s   │
              └────────────────────────┘
```

## Update Sequence Per Repo

```
1. Started
2. DetectingBranch -> get current branch
3. CheckingChanges -> check for uncommitted changes
4. Stashing -> git stash (if needed)
5. CheckingOut -> git checkout master (fallback to main)
6. Fetching -> git fetch --prune
7. RestoringBranch -> git checkout original-branch
8. PoppingStash -> git stash pop (if needed)
9. Completed
```

On failure: exit immediately, record failure with step and error info. No automatic state restoration is attempted – this avoids compounding errors and lets the user resolve issues (like stash pop conflicts) manually with full context.

## CLI Interface

```
git-daily-v2          # Update current repo or all repos in subdirectories
git-daily-v2 -v       # Verbose mode - show git commands being run
```

## Output Examples

### Single repo

```
Working in: /Users/jan/projects/my-app

[============================..] 6/7 Fetching...

==================================================
Summary
==================================================

Succeeded (1):
  OK my-app [feature-branch] (stash restored)

Total: 1/1 repos in 1.2s
```

### Workspace

```
Working in: /Users/jan/projects

Starting in workspace mode with 12 repositories

[================..............] 8/12 repos Completed: project-h

==================================================
Summary
==================================================

Succeeded (10):
  OK project-a [main]
  OK project-b [develop]
  ...

Failed (2):
  FAIL project-k at Checkout: could not checkout master or main
  FAIL project-l at Fetch: unable to access remote

Total: 10/12 repos in 4.3s
```

## Testing Strategy

### Approach

Integration tests with real git repos in temp directories. No mocking.

### Test Helper (`tests/common/mod.rs`)

```rust
/// Initializes a git repo at a given path (for workspace tests)
pub fn init_repo(path: &Path, branch: &str) -> Result<()> { ... }

pub struct TestRepo {
    _temp: TempDir,
    pub path: PathBuf,
}

impl TestRepo {
    pub fn new() -> Result<Self> { /* git init in temp dir */ }
    pub fn with_remote(branch: Option<&str>) -> Result<(Self, TempDir)> { /* origin + push */ }
    pub fn create_branch(&self, name: &str) -> Result<()> { ... }
    pub fn make_dirty(&self) -> Result<()> { ... }
    pub fn make_untracked(&self) -> Result<()> { ... }
    pub fn has_stash(&self) -> Result<bool> { ... }
    pub fn file_exists(&self, name: &str) -> bool { ... }
}
```

### Core Tests

| Test                                          | What it verifies                    |
|-----------------------------------------------|-------------------------------------|
| `updates_repo_and_returns_to_original_branch` | Happy path                          |
| `stashes_and_restores_uncommitted_changes`    | Stash/restore flow                  |
| `falls_back_to_main_when_no_master`           | Main branch detection               |
| `reports_failure_when_no_remote`              | Error handling                      |
| `handles_already_on_main_branch`              | Edge case                           |
| `update_workspace_updates_multiple_repos`     | Basic workspace mode                |
| `workspace_mixed_success_and_failure`         | Partial failures in workspace       |
| `workspace_with_dirty_repos`                  | Stash handling across repos         |
| `workspace_callbacks_called_for_each_repo`    | Callback invocation correctness     |
| `workspace_repos_on_different_branches`       | Branch restoration per repo         |
| `workspace_empty_directory`                   | Empty workspace handling            |
| `workspace_nested_repos_not_discovered`       | Only immediate subdirs scanned      |
| `workspace_order_independence`                | Parallel execution consistency      |
| `workspace_with_untracked_files_only`         | Untracked files don't cause stash   |

### What NOT to Test

- `git.rs` in isolation (thin wrappers)
- Output formatting (verified by eye)
- That dependencies work
- 100% coverage

## Design Decisions

| Decision          | Choice                    | Rationale                                   |
|-------------------|---------------------------|---------------------------------------------|
| Git interaction   | Shell out to `git` binary | Matches user's git config, easier debugging |
| Parallelism       | `rayon`                   | Simple `.par_iter()`, not async overhead    |
| Single-repo callback | Closure-based          | Simpler for single use: `\|step\| { ... }`  |
| Workspace callback | Trait-based (`UpdateCallbacks`) | Zero-cost abstraction, no heap alloc |
| Error handling    | `anyhow`                  | CLI tool - good messages, not typed errors  |
| File structure    | 5 source files            | lib.rs needed for test access               |
| Types location    | In `repo.rs`              | <40 lines, tightly coupled to logic         |
| Testing           | Integration with real git | No mocking complexity, catches real bugs    |

## Future Extensions (Not Implemented Now)

- `--verbose` flag to show git commands
- `--quiet` flag to suppress output
- Config file for branch order, excluded dirs
- Subcommands (`git-daily-v2 status`, `git-daily-v2 config`)
- Max parallelism control
