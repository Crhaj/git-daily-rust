# git-daily-rust: Implementation Plan

## Overview

This document tracks the implementation progress. Each phase builds on the previous one, creating working milestones along the way.

## Dependencies Between Modules

```
main.rs
   ├── imports lib.rs
   ├── uses output.rs (printing)
   └── uses repo.rs (update logic)
              └── uses git.rs (git commands)
```

Build order: `git.rs` → `repo.rs` → `output.rs` → `main.rs`

---

## Phase 1: Project Skeleton
*Goal: Compilable project with module structure*

- [x] **1.1** Update `Cargo.toml` with dependencies (anyhow, colored, rayon, indicatif, clap)
- [x] **1.2** Update `Cargo.toml` with dev-dependencies (tempfile)
- [x] **1.3** Update `Cargo.toml` with binary name `git-daily-v2`
- [x] **1.4** Create `src/lib.rs` - export empty modules
- [x] **1.5** Create `src/output.rs` - empty module with TODO comments
- [x] **1.6** Create `src/git.rs` - empty module with TODO comments
- [x] **1.7** Create `src/repo.rs` - empty module with TODO comments
- [x] **1.8** Update `src/main.rs` - import from lib, print "Hello from git-daily-v2"
- [x] **1.9** Verify: `cargo build` succeeds, `cargo run` prints message

---

## Phase 2: Basic Output and Detection
*Goal: Print working directory, detect if the current dir is a git repo*

- [x] **2.1** `output.rs`: implement `print_working_dir(path: &Path)`
- [x] **2.2** `repo.rs`: implement `is_git_repo(path: &Path) -> bool`
- [x] **2.3** `main.rs`: get cwd, print it, check if git repo, print result
- [x] **2.4** Verify: run in a git repo vs. non-git directory, see different output

---

## Phase 3: Workspace Discovery (Stub)
*Goal: If not a repo, find subdirectories that are repos*

- [x] **3.1** `main.rs`: add logic to branch based on `is_git_repo` result
- [x] **3.2** `main.rs`: if not a repo, iterate subdirs, print each that is a repo
- [x] **3.3** `repo.rs`: add logic to find all git repos in directory `find_git_repos` 
- [x] **3.4** `output.rs`: add `print_no_repos()` and `print_workspace_start(count)`
- [x] **3.5** Verify: run in a directory containing multiple git repos

---

## Phase 4: Git Wrappers
*Goal: All git operations implemented and manually testable*

- [x] **4.1** `git.rs`: implement `run_git(repo, args)` helper (private)
- [x] **4.2** `git.rs`: implement `get_current_branch(repo) -> Result<String>`
- [x] **4.3** `git.rs`: implement `has_uncommitted_changes(repo) -> Result<bool>`
- [x] **4.4** `git.rs`: implement `stash(repo) -> Result<()>`
- [x] **4.5** `git.rs`: implement `stash_pop(repo) -> Result<()>`
- [x] **4.6** `git.rs`: implement `checkout(repo, branch) -> Result<()>`
- [x] **4.7** `git.rs`: implement `fetch_prune(repo) -> Result<()>`
- [x] **4.8** Verify: manually test each function in a test repo

---

## Phase 5: Core Types
*Goal: Define the data structures for update results*

- [x] **5.1** `repo.rs`: define `UpdateStep` enum
- [x] **5.2** `repo.rs`: define `UpdateOutcome` enum (Success, Failed)
- [x] **5.3** `repo.rs`: define `UpdateResult` struct
- [x] **5.4** Verify: `cargo build` succeeds

---

## Phase 6: Single Repo Update (No Progress)
*Goal: Core update logic working for one repo*

- [x] **6.1** `repo.rs`: implement `update<F>(path, on_step) -> UpdateResult` signature
- [x] **6.2** `repo.rs`: implement `do_update()` - detect branch
- [x] **6.3** `repo.rs`: add stash logic (if dirty)
- [x] **6.4** `repo.rs`: add checkout main/master with fallback
- [x] **6.5** `repo.rs`: add fetch --prune
- [x] **6.6** `repo.rs`: add restore original branch
- [x] **6.7** `repo.rs`: add stash pop (if stashed)
- [x] **6.8** `main.rs`: call `repo::update()` for single repo mode (with `|_| {}` callback)
- [x] **6.9** Verify: manually test on a real repo with a feature branch

---

## Phase 7: Summary Output
*Goal: Show results with colors after update*

- [x] **7.1** `output.rs`: implement `print_summary(results, duration)`
- [x] **7.2** `output.rs`: implement `print_successes(result)` helper
- [x] **7.3** `output.rs`: implement `print_failures(result)` helper
- [x] **7.4** `main.rs`: collect results, call `print_summary()`
- [x] **7.5** `main.rs`: exit with code 1 if any failures
- [x] **7.6** Verify: see green success output for working repo

---

## Phase 8: Progress Bars
*Goal: Visual progress during updates*

