# git-daily cleanup: Design Document

## Overview

A new `cleanup` subcommand for git-daily-rust that provides interactive deletion of stale local branches.

## Command Structure

```bash
git-daily           # Current behavior (update repos)
git-daily update    # Explicit update (same as above)
git-daily cleanup   # NEW: Interactive branch cleanup
```

Backward compatibility: Running `git-daily` without subcommand defaults to `update` behavior.

---

## User Experience

### Branch List Display

```
git-daily cleanup

Analyzing branches...

Found 7 local branches:

  BRANCH                      STATUS              REMOTE
  ─────────────────────────────────────────────────────────────
  feature/auth-refactor       merged              gone
  feature/login-page          merged              gone
  bugfix/header-crash         merged              exists
* feature/dark-mode           (current branch)    exists
  bugfix/old-header           unclear             exists
  experiment/new-api          unmerged            gone
  hotfix/security-patch       merged              gone

Legend:
  merged   = fully merged into master/main (regular or squash merge)
  unclear  = could not verify merge status (git command failed)
  unmerged = definitely contains changes not in master/main
  gone     = remote tracking branch deleted
  exists   = remote tracking branch still exists
  *        = current branch
```

### Color Scheme

| Element            | Color       | Rationale                             |
|--------------------|-------------|---------------------------------------|
| `merged`           | green       | Safe to delete                        |
| `unmerged`         | yellow      | Caution, may lose work                |
| `unclear`          | magenta     | Needs attention, status unknown       |
| `(current branch)` | cyan        | Informational                         |
| `gone`             | dim gray    | Orphaned, likely safe                 |
| `exists`           | default     | Neutral                               |
| Warnings           | yellow bold | Attention needed                      |
| Errors             | red bold    | Problem occurred                      |
| Success            | green       | Positive outcome                      |

### Selection Interface

Interactive checkbox list with standard controls:

```
Select branches to delete:

  [ ] feature/auth-refactor     merged    gone
  [x] feature/login-page        merged    gone
  [ ] bugfix/header-crash       merged    exists
> [x] hotfix/security-patch     merged    gone
  ─────────────────────────────────────────────
  [ ] bugfix/old-header         unclear   exists   ⚠ status unknown
  [ ] experiment/new-api        unmerged  gone     ⚠ unmerged

────────────────────────────────────────────────────────────
2 selected | ↑↓ navigate | Space toggle | Enter continue
```

**Controls:**

- `↑`/`↓` = navigate through list
- `Space` = toggle checkbox
- `Enter` = continue to confirmation
- `Esc` or `q` = cancel

**Design rationale:**

- Standard controls match CLI conventions (`fzf`, `inquire`, `dialoguer`)
- Breaking conventions causes confusion for experienced CLI users
- Visual separator (`───`) divides safe from dangerous branches
- Real-time selection count provides feedback
- Hint bar makes controls discoverable
- Safety is handled by the confirmation step, not the selection step

**Why Enter confirms (not toggles):**
Accidental Enter at selection just shows the summary screen - user must still explicitly confirm before deletion.
Friction belongs at the point of danger (confirmation), not at selection.

### Confirmation Screen

After pressing Enter in selection, user sees a review screen with back option:

```
You selected 3 branches to delete:

  • feature/auth-refactor     merged    gone
  • feature/login-page        merged    gone
  • hotfix/security-patch     merged    gone

────────────────────────────────────────────────────────────
[y]es delete / [n]o cancel / [b]ack to selection
```

**Controls:**

- `y` or `Y` = proceed to deletion
- `n`, `N`, or `Esc` = cancel entirely
- `b` or `B` = go back to selection (preserving previous selections)

This provides an escape hatch if Enter was pressed accidentally.

### Confirmation Hierarchy

**Level 1 - Merged + Gone (Safe):** Simple confirmation

```
You selected 3 branches to delete:

  • feature/auth-refactor     merged    gone
  • feature/login-page        merged    gone
  • hotfix/security-patch     merged    gone

────────────────────────────────────────────────────────────
[y]es delete / [n]o cancel / [b]ack to selection
```

**Level 2 - Merged + Remote Exists:** Warning note

```
You selected 1 branch to delete:

  • bugfix/header-crash       merged    exists   ⚠ remote still exists

────────────────────────────────────────────────────────────
[y]es delete / [n]o cancel / [b]ack to selection
```

