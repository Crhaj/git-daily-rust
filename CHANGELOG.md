# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.5.1] - 2026-02-03

### Fixed

- Improve merge detection to handle edge cases correctly

## [0.5.0] - 2026-02-02

### Added

- Test automated changelog generation

## [0.4.0] - 2026-02-02

### Added

- Initial project setup, architecture decisions, implementation plan, project skeleton
- Implement print_working_dir method
- Add is_git_repo function
- Implement find_git_repos method to find all git repos inside directory
- Implement printing of workspace repos count
- Implement run_git private method to run git command
- Implement git wrapper and commands fns
- Test git functions
- Add enums and struct for update storing and outcomes
- Implement do_update logic
- Implement logic for repo update
- Implement summary printing and colors, make structs fields public, polish main function
- Parallel processing, terminal colors and outcome formatting. Set ryon thread count
- Refactoring, more idiomatic rust code structure, imports, constants, structures, then functions, etc
- Improve docs
- Make run_git public since it will be used in tests
- Implement basic tests and methods to setup tests
- Complex tests scenarios. Refactoring to use callback traits
- Finalization of basic implementation
- Pull changes instead of fetch only
- Add readme, add comments, extract config, constants and logger setup, tests
- Prepare cleanup git functions, add tests
- Setup ci
- Add ci and codecov badges to readme
- Implement git operations necessary to delete stale branches
- Prompter trait, implement terminal interactivness, mock prompter for tests
- Cleanup subcommand implementation
- Fetch --prune before cleanup and show full error messages
- **BREAKING**: Inline branch status in selection for better UX

### Changed

- Extract repetative code
- Remove unused code
- Hide implementation detail of TestRepo
- Tests
- Impl display trait for structs, test colored output, implement small helper functions
- Split output.rs file into smaller semantic modules
- Move logic away from main.rs
- Detect main branch once instead of 4 times
- Consolidate MockPrompter into library with test-utils feature

### Documentation

- Add CHANGELOG starting from v0.4.0

### Fixed

- Untracked files only causing stash pop failures
- Grammar and formatting of docs
- Comment typos
- Typo
- Cargo fmt
- Cargo fmt
- Use assert_eq and fix comment
- Defer checkout until after loop to prevent stale state on back
- Update is_current flags after branch switch
- Delete current branch after switching away from it
- Always warn when fetch fails during cleanup

### Testing

- Prepare test infra and setup TestRepo
- Add integration test to run update and return to the original branch
- Implement integration tests for stashing/popping and then untracked only
- Finish integration tests
- Improve test coverage of edge cases
- Improve test coverage
- Add missing cases
- Add integration tests
- Add integration tests for run_interactive