- [x] **8.1** `output.rs`: implement `create_repo_progress() -> ProgressBar`
- [x] **8.2** `output.rs`: implement `update_progress(pb, step)`
- [x] **8.3** `main.rs`: create progress bar, pass callback to `repo::update()`
- [x] **8.4** `repo.rs`: ensure all steps call `on_step()` callback
- [x] **8.5** Verify: see progress bar animate during a single repo update

---

## Phase 9: Parallel Workspace Updates
*Goal: Update multiple repos in parallel with overall progress*

- [x] **9.1** `output.rs`: implement `create_workspace_progress(total) -> WorkspaceProgress`
- [x] **9.2** `main.rs`: branch logic for single repo vs. workspace mode
- [x] **9.3** `main.rs`: implement workspace mode with `WorkspaceProgress`
- [x] **9.4** `main.rs`: add rayon `.par_iter()` in workspace mode
- [x] **9.5** `output.rs`: implement a rolling window of last 5 completed repos with "..." indicator
- [x] **9.6** Verify: run in a workspace with 3+ repos, see parallel execution

---

## Phase 9.5: Bug Fix – Stash with Untracked Files Only
*Goal: Fix failure when repo has only untracked files*

**Problem**: When a repo has only untracked files (no modified tracked files):
- `git status --porcelain` shows `??` lines → `has_uncommitted_changes()` returns true
- `git stash` does nothing (untracked files aren't stashed by default)
- We incorrectly set `had_stash = true`
- `git stash pop` fails because there's nothing to pop

**Solution**: Change `stash()` to return `bool` indicating if a stash was actually created.

- [x] **9.5.1** `git.rs`: change `stash(repo) -> Result<bool>` - return false if "No local changes to save"
- [x] **9.5.2** `repo.rs`: use return value of `stash()` for `had_stash` instead of `is_dirty`
- [x] **9.5.3** Verify: test with repo containing only untracked files

---

## Phase 10: Test Infrastructure
*Goal: TestRepo helper ready for integration tests*

**Design Principle**: Reuse production `git.rs` functions in tests. Only create test-specific
helpers for environment setup and queries that don't exist in production.

- [x] **10.1** Create `tests/common/mod.rs`
- [x] **10.2** Implement `TestRepo::new()` - temp dir, git init, configure user, initial commit
- [x] **10.3** Implement `TestRepo::with_remote()` - bare repo as origin, push initial commit
- [x] **10.4** Implement `TestRepo::create_branch(name)` - creates branch without checkout
- [x] **10.5** Implement `TestRepo::make_dirty()` - creates uncommitted file
- [x] **10.6** Implement `TestRepo::make_untracked()` - creates untracked file (for 9.5 bug test)
- [x] **10.7** Implement `TestRepo::has_stash()` - test-specific query
- [x] **10.8** Implement `TestRepo::file_exists(name)` - convenience for verification
- [x] **10.9** Verify: write tests that exercise all TestRepo helpers (7 tests passing)

**Note**: Do NOT implement `current_branch()` or `checkout()` - tests should use
`git::get_current_branch()` and `git::checkout()` directly to avoid duplicating
production code and the "who tests the tests?" problem.

---

## Phase 11: Integration Tests
*Goal: Core behaviors verified with automated tests*

- [x] **11.1** Create `tests/integration_test.rs`
- [x] **11.2** Test: updates repo and returns to the original branch
- [ ] **11.3** Test: stashes and restores uncommitted changes (modified tracked files)
- [ ] **11.4** Test: handles untracked files only (no stash created, no pop attempted)
- [ ] **11.5** Test: handles repo already on main
- [ ] **11.6** Test: falls back to main when no master branch
- [ ] **11.7** Test: reports failure when fetch fails (no remote)
- [ ] **11.8** Verify: `cargo test` passes

---

## Phase 12: Polish
*Goal: Handle edge cases, improve UX*

- [ ] **12.1** Handle detached HEAD state gracefully
- [ ] **12.2** Handle case where stash pop has conflicts (warn, don't fail)
- [ ] **12.3** Add `--help` output with clap
- [ ] **12.4** Add `-v` verbose flag
- [ ] **12.5** Test on macOS/Linux (if applicable)
- [ ] **12.6** Final manual testing of all scenarios

---

## Milestone Summary

| Phase | Milestone   | You'll Know It Works When...               |
|-------|-------------|--------------------------------------------|
| 1     | Skeleton    | `cargo build` succeeds                     |
| 2-3   | Detection   | Correctly identifies repos vs workspaces   |
| 4-6   | Core Logic  | Single repo updates successfully           |
| 7-8   | UX          | Colored output and progress bars work      |
| 9     | Parallelism | Multiple repos update simultaneously       |
| 9.5   | Bug Fix     | Repos with only untracked files don't fail |
| 10-11 | Tests       | `cargo test` passes                        |
| 12    | Polish      | Ready for daily use                        |

---

## Session Planning

**First session goal:** Complete Phases 1–3
- Project compiles
- Prints working directory
- Detects git repos
- Lists repos in a workspace

Each later session can tackle one phase.

---

## Testing Strategy

- **Phases 1–9:** Build it, verify manually, get it working
- **Phase 10–11:** Add tests to lock in the behavior
- **Future bugs:** Write a failing test first, then fix