**Level 3 - Unclear Status:** Additional warning

```
You selected 2 branches to delete:

  • feature/old-work          merged    gone
  • bugfix/header             unclear   exists   ⚠ status unknown

Note: 'unclear' means we could not verify the merge status (git command failed).
      Verify manually if unsure before deleting.

────────────────────────────────────────────────────────────
[y]es delete / [n]o cancel / [b]ack to selection
```

**Level 4 - Unmerged Branches:** Type-to-confirm

```
WARNING: You selected unmerged branches:

  • experiment/new-api        unmerged  gone     (has unique changes)

These branches have NOT been merged. Deleting may result in lost work.

Type 'delete' to confirm, or 'back' to return to selection:
```

### Current Branch Handling

When user selects current branch:

```
You selected 'feature/dark-mode', which is your current branch.
Switch to 'master' to continue? [Y/n] y

Switched to 'master'.
Continuing with deletion...
```

The main branch is auto-detected (same logic as `detect_main_branch`), so no need to ask the user which branch to switch
to.

### Deletion Feedback

Real-time progress:

```
Deleting branches...
  ✓ feature/auth-refactor
  ✓ feature/login-page
  ✓ hotfix/security-patch
  ✗ experiment/new-api: branch not fully merged
```

Summary:

```
════════════════════════════════════════════════════════════════
                      Cleanup Complete
════════════════════════════════════════════════════════════════

Deleted (3):
  ✓ feature/auth-refactor
  ✓ feature/login-page
  ✓ hotfix/security-patch

Failed (1):
  ✗ experiment/new-api
    Error: branch 'experiment/new-api' is not fully merged
    Hint: Use 'git branch -D experiment/new-api' to force delete

Remaining branches: 3
  - master (current)
  - feature/dark-mode
  - main
```

### Edge Cases

**No branches to clean:**

```
No branches available for cleanup.
All branches are either current or unmerged with active remotes.
```

**Detached HEAD state:**

```
Note: You are in detached HEAD state at commit abc1234.
All branches can be selected for deletion.
```

---

## Architecture

### Module Structure

```
src/
├── main.rs         # Add subcommand dispatch
├── lib.rs          # Add cleanup, prompt module exports
├── config.rs       # No changes needed
├── constants.rs    # Add ORIGIN_REMOTE constant
├── output.rs       # Add cleanup output functions
├── git.rs          # Add new git operations
├── repo.rs         # No changes (update-specific)
├── cleanup.rs      # NEW: Branch types, cleanup orchestration
└── prompt.rs       # NEW: Interactive selection abstraction
```

### Layer Separation

```
Presentation (main.rs, output.rs, prompt.rs)
    ↓
Domain (cleanup.rs, config.rs)
    ↓
Infrastructure (git.rs)
```

### Core Types

```rust
// src/cleanup.rs

/// Information about a local branch's status.
#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub merge_status: MergeStatus,
    pub tracking_status: TrackingStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStatus {
    Merged,    // Changes are in main (regular merge, squash merge, or cherry-pick)
    Unmerged,  // Definitely has unique changes not in master/main
    Unclear,   // Cannot determine status (git command failed)
}

impl MergeStatus {
    /// Returns the display label for the UI.
    pub fn display(&self) -> &'static str {
        match self {
            MergeStatus::Merged => "merged",
            MergeStatus::Unmerged => "unmerged",
            MergeStatus::Unclear => "unclear",
        }
    }

    /// Returns true if the branch is safe to delete without force.
    pub fn is_safely_deletable(&self) -> bool {
        matches!(self, MergeStatus::Merged)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackingStatus {
    RemoteExists(String),  // e.g., "origin/feature-x"
    RemoteGone,            // Remote tracking branch deleted
    NoUpstream,            // No remote tracking configured
}

#[derive(Debug, Clone)]
pub struct DeletionResult {
    pub branch: String,
    pub outcome: DeletionOutcome,
}

#[derive(Debug, Clone)]
pub enum DeletionOutcome {
    Deleted,
    ForceDeleted,
    Skipped { reason: String },
    Failed { error: String },
}

pub struct CleanupResult {
    pub main_branch: String,
    pub deletions: Vec<DeletionResult>,
    pub switched_from: Option<String>,
}
```

