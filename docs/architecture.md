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

- Print working directory at start
- Parse CLI args with clap (`-v` for verbose)
- Detect if current directory is a git repo
- If yes: run single repo update with step-by-step progress bar
- If no: discover repos, update in parallel with repo-count progress bar
- Call `output::print_summary()` at the end
- Exit with code 1 if any failures

### `output.rs`

- `print_working_dir(path)` - prints "Working in: /path"
- `create_repo_progress()` - progress bar for single repo (7 steps)
- `create_workspace_progress(count)` - progress bar for workspace
- `update_progress(pb, step)` - updates progress bar based on `UpdateStep`
- `print_summary(results, duration)` - colored summary
- `print_no_repos()` - yellow warning when no repos found
- `print_workspace_start(count)` - "Found N repositories"

### `git.rs`

- Thin wrappers around `git` binary via `std::process::Command`
- Functions: `current_branch()`, `has_uncommitted_changes()`, `stash()`, `stash_pop()`, `checkout()`, `fetch_prune()`
- Returns `anyhow::Result`

### `repo.rs`

- `is_git_repo(path) -> bool`
- `update(path, on_step) -> UpdateResult` - orchestrates update with progress callback
- Types: `UpdateResult`, `UpdateOutcome`, `UpdateStep`

## Core Types

Defined in `repo.rs`:

```rust
#[derive(Debug, Clone)]
pub enum UpdateStep {
    Started,
    DetectingBranch,
    CheckingChanges,
    Stashing,
    CheckingOut { branch: String },
    Fetching,
    RestoringBranch { branch: String },
    PoppingStash,
    Completed,
}

#[derive(Debug)]
pub struct UpdateResult {
    pub path: PathBuf,
    pub outcome: UpdateOutcome,
    pub duration: Duration,
}

#[derive(Debug)]
pub enum UpdateOutcome {
    Success {
        original_branch: String,
        main_branch: String,
        had_stash: bool,
    },
    Failed {
        error: String,
        step: UpdateStep,
    },
}
```

## Progress Callback Pattern

Closure-based for simplicity:

```rust
pub fn update<F>(path: &Path, on_step: F) -> UpdateResult
where
    F: Fn(UpdateStep),
{
    on_step(UpdateStep::DetectingBranch);
    // ... work ...
}
```

Usage:

```rust
// Single repo - with progress bar
let pb = output::create_repo_progress();
repo::update(&path, |step| output::update_progress(&pb, &step));

// Workspace mode - no per-step progress
repo::update(&path, |_| {});
```

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
4. Stashing -> git stash push (if needed)
5. CheckingOut -> git checkout master (fallback to main)
6. Fetching -> git fetch --prune --all
7. RestoringBranch -> git checkout original-branch
8. PoppingStash -> git stash pop (if needed)
9. Completed
```

On failure: attempt to restore original state, record failure with step info.

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

Found 12 repositories

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
pub struct TestRepo {
    _temp: TempDir,
    pub path: PathBuf,
}

impl TestRepo {
    pub fn new() -> Self { /* git init in temp dir */ }
    pub fn with_remote() -> (Self, Self) { /* origin + clone */ }
    pub fn current_branch(&self) -> String { ... }
    pub fn create_branch(&self, name: &str) { ... }
    pub fn checkout(&self, branch: &str) { ... }
    pub fn make_dirty(&self) { ... }
    pub fn has_stash(&self) -> bool { ... }
}
```

### Core Tests

| Test | What it verifies |
|------|------------------|
| `updates_repo_and_returns_to_original_branch` | Happy path |
| `stashes_and_restores_uncommitted_changes` | Stash/restore flow |
| `falls_back_to_main_when_no_master` | Main branch detection |
| `reports_failure_when_no_remote` | Error handling |
| `handles_already_on_main_branch` | Edge case |
| `restores_branch_on_fetch_failure` | Error recovery |

### What NOT to Test

- `git.rs` in isolation (thin wrappers)
- Output formatting (verify by eye)
- That dependencies work
- 100% coverage

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Git interaction | Shell out to `git` binary | Matches user's git config, easier debugging |
| Parallelism | `rayon` | Simple `.par_iter()`, not async overhead |
| Progress callback | Closure, not trait | Simpler: `\|_\| {}` vs NoOpReporter |
| Error handling | `anyhow` | CLI tool - good messages, not typed errors |
| File structure | 5 source files | lib.rs needed for test access |
| Types location | In `repo.rs` | <40 lines, tightly coupled to logic |
| Testing | Integration with real git | No mocking complexity, catches real bugs |

## Future Extensions (Not Implemented Now)

- `--verbose` flag to show git commands
- `--quiet` flag to suppress output
- Config file for branch order, excluded dirs
- Subcommands (`git-daily-v2 status`, `git-daily-v2 config`)
- Max parallelism control
