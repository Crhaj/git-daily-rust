# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.4.0] - 2025-02-02

### Added
- Inline branch status display in multi-select prompt for better UX
- Column alignment tests to prevent ANSI color rendering regressions
- `test-utils` feature flag exposing `MockPrompter` for integration tests

### Changed
- Branch selection now shows status, remote info, and warnings inline
- Padding applied before colorization to fix column alignment with ANSI codes

### Removed
- **BREAKING**: `on_branch_list` callback from `CleanupCallbacks` trait

## [0.3.0] and earlier

Initial development of git-daily-rust with:
- Single repo and workspace update modes
- Interactive branch cleanup with merge detection
- Squash-merge detection via `git merge-tree`
- Three-tier confirmation flow (safe/unclear/unmerged)