### Prompter Trait (for testability)

```rust
// src/prompt.rs

/// Abstraction for interactive user prompts.
/// Enables testing cleanup logic without terminal interaction.
pub trait Prompter: Send + Sync {
    /// Multi-select from a list. Returns indices of selected items.
    fn multi_select(&self, prompt: &str, items: &[String]) -> anyhow::Result<Vec<usize>>;

    /// Yes/no confirmation.
    fn confirm(&self, prompt: &str, default: bool) -> anyhow::Result<bool>;

    /// Single selection from a list. Returns index.
    fn select(&self, prompt: &str, items: &[String]) -> anyhow::Result<usize>;

    /// Type-to-confirm for dangerous operations.
    fn type_to_confirm(&self, prompt: &str, expected: &str) -> anyhow::Result<bool>;
}

// Real implementation
pub struct TerminalPrompter;

// For tests
pub struct MockPrompter {
    pub selections: Vec<usize>,
    pub confirmations: Vec<bool>,
}
```

### Interactive Library Choice: `dialoguer`

**Recommendation: `dialoguer`**

| Aspect       | dialoguer           | inquire               |
|--------------|---------------------|-----------------------|
| Popularity   | ~2M downloads/month | ~200K downloads/month |
| Maintenance  | Active, stable      | Active                |
| API style    | Builder pattern     | Builder pattern       |
| MultiSelect  | Yes                 | Yes                   |
| Styling      | Basic theming       | More customizable     |
| Dependencies | Minimal             | Minimal               |

`dialoguer` is preferred because:

1. More established with proven stability
2. Simpler API that matches our needs
3. Already used by many Rust CLI tools
4. Lower risk of breaking changes

```toml
[dependencies]
dialoguer = "0.11"
```

---

## Git Implementation

### Commands Needed

**List branches with tracking info:**

```bash
git for-each-ref --format='%(refname:short)|%(upstream:short)' refs/heads/
```

**Detect main branch:**

```bash
# Try remote's default first
git symbolic-ref refs/remotes/origin/HEAD
# Fallback: check existence
git rev-parse --verify master
git rev-parse --verify main
```

**Check merged status (diff-based, detects regular and squash merges):**

```bash
# Merge detection using tree comparison (detects regular merges, squash merges, and cherry-picks)
git diff main feature-branch
# Empty output = branches have identical content (merged)
# Non-empty output = branches have different content (unmerged)
```

**Verify remote ref exists:**

```bash
git remote update --prune  # Update local cache (optional, slow)
git rev-parse --verify refs/remotes/origin/feature-x
```

**Delete branch:**

```bash
git branch -d feature-x   # Safe delete (fails if unmerged)
git branch -D feature-x   # Force delete
```

### New Functions in `git.rs`

```rust
/// Lists local branches with upstream tracking info.
pub fn list_branches_with_upstream(
    repo: &Path,
    config: &Config,
    logger: GitLogger,
) -> anyhow::Result<String>;

/// Lists branches merged into the specified branch.
pub fn list_merged_branches(
    repo: &Path,
    config: &Config,
    target: &str,
    logger: GitLogger,
) -> anyhow::Result<String>;

/// Gets the merge-base between two refs.
pub fn merge_base(
    repo: &Path,
    config: &Config,
    ref1: &str,
    ref2: &str,
    logger: GitLogger,
) -> anyhow::Result<String>;

/// Diffs a branch against main using `git diff main branch` (tree comparison).
/// Empty output = branch changes are in main (merged, squash-merged, or cherry-picked).
/// Non-empty output = branch has unique changes not in main.
pub fn diff_branch_trees(
    repo: &Path,
    config: &Config,
    main_branch: &str,
    branch: &str,
    logger: GitLogger,
) -> anyhow::Result<String>;

/// Checks if a remote tracking branch exists.
/// `remote_ref` must be in `<remote>/<branch>` form (for example, `origin/feature-x`).
pub fn remote_ref_exists(
    repo: &Path,
    config: &Config,
    remote_ref: &str,
    logger: GitLogger,
) -> anyhow::Result<bool>;

/// Deletes a local branch (safe delete).
pub fn delete_branch(
    repo: &Path,
    config: &Config,
    branch: &str,
    logger: GitLogger,
) -> anyhow::Result<()>;

/// Force deletes a local branch.
pub fn delete_branch_force(
    repo: &Path,
    config: &Config,
    branch: &str,
    logger: GitLogger,
) -> anyhow::Result<()>;
```

