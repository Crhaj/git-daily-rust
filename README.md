# git-daily-rust

[![CI](https://github.com/Crhaj/git-daily-rust/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/Crhaj/git-daily-rust/actions/workflows/ci.yml)
[![Codecov](https://codecov.io/gh/Crhaj/git-daily-rust/branch/master/graph/badge.svg)](https://codecov.io/gh/Crhaj/git-daily-rust)

A fast CLI tool for keeping multiple git repositories up to date. Updates repositories by stashing local changes,
fetching from remote, pulling the main branch, and restoring your previous state.

## Features

- **Workspace mode**: Update all git repositories in a directory in parallel
- **Single repo mode**: Update a single repository
- **Safe updates**: Automatically stashes uncommitted changes and restores them after update
- **Branch preservation**: Returns to your original branch after updating master/main
- **Smart branch detection**: Tries `master` first, falls back to `main`
- **Progress tracking**: Visual progress bars for workspace updates
- **Verbosity controls**: Quiet mode for CI, verbose mode for debugging

## Installation

```bash
cargo install --path .
```

## Usage

```bash
# Update all repos in current directory (workspace mode)
# Automatically detected when not inside a git repository
git-daily-v2

# Update a single repository (auto-detected when inside a git repo)
cd my-project && git-daily-v2

# Verbose mode - show git commands being executed
git-daily-v2 --verbose

# Quiet mode - minimal output for scripts/CI
git-daily-v2 --quiet

# Custom timeout for slow networks (default: 30 seconds)
GIT_DAILY_TIMEOUT=60 git-daily-v2
```

## Exit Codes

| Code | Meaning                               |
|------|---------------------------------------|
| 0    | All repositories updated successfully |
| 1    | Some repositories failed to update    |
| 2    | All repositories failed to update     |

## How It Works

For each repository, git-daily-rust:

1. Detects the current branch
2. Checks for uncommitted changes
3. Fetches from remote with pruning
4. Stashes changes (if any tracked files are modified)
5. Checks out master/main branch
6. Pulls latest changes (fast-forward only)
7. Restores original branch
8. Pops stash (if changes were stashed)

## License

MIT
