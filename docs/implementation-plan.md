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
*Goal: Print working directory, detect if current dir is a git repo*

- [x] **2.1** `output.rs`: implement `print_working_dir(path: &Path)`
- [ ] **2.2** `repo.rs`: implement `is_git_repo(path: &Path) -> bool`
- [ ] **2.3** `main.rs`: get cwd, print it, check if git repo, print result
- [ ] **2.4** Verify: run in a git repo vs non-git directory, see different output

---

## Phase 3: Workspace Discovery (Stub)
*Goal: If not a repo, find subdirectories that are repos*

- [ ] **3.1** `main.rs`: add logic to branch based on `is_git_repo` result
- [ ] **3.2** `main.rs`: if not a repo, iterate subdirs, print each that is a repo
- [ ] **3.3** `output.rs`: add `print_no_repos()` and `print_workspace_start(count)`
- [ ] **3.4** Verify: run in a directory containing multiple git repos

---

## Phase 4: Git Wrappers
*Goal: All git operations implemented and manually testable*

- [ ] **4.1** `git.rs`: implement `run_git(repo, args)` helper (private)
- [ ] **4.2** `git.rs`: implement `current_branch(repo) -> Result<String>`
- [ ] **4.3** `git.rs`: implement `has_uncommitted_changes(repo) -> Result<bool>`
- [ ] **4.4** `git.rs`: implement `stash(repo) -> Result<()>`
- [ ] **4.5** `git.rs`: implement `stash_pop(repo) -> Result<()>`
- [ ] **4.6** `git.rs`: implement `checkout(repo, branch) -> Result<()>`
- [ ] **4.7** `git.rs`: implement `fetch_prune(repo) -> Result<()>`
- [ ] **4.8** Verify: manually test each function in a test repo

---

## Phase 5: Core Types
*Goal: Define the data structures for update results*

- [ ] **5.1** `repo.rs`: define `UpdateStep` enum
- [ ] **5.2** `repo.rs`: define `UpdateOutcome` enum (Success, Failed)
- [ ] **5.3** `repo.rs`: define `UpdateResult` struct
- [ ] **5.4** Verify: `cargo build` succeeds

---

## Phase 6: Single Repo Update (No Progress)
*Goal: Core update logic working for one repo*

- [ ] **6.1** `repo.rs`: implement `update<F>(path, on_step) -> UpdateResult` signature
- [ ] **6.2** `repo.rs`: implement `do_update()` - detect branch
- [ ] **6.3** `repo.rs`: add stash logic (if dirty)
- [ ] **6.4** `repo.rs`: add checkout main/master with fallback
- [ ] **6.5** `repo.rs`: add fetch --prune
- [ ] **6.6** `repo.rs`: add restore original branch
- [ ] **6.7** `repo.rs`: add stash pop (if stashed)
- [ ] **6.8** `repo.rs`: implement `restore_state()` helper for error recovery
- [ ] **6.9** `main.rs`: call `repo::update()` for single repo mode (with `|_| {}` callback)
- [ ] **6.10** Verify: manually test on a real repo with a feature branch

---

## Phase 7: Summary Output
*Goal: Show results with colors after update*

- [ ] **7.1** `output.rs`: implement `print_summary(results, duration)`
- [ ] **7.2** `output.rs`: implement `print_success(result)` helper
- [ ] **7.3** `output.rs`: implement `print_failure(result)` helper
- [ ] **7.4** `main.rs`: collect results, call `print_summary()`
- [ ] **7.5** `main.rs`: exit with code 1 if any failures
- [ ] **7.6** Verify: see green success output for working repo

---

## Phase 8: Progress Bars
*Goal: Visual progress during updates*

- [ ] **8.1** `output.rs`: implement `create_repo_progress() -> ProgressBar`
- [ ] **8.2** `output.rs`: implement `update_progress(pb, step)`
- [ ] **8.3** `main.rs`: create progress bar, pass callback to `repo::update()`
- [ ] **8.4** `repo.rs`: ensure all steps call `on_step()` callback
- [ ] **8.5** Verify: see progress bar animate during single repo update

---

## Phase 9: Parallel Workspace Updates
*Goal: Update multiple repos in parallel with overall progress*

- [ ] **9.1** `output.rs`: implement `create_workspace_progress(total) -> ProgressBar`
- [ ] **9.2** `main.rs`: extract `run_single_repo()` function
- [ ] **9.3** `main.rs`: extract `run_workspace()` function
- [ ] **9.4** `main.rs`: add rayon `.par_iter()` in workspace mode
- [ ] **9.5** `main.rs`: add atomic counter for progress updates
- [ ] **9.6** Verify: run in workspace with 3+ repos, see parallel execution

---

## Phase 10: Test Infrastructure
*Goal: TestRepo helper ready for integration tests*

- [ ] **10.1** Create `tests/common/mod.rs`
- [ ] **10.2** Implement `TestRepo::new()` - basic repo with initial commit
- [ ] **10.3** Implement `TestRepo::with_remote()` - clone setup
- [ ] **10.4** Implement helper methods: `current_branch()`, `create_branch()`, `checkout()`
- [ ] **10.5** Implement helper methods: `make_dirty()`, `has_stash()`, `file_exists()`
- [ ] **10.6** Verify: write a trivial test that creates and uses TestRepo

---

## Phase 11: Integration Tests
*Goal: Core behaviors verified with automated tests*

- [ ] **11.1** Create `tests/integration_test.rs`
- [ ] **11.2** Test: updates repo and returns to original branch
- [ ] **11.3** Test: stashes and restores uncommitted changes
- [ ] **11.4** Test: handles repo already on main
- [ ] **11.5** Test: falls back to main when no master branch
- [ ] **11.6** Test: reports failure when fetch fails (no remote)
- [ ] **11.7** Verify: `cargo test` passes

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

| Phase | Milestone | You'll Know It Works When... |
|-------|-----------|------------------------------|
| 1 | Skeleton | `cargo build` succeeds |
| 2-3 | Detection | Correctly identifies repos vs workspaces |
| 4-6 | Core Logic | Single repo updates successfully |
| 7-8 | UX | Colored output and progress bars work |
| 9 | Parallelism | Multiple repos update simultaneously |
| 10-11 | Tests | `cargo test` passes |
| 12 | Polish | Ready for daily use |

---

## Session Planning

**First session goal:** Complete Phases 1-3
- Project compiles
- Prints working directory
- Detects git repos
- Lists repos in a workspace

Each subsequent session can tackle one phase.

---

## Testing Strategy

- **Phases 1-9:** Build it, verify manually, get it working
- **Phase 10-11:** Add tests to lock in the behavior
- **Future bugs:** Write a failing test first, then fix