### Merge Detection Logic in `cleanup.rs`

```rust
/// Determines if a branch is merged into main (supports both regular and squash merges).
///
/// Uses `git diff main branch` (tree comparison) to check if the branches have identical content.
/// If the diff is empty, the branches match (via regular merge, squash merge, or cherry-pick).
fn check_merge_status(
    repo: &Path,
    branch: &str,
    main_branch: &str,
    config: &Config,
    logger: GitLogger,
) -> MergeStatus {
    match git::diff_branch_trees(repo, config, main_branch, branch, logger) {
        Ok(output) if output.trim().is_empty() => MergeStatus::Merged,
        Ok(_) => MergeStatus::Unmerged,
        Err(_) => MergeStatus::Unclear,
    }
}
```

**Logic explanation:**

- **Empty diff** → Branch changes are already in main (merged, squash-merged, or cherry-picked)
- **Non-empty diff** → Branch has unique work not in main (unmerged)
- **Error** → Can't determine (unclear)

### Edge Cases

| Edge Case               | Handling                                       |
|-------------------------|------------------------------------------------|
| Detached HEAD           | Mark no branch as current, allow all deletions |
| Current branch selected | Offer to switch to master/main first           |
| Protected branches      | Never delete master/main/develop               |
| Shallow clones          | Merged status may be unreliable - add warning  |
| No remote configured    | Skip remote checks, mark as NoUpstream         |
| Worktree checked out    | Let git's error bubble up naturally            |
| Squash-merged branches  | Detected via `git diff` (empty diff = merged)  |

---

## Operation Flow

```
┌─────────────────────────────────────────────────────┐
│  git-daily cleanup                                   │
└─────────────────────────────────────────────────────┘
                         │
                         ▼
              ┌────────────────────────┐
              │  Verify in git repo    │
              └────────────────────────┘
                         │
                         ▼
              ┌────────────────────────┐
              │  Detect main branch    │
              │  (master or main)      │
              └────────────────────────┘
                         │
                         ▼
              ┌────────────────────────┐
              │  List local branches   │
              │  with status info      │
              └────────────────────────┘
                         │
                         ▼
              ┌────────────────────────┐
              │  Display formatted     │
              │  branch list           │
              └────────────────────────┘
                         │
                         ▼
              ┌────────────────────────┐
              │  Numbered multi-select │
              │  (Prompter trait)      │
              └────────────────────────┘
                         │
                         ▼
              ┌────────────────────────┐
              │  Current branch        │──YES──▶ Offer switch flow
              │  selected?             │
              └────────────────────────┘
                         │ NO
                         ▼
              ┌────────────────────────┐
              │  Unmerged branches     │──YES──▶ Type-to-confirm
              │  selected?             │
              └────────────────────────┘
                         │
                         ▼
              ┌────────────────────────┐
              │  Delete selected       │
              │  (continue on failure) │
              └────────────────────────┘
                         │
                         ▼
              ┌────────────────────────┐
              │  Print summary         │
              └────────────────────────┘
```

---

## Error Handling

**Gather phase:** Fail fast if cannot list branches or detect main branch.

**Deletion phase:** Continue on partial failure, collect all results.

**Rationale:** If user selects 5 branches and one fails, they probably want the other 4 deleted.

---

## CLI Interface

```
git-daily cleanup --help

Clean up stale local git branches

Usage: git-daily cleanup [OPTIONS]

Options:
      --dry-run       Show what would be deleted without making changes
  -v, --verbose       Show detailed output including git commands
  -q, --quiet         Minimal output (errors only)
  -h, --help          Print help

Examples:
  git-daily cleanup              # Interactive mode
  git-daily cleanup --dry-run    # Preview what would be deleted
```

---

## Testing Strategy

### Unit Tests

- `cleanup.rs`: Test `BranchInfo` creation, merge/tracking status logic
- `prompt.rs`: Test `MockPrompter` behavior

### Integration Tests

```rust
// tests/cleanup_test.rs

#[test]
fn test_lists_branches_with_merge_status() { ... }

#[test]
fn test_cleanup_deletes_selected_branches() { ... }

#[test]
fn test_cannot_delete_current_branch_without_switch() { ... }

#[test]
fn test_unmerged_branch_requires_force() { ... }

#[test]
fn test_protected_branch_cannot_be_deleted() { ... }
```

