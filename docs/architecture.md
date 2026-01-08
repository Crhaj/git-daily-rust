# git-daily-rust: Architecture Design

## Purpose

A CLI tool that updates git repositories by stashing changes, switching to the main branch, fetching with prune, and
restoring the original state. When run in a non-git directory, it performs this operation on all git repositories in
subdirectories (in parallel).

## Project Structure

```
src/
├── main.rs      # Entry point, CLI parsing, orchestration
├── lib.rs       # Exports modules for binary and tests
├── config.rs    # Config struct and Verbosity enum
├── constants.rs # Application-wide constants (timeouts, thread counts)
├── output.rs    # Progress bars, colored output, summary formatting
├── git.rs       # Thin wrappers around git binary commands (with timeout)
└── repo.rs      # Repository detection, update logic, result types

tests/
├── common/
│   └── mod.rs   # TestRepo helper for creating temp git repos
└── integration_test.rs  # Integration tests
```

**7 source files + test infrastructure.**

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

Exports modules for use by `main.rs` and integration tests. Includes usage examples:

```rust
pub mod config;
pub mod constants;
pub mod git;
pub mod output;
pub mod repo;
```

### `constants.rs`

Centralized application-wide constants to avoid magic numbers:

```rust
pub fn git_timeout() -> Duration;  // Configurable via GIT_DAILY_TIMEOUT env var (default: 30s)
pub const RAYON_THREAD_COUNT: usize = 60;
pub const PROGRESS_TICK_MS: u64 = 80;
pub const MAX_VISIBLE_COMPLETIONS: usize = 5;
pub const MASTER_BRANCH: &str = "master";
pub const MAIN_BRANCH: &str = "main";
pub const GIT_DIR: &str = ".git";
pub const DEFAULT_REPO_NAME: &str = "repository";
```

The git timeout can be customized via environment variable:

```bash
GIT_DAILY_TIMEOUT=60 git-daily-v2  # 60 second timeout
```

### `config.rs`

Runtime configuration derived from CLI arguments:

```rust
pub struct Config {
    pub verbosity: Verbosity,
}

impl Config {
    pub fn is_quiet(&self) -> bool;
    pub fn is_verbose(&self) -> bool;
    pub fn git_logger(&self) -> GitLogger;  // Returns verbose or no-op logger
}

pub enum Verbosity {
    Quiet,   // -q: Minimal output, errors only
    Normal,  // Default: Progress bars and summary
    Verbose, // -v: Show git commands, sequential in workspace
}
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

Presentation layer with progress bars, colored output, and callbacks:

- `NoOpCallbacks` - null object pattern for when no output is needed
- `SingleRepoCallbacks` - combines progress bar + verbose output for single repo
- `RepoProgressTracker` - per-repo tracker for workspace mode
- `create_single_repo_progress()` - progress bar for single repo
- `create_workspace_progress(count)` - progress bar for workspace
- `print_working_dir(path)` - prints "Working in: /path"
- `print_summary(results, duration)` - colored summary
- `print_workspace_start(count)` - "Found N repositories"

Includes unit tests for formatting functions.

### `git.rs`

- Thin wrappers around `git` binary via `std::process::Command`
- **Configurable timeout** on all git operations (default 30s, via `GIT_DAILY_TIMEOUT` env var)
- **Callback-based logging** to decouple from presentation layer
- **Branch name validation** to prevent command injection attacks
- Functions: `run_git()`, `run_git_with_logger()`, `get_current_branch()`, `get_current_commit()`,
  `has_uncommitted_changes()`, `fetch_prune()`, `stash()`, `stash_pop()`, `checkout()`, `pull()`
- All functions accept a `GitLogger` callback for verbose output
- Returns `anyhow::Result`

```rust
/// Callback for logging git commands and their output.
pub type GitLogger = fn(&Config, &[&str], Option<&str>);

/// Default no-op logger for non-verbose modes.
pub fn no_op_logger(_config: &Config, _args: &[&str], _output: Option<&str>) {}

/// Verbose logger that prints git commands and output.
pub fn verbose_logger(config: &Config, args: &[&str], output: Option<&str>) { ... }
```

Includes unit tests for branch name validation (shell metacharacters, argument injection, unicode).

### `repo.rs`

Domain layer with core update logic and types:

- `is_git_repo(path) -> bool`
- `find_git_repos(path) -> Vec<PathBuf>` - discovers git repos in subdirectories
- `update(path, callbacks, config) -> UpdateResult` - orchestrates update with callbacks
- `update_workspace(repos, make_callbacks, config) -> Vec<UpdateResult>` - parallel update
- Types: `UpdateResult`, `UpdateOutcome`, `UpdateStep`, `OriginalHead`, `UpdateSuccess`, `UpdateFailure`
- Traits: `UpdateCallbacks` - trait for progress callbacks (zero-cost abstraction)

Note: `NoOpCallbacks` is in `output.rs` (presentation layer), not here.

## Core Types

Defined in `repo.rs`:

```rust
/// Callbacks for monitoring repository update progress and output.
/// Decouples domain logic from presentation concerns.
///
/// Required methods: on_step(), on_complete()
/// Optional methods: on_update_start(), on_step_execute(), on_completion_status()
pub trait UpdateCallbacks: Send + Sync {
    fn on_update_start(&self, repo_name: &str) {}     // Called when update begins (optional)
    fn on_step(&self, step: &UpdateStep);              // Progress tracking (required)
    fn on_step_execute(&self, step: &UpdateStep) {}    // Verbose output (optional)
    fn on_complete(&self, result: &UpdateResult);      // Update finished (required)
    fn on_completion_status(&self, success: bool, error: Option<&str>) {} // (optional)
}

