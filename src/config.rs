//! Configuration types for CLI verbosity and options.

use crate::git::{self, GitLogger};

/// Runtime configuration derived from CLI arguments.
#[derive(Debug, Clone, Copy, Default)]
pub struct Config {
    /// Controls the verbosity level of CLI output.
    pub verbosity: Verbosity,
}

impl Config {
    #[must_use]
    pub fn is_quiet(&self) -> bool {
        self.verbosity == Verbosity::Quiet
    }

    #[must_use]
    pub fn is_verbose(&self) -> bool {
        self.verbosity == Verbosity::Verbose
    }

    /// Returns the appropriate git logger based on verbosity settings.
    ///
    /// This is a presentation-layer concern: config controls which logger
    /// function to use, but doesn't implement logging itself. The actual
    /// logging is implemented as callbacks in the git module.
    #[must_use]
    pub fn git_logger(&self) -> GitLogger {
        if self.is_verbose() {
            git::verbose_logger
        } else {
            git::no_op_logger
        }
    }
}

/// Verbosity level for CLI output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Verbosity {
    Quiet,
    #[default]
    Normal,
    Verbose,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git;

    #[test]
    fn test_config_quiet_and_verbose_flags() {
        let quiet = Config {
            verbosity: Verbosity::Quiet,
        };
        assert!(quiet.is_quiet());
        assert!(!quiet.is_verbose());

        let verbose = Config {
            verbosity: Verbosity::Verbose,
        };
        assert!(!verbose.is_quiet());
        assert!(verbose.is_verbose());
    }

    #[test]
    fn test_git_logger_selects_verbose_or_no_op() {
        let verbose = Config {
            verbosity: Verbosity::Verbose,
        };
        assert!(std::ptr::fn_addr_eq(
            verbose.git_logger() as GitLogger,
            git::verbose_logger as GitLogger
        ));

        let normal = Config {
            verbosity: Verbosity::Normal,
        };
        assert!(std::ptr::fn_addr_eq(
            normal.git_logger() as GitLogger,
            git::no_op_logger as GitLogger
        ));
    }
}