### TestRepo Extensions Needed

```rust
impl TestRepo {
    pub fn merge(&self, branch: &str) -> Result<()>;
    pub fn branch_exists(&self, branch: &str) -> bool;
}
```

---

## Dependencies

```toml
[dependencies]
dialoguer = "0.11"  # Interactive terminal prompts
```

---

## Known Limitations

1. **Remote status staleness:** Without `git remote update --prune`, the remote tracking info may be stale. We check
   local refs by default for speed.

2. **Single repo only:** No workspace mode for cleanup (intentional scope limitation).

---

## Step-by-Step Implementation Guide

This guide provides requirements and hints for each phase. You'll need to figure out the implementation details
yourself - that's where the learning happens.

### Phase 1: Project Setup (30 min)

**Goal:** Get the project scaffolding ready.

**Tasks:**

1. Add `dialoguer = "0.11"` to Cargo.toml
2. Create new module files: `src/cleanup.rs` and `src/prompt.rs`
3. Export them from `src/lib.rs`

**Checkpoint:** `cargo build` succeeds (warnings about empty modules are OK).

**Hint:** Look at how existing modules are declared in `lib.rs`.

---

### Phase 2: Infrastructure Layer - git.rs (1-2 hours)

**Goal:** Add git command wrappers needed for branch operations.

**Functions to implement:**

| Function                      | Git Command                                             | Returns                         |
|-------------------------------|---------------------------------------------------------|---------------------------------|
| `list_branches_with_upstream` | `git for-each-ref --format=... refs/heads/`             | Branch names with upstream info |
| `list_merged_branches`        | `git branch --merged <target>`                          | Branches merged into target     |
| `merge_base`                  | `git merge-base <ref1> <ref2>`                          | Common ancestor commit          |
| `diff_branch_trees`           | `git diff <main> <branch>`                              | Tree comparison (content diff)  |
| `remote_ref_exists`           | `git rev-parse --verify refs/remotes/<remote>/<branch>` | bool (via Result)               |
| `delete_branch`               | `git branch -d <name>`                                  | Result (safe delete)            |
| `delete_branch_force`         | `git branch -D <name>`                                  | Result (force delete)           |

**Hints:**

- Study existing functions in `git.rs` to understand the patterns
- Run the git commands manually first to see their output format
- For `for-each-ref`, use `%(refname:short)` and `%(upstream:short)` format specifiers
- Remember to validate branch names before passing to git (security!)

**Testing:** Run the git commands in a real repo to understand their output.

**Checkpoint:** `cargo build` and `cargo test` pass.

---

### Phase 3: Domain Layer - cleanup.rs (2-3 hours)

**Goal:** Implement the core business logic for branch cleanup.

**Types to define:**

```
BranchInfo
├── name: String
├── is_current: bool
├── merge_status: MergeStatus
└── tracking_status: TrackingStatus

MergeStatus: Merged | Unmerged | Unclear
TrackingStatus: RemoteExists(String) | RemoteGone | NoUpstream
DeletionResult, DeletionOutcome, CleanupResult
```

**Functions to implement:**

| Function                | Purpose                       | Hints                                                 |
|-------------------------|-------------------------------|-------------------------------------------------------|
| `detect_main_branch`    | Find master or main           | Try origin/HEAD first, then check which exists        |
| `check_merge_status`    | Determine if branch is merged | Use `git diff main branch` - empty means merged       |
| `check_tracking_status` | Check if remote still exists  | Use `remote_ref_exists` from git.rs                   |
| `list_branches`         | Get all branches with status  | Parse `for-each-ref` output, skip main branch         |
| `delete_single_branch`  | Delete one branch             | Use force if not safe                                 |

**Key logic for `check_merge_status`:**

Uses `git diff main branch` (tree comparison) to detect if branches have identical content:
- Empty diff = merged (regular merge, squash merge, or cherry-pick)
- Non-empty diff = unmerged (has unique changes)
- Error = unclear (cannot determine)

**Hints:**

- `git branch --merged` output has `*` prefix for current branch - strip it
- When parsing pipe-delimited output, handle empty strings
- Consider what happens in detached HEAD state