// NoOpCallbacks is in output.rs (null object pattern for presentation)

#[non_exhaustive]  // New variants may be added in future versions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStep {
    Started,
    DetectingBranch,
    CheckingChanges,
    Fetching,
    Stashing,
    CheckingOut,
    Pulling,
    RestoringBranch,
    PoppingStash,
    Completed,
}

/// The original state of HEAD before an update operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginalHead {
    Branch(String),      // HEAD was on a named branch
    DetachedAt(String),  // HEAD was detached at a commit SHA
}

impl OriginalHead {
    pub fn git_ref(&self) -> &str;      // Returns branch name or SHA for checkout
    pub fn is_detached(&self) -> bool;  // True if DetachedAt
    pub fn display(&self) -> String;    // "[branch]" or "[abc1234...detached]"
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
    pub original_head: OriginalHead,   // Type-safe HEAD state
    pub master_branch: &'static str,   // "master" or "main"
    pub had_stash: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateFailure {
    pub error: String,
    pub step: UpdateStep,
}

impl Display for UpdateFailure { ... }  // "failed at CheckingOut: error message"
```

## Callback Pattern

Both single-repo and workspace modes use the unified `UpdateCallbacks` trait:

### Single Repository

```rust
pub fn update<C>(path: &Path, callbacks: &C, config: &Config) -> UpdateResult
where
    C: UpdateCallbacks,

// Usage with SingleRepoCallbacks (combines progress + verbose output)
let progress = output::create_single_repo_progress( & config);
let callbacks = output::SingleRepoCallbacks::new(progress, & config);
let result = repo::update(path, & callbacks, & config);
callbacks.finish( & result);
```

### Workspace Mode

```rust
// With per-repo callbacks factory
let workspace_progress = output::create_workspace_progress(repos.len(), & config);
repo::update_workspace( & repos, | path| {
workspace_progress.create_repo_tracker(repo_name, & config)
}, & config);

// With no-op callbacks (for tests)
repo::update_workspace( & repos, | _ | NoOpCallbacks, & config);
```

### Decoupling Domain from Presentation

The `UpdateCallbacks` trait decouples `repo.rs` (domain) from `output.rs` (presentation):

```
repo.rs (domain)           output.rs (presentation)
     │                            │
     │  UpdateCallbacks trait     │
     ├────────────────────────────┤
     │  on_update_start()         │  SingleRepoCallbacks
     │  on_step()                 │  RepoProgressTracker
     │  on_step_execute()         │  NoOpCallbacks
     │  on_complete()             │     └── all implement trait
     │  on_completion_status()    │
     │                            │

config.rs
     │
     └── git_logger() -> selects verbose or no-op logger
```

**Benefits:**

- Domain layer has no direct imports from presentation layer
- Easy to provide alternative implementations (quiet, verbose, progress bars)
- Testable in isolation with `NoOpCallbacks`
- Git command logging is configurable via `Config::git_logger()`

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
4. Fetching -> git fetch --prune (updates all remote refs)
5. Stashing -> git stash (if needed)
6. CheckingOut -> git checkout master (fallback to main)
7. Pulling -> git pull --ff-only origin master (fast-forward only)
8. RestoringBranch -> git checkout original-branch
9. PoppingStash -> git stash pop (if needed)
10. Completed
```

**Why fetch before stash?** Updates remote tracking branches before modifying working directory state.

**Why pull with --ff-only?** Prevents accidental merge commits during automated updates. If master has diverged from
remote, the operation fails explicitly and user must resolve manually.

On failure: exit immediately, record failure with step and error info. No automatic state restoration is attempted –
this avoids compounding errors and lets the user resolve issues (like stash pop conflicts) manually with full context.

## CLI Interface

```
git-daily-v2              # Normal mode with progress bars
git-daily-v2 -v, --verbose  # Show git commands (sequential in workspace)
git-daily-v2 -q, --quiet    # Minimal output for CI/scripts
git-daily-v2 --help         # Show help
git-daily-v2 --version      # Show version
```

### Verbosity Modes

| Mode           | Progress    | Summary             | Git Commands    | Workspace Execution |
|----------------|-------------|---------------------|-----------------|---------------------|
| Quiet (`-q`)   | Hidden      | Count + errors only | Hidden          | Parallel            |
| Normal         | Spinner/bar | Full details        | Hidden          | Parallel            |
| Verbose (`-v`) | Hidden      | Full details        | Shown to stderr | Sequential          |

### Exit Codes

| Code | Meaning                                    |
|------|--------------------------------------------|
| 0    | All repositories updated successfully      |
| 1    | Some repositories failed (partial success) |
| 2    | All repositories failed                    |

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

| Test                                          | What it verifies                  |
|-----------------------------------------------|-----------------------------------|
| `updates_repo_and_returns_to_original_branch` | Happy path                        |
| `stashes_and_restores_uncommitted_changes`    | Stash/restore flow                |
| `falls_back_to_main_when_no_master`           | Main branch detection             |
| `reports_failure_when_no_remote`              | Error handling                    |
| `handles_already_on_main_branch`              | Edge case                         |
| `handles_detached_head`                       | Detached HEAD state handling      |
| `update_workspace_updates_multiple_repos`     | Basic workspace mode              |
| `workspace_mixed_success_and_failure`         | Partial failures in workspace     |
| `workspace_with_dirty_repos`                  | Stash handling across repos       |
| `workspace_callbacks_called_for_each_repo`    | Callback invocation correctness   |
| `workspace_repos_on_different_branches`       | Branch restoration per repo       |
| `workspace_empty_directory`                   | Empty workspace handling          |
| `workspace_nested_repos_not_discovered`       | Only immediate subdirs scanned    |
| `workspace_order_independence`                | Parallel execution consistency    |
| `workspace_with_untracked_files_only`         | Untracked files don't cause stash |

### What NOT to Test

- `git.rs` in isolation (thin wrappers)
- Output formatting (verified by eye)
- That dependencies work
- 100% coverage

## Design Decisions

| Decision             | Choice                          | Rationale                                   |
|----------------------|---------------------------------|---------------------------------------------|
| Git interaction      | Shell out to `git` binary       | Matches user's git config, easier debugging |
| Parallelism          | `rayon`                         | Simple `.par_iter()`, not async overhead    |
| Single-repo callback | Closure-based                   | Simpler for single use: `\|step\| { ... }`  |
| Workspace callback   | Trait-based (`UpdateCallbacks`) | Zero-cost abstraction, no heap alloc        |
| Error handling       | `anyhow`                        | CLI tool - good messages, not typed errors  |
| File structure       | 6 source files                  | lib.rs needed for test access               |
| CLI config           | `Config` struct                 | Extensible for future flags (dry-run, etc.) |
| Types location       | In `repo.rs`                    | <40 lines, tightly coupled to logic         |
| Testing              | Integration with real git       | No mocking complexity, catches real bugs    |

## Edge Cases and Handling

| Edge Case                    | Current Handling                         | Notes                                          |
|------------------------------|------------------------------------------|------------------------------------------------|
| **Detached HEAD**            | Stores commit SHA, restores after update | Displayed as `[abc1234...detached]` in summary |
| **Stash pop conflicts**      | Fails entire operation                   | User must resolve manually                     |
| **No master or main branch** | Fails at checkout step                   | Repo needs one of these branches               |
| **Only untracked files**     | No stash created, no pop attempted       | Untracked files preserved                      |
| **Empty workspace**          | Returns empty results, exit 0            | Not an error condition                         |
| **Nested git repos**         | Only immediate subdirs scanned           | Intentional to avoid complexity                |
| **Git command timeout**      | Fails after timeout (default 30s)        | Configurable via GIT_DAILY_TIMEOUT env var     |
| **Shallow clones**           | Works normally                           | fetch/pull handle shallow repos                |
| **No remote configured**     | Fails at fetch step                      | Clear error message                            |

### Design Principle: Fail Fast, Don't Auto-Recover

When an error occurs mid-update (e.g., checkout fails after stash), we:

1. Record the failure with step and error info
2. Exit immediately without attempting recovery
3. Let the user resolve with full context

**Rationale:** Auto-recovery (like auto-popping stash after checkout failure) could compound errors
and leave repos in confusing states. Manual resolution is safer.

## Layer Separation

The codebase follows clean layer separation:

```
Presentation (main.rs, output.rs)
    ↓
Domain (repo.rs, config.rs, constants.rs)
    ↓
Infrastructure (git.rs)
```

**Key decoupling:** `git.rs` does not depend on `output.rs`. Instead, it uses a callback-based
logging pattern (`GitLogger`) that the domain layer injects. This keeps infrastructure
unaware of presentation concerns.

## Thread Safety in WorkspaceProgress

The `WorkspaceProgress` struct uses a consolidated `CompletionState` to reduce lock contention:

```rust
struct CompletionState {
    repos: VecDeque<(String, bool)>,  // Recent completions
    failed_count: usize,               // For status message
    total_completed: usize,            // For ellipsis logic
}

// Single lock acquisition instead of three separate locks
// Uses .expect() for clear error message on mutex poisoning
let mut state = self .state.lock().expect("WorkspaceProgress state mutex poisoned");
```

## Future Extensions (Not Implemented Now)

- Config file for branch order, excluded dirs
- Subcommands (`git-daily-v2 status`, `git-daily-v2 config`)
- Max parallelism control (`-j, --jobs <N>`)
- `--dry-run` flag to preview operations without executing
- Warn on stash pop conflicts instead of failing