**Checkpoint:** `cargo build` passes.

---

### Phase 4: Presentation Layer - prompt.rs (1 hour)

**Goal:** Create an abstraction for interactive prompts (for testability).

**Trait to define:**

```rust
trait Prompter {
    fn multi_select(...) -> Result<Vec<usize>>;  // Returns selected indices
    fn confirm(...) -> Result<bool>;              // Yes/no
    fn select(...) -> Result<usize>;              // Single selection
    fn type_to_confirm(...) -> Result<bool>;      // Type specific text
}
```

**Implementations needed:**

1. `TerminalPrompter` - uses dialoguer for real interaction
2. `MockPrompter` (cfg(test)) - returns predefined values for testing

**Hints:**

- Read dialoguer docs: https://docs.rs/dialoguer
- Use `MultiSelect`, `Confirm`, `Select`, and `Input` from dialoguer
- For mock, use `RefCell<Vec<...>>` to store predefined responses

---

### Phase 5: Output Functions - output.rs (1 hour)

**Goal:** Add formatted output for branch display and cleanup results.

**Functions to implement:**

| Function                   | Purpose                          |
|----------------------------|----------------------------------|
| `format_branch_line`       | Format single branch with colors |
| `print_branch_list_header` | Print column headers             |
| `print_deletion_result`    | Show ✓ or ✗ for each deletion    |
| `print_cleanup_summary`    | Show final summary               |

**Color scheme (from design):**

- merged = green
- unmerged = yellow
- unclear = magenta
- gone = dimmed

**Hints:**

- Look at existing output functions for patterns
- Use the `colored` crate methods: `.green()`, `.yellow()`, `.dimmed()`, etc.
- Use format strings with width specifiers for alignment: `{:<30}`

---

### Phase 6: CLI Integration - main.rs (1-2 hours)

**Goal:** Add subcommands and wire everything together.

**CLI structure:**

```
git-daily           # Default: runs update
git-daily update    # Explicit update
git-daily cleanup   # New: branch cleanup
  --dry-run         # Preview without deleting
```

**Tasks:**

1. Add `Command` enum with `Update` and `Cleanup` variants
2. Modify Args struct to use `#[command(subcommand)]`
3. Keep backward compatibility: no subcommand = update
4. Implement `run_cleanup` function

**`run_cleanup` flow:**

1. Verify we're in a git repo
2. List branches with status
3. Display branch list
4. Get user selection (multi-select)
5. Handle special cases (current branch, unmerged)
6. Confirm deletion
7. Delete and show results

**Hints:**

- Look at clap docs for subcommand syntax
- Use `Option<Command>` to make subcommand optional
- Start simple - get basic flow working, then add edge case handling

**Checkpoint:** `cargo run -- cleanup --help` shows help text.

---

### Phase 7: Testing (1-2 hours)

**Goal:** Add integration tests for cleanup functionality.

**TestRepo helpers needed:**

- `merge(branch)` - regular merge
- `squash_merge(branch)` - squash merge
- `branch_exists(name)` - check if branch exists

**Test cases to implement:**

| Test                      | What to verify                                 |
|---------------------------|------------------------------------------------|
| Merged branch detection   | Regular merge shows as "merged"                |
| Unmerged branch detection | Branch with unique commits shows as "unmerged" |
| Squash-merge detection    | Squash-merged branch shows as safe             |
| Branch deletion           | Deleted branch no longer exists                |
| Current branch handling   | Cannot delete without switching                |

**Hints:**

- Look at existing tests in `tests/integration_test.rs` for patterns
- Each test needs its own `TestRepo` for isolation
- Set up the git state you need, then call cleanup functions and assert

---

### Phase 8: Polish & Edge Cases (1-2 hours)

**Goal:** Handle all the edge cases from the design document.

**Tasks:**

1. **Current branch handling**
    - Detect if current branch is selected
    - Offer to switch to main/master first
    - Then proceed with deletion

2. **Confirmation flow with back option**
    - Show summary of selected branches
    - `[y]es / [n]o / [b]ack` prompt
    - Back returns to selection (preserving previous choices)

3. **Type-to-confirm for dangerous operations**
    - If any unmerged branches selected, require typing "delete"
    - Show clear warning about data loss

4. **Protected branches**
    - Never allow deletion of master/main
    - Even if somehow selected, skip with message

**Hints:**

- You'll need a loop for the selection → confirm → (maybe back) flow
- Keep track of previously selected indices for the "back" feature
- Test each edge case manually as you implement

---

### Implementation Checklist

Use this checklist to track progress. Estimated total time: 10-15 hours.

- [x] **Phase 1: Setup** (~30 min)
    - [x] Add dialoguer dependency
    - [x] Create cleanup.rs and prompt.rs files
    - [x] Update lib.rs exports
    - [x] Verify: `cargo build` succeeds

- [x] **Phase 2: Git commands** (~1-2 hours)
    - [x] list_branches_with_upstream
    - [x] list_merged_branches
    - [x] merge_base
    - [x] diff_branch_trees
    - [x] remote_ref_exists
    - [x] delete_branch / delete_branch_force
    - [x] Verify: `cargo build && cargo test` pass

- [x] **Phase 3: Domain logic** (~2-3 hours)
    - [x] Define types: BranchInfo, MergeStatus, TrackingStatus, etc.
    - [x] detect_main_branch
    - [x] check_merge_status (diff-based detection)
    - [x] check_tracking_status
    - [x] list_branches
    - [x] delete_single_branch
    - [x] Verify: `cargo build` passes

- [x] **Phase 4: Prompts** (~1 hour)
    - [x] Define Prompter trait
    - [x] TerminalPrompter (using dialoguer)
    - [x] MockPrompter for tests
    - [x] Verify: `cargo build` passes

- [x] **Phase 5: Output** (~1 hour)
    - [x] format_branch_line (with colors)
    - [x] print_branch_list_header
    - [x] print_deletion_result
    - [x] print_cleanup_summary
    - [x] Verify: `cargo build` passes

- [x] **Phase 6: CLI** (~1-2 hours)
    - [x] Add Command enum with subcommands
    - [x] Modify Args to support optional subcommand
    - [x] Implement run_cleanup (basic flow)
    - [x] Handle --dry-run flag
    - [x] Verify: `cargo run -- cleanup --help` works

- [x] **Phase 7: Tests** (~1-2 hours)
    - [x] TestRepo.merge helper
    - [x] TestRepo.squash_merge helper
    - [x] TestRepo.branch_exists helper
    - [x] Test: merged branch detection
    - [x] Test: unmerged branch detection
    - [x] Test: squash-merge detection
    - [x] Test: branch deletion
    - [x] Verify: `cargo test` passes

- [x] **Phase 8: Polish** (~1-2 hours)
    - [x] Current branch switch flow
    - [x] Confirmation with back option
    - [x] Type-to-confirm for unmerged
    - [x] Protected branch check
    - [x] Manual testing of all flows

---

### Common Pitfalls

Things that will bite you if you're not careful:

1. **Branch name validation**
    - Problem: User input passed directly to git commands could be malicious
    - Solution: Use `validate_branch_name()` before any git command

2. **Empty string vs None**
    - Problem: `git for-each-ref` returns empty string for missing upstream, not "nothing"
    - Solution: Check for both `None` and empty string when parsing

3. **Current branch marker**
    - Problem: `git branch --merged` output has `*` prefix for current branch
    - Solution: Strip the `*` and whitespace when parsing branch names

4. **Detached HEAD state**
    - Problem: `get_current_branch` returns "HEAD" when detached
    - Solution: Check for this and handle accordingly (no branch is "current")

5. **Test isolation**
    - Problem: Tests sharing state cause flaky failures
    - Solution: Each test creates its own fresh TestRepo

6. **Force delete logic**
    - Problem: Using `-d` on unmerged branch fails
    - Solution: Use `-D` when `is_safely_deletable()` returns false

---

### Resources

- **dialoguer docs**: https://docs.rs/dialoguer - for interactive prompts
- **clap subcommands**: https://docs.rs/clap - search for "subcommand" examples
- **git for-each-ref**: `git help for-each-ref` - format specifiers
- **git diff**: `git help diff` - tree comparison `main branch`
- **Existing code**: Study `git.rs` and `repo.rs` for patterns

---

## Future Extensions (Not in v1)

- `--all-merged` flag for non-interactive deletion of safe branches
- `--delete-remote` flag to also delete remote branches
- Configurable protected branch list
- Cherry-pick detection via patch-id comparison
