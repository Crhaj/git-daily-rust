//! Interactive prompt abstractions for terminal UI.
//!
//! Provides a trait-based abstraction over terminal prompts, enabling
//! testability of interactive workflows via dependency injection.

use anyhow::Context;
use dialoguer::{Confirm, Input, MultiSelect, Select};

/// Abstraction for interactive user prompts.
///
/// Implementations provide either real terminal interaction ([`TerminalPrompter`])
/// or mock responses for testing.
pub trait Prompter: Send + Sync {
    /// Presents a multi-select list and returns indices of selected items.
    ///
    /// # Errors
    ///
    /// Returns an error if the user cancels (Esc) or on terminal I/O error.
    fn multi_select(&self, prompt: &str, items: &[&str]) -> anyhow::Result<Vec<usize>>;

    /// Presents a single-select list and returns the index of the selected item.
    ///
    /// # Errors
    ///
    /// Returns an error if the user cancels or on terminal I/O error.
    fn select(&self, prompt: &str, items: &[&str]) -> anyhow::Result<usize>;

    /// Asks a yes/no confirmation question.
    ///
    /// # Errors
    ///
    /// Returns an error if the user cancels or on terminal I/O error.
    fn confirm(&self, prompt: &str, default: bool) -> anyhow::Result<bool>;

    /// Requires the user to type a specific phrase to confirm.
    ///
    /// Returns `Ok(true)` only if the user types the exact `expected` string.
    ///
    /// # Errors
    ///
    /// Returns an error if the user cancels or on terminal I/O error.
    fn type_to_confirm(&self, prompt: &str, expected: &str) -> anyhow::Result<bool>;
}

/// Terminal-based prompter using dialoguer.
#[derive(Debug, Clone, Copy, Default)]
pub struct TerminalPrompter;

impl Prompter for TerminalPrompter {
    fn multi_select(&self, prompt: &str, items: &[&str]) -> anyhow::Result<Vec<usize>> {
        MultiSelect::new()
            .with_prompt(prompt)
            .items(items)
            .interact()
            .context("Multi-select prompt failed")
    }

    fn select(&self, prompt: &str, items: &[&str]) -> anyhow::Result<usize> {
        Select::new()
            .with_prompt(prompt)
            .items(items)
            .interact()
            .context("Select prompt failed")
    }

    fn confirm(&self, prompt: &str, default: bool) -> anyhow::Result<bool> {
        Confirm::new()
            .with_prompt(prompt)
            .default(default)
            .interact()
            .context("Confirm prompt failed")
    }

    fn type_to_confirm(&self, prompt: &str, expected: &str) -> anyhow::Result<bool> {
        let input: String = Input::new()
            .with_prompt(prompt)
            .interact_text()
            .context("Type-to-confirm prompt failed")?;

        Ok(input.trim() == expected)
    }
}

#[cfg(test)]
pub use mock::MockPrompter;

#[cfg(test)]
mod mock {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    /// Mock prompter that returns predefined responses for testing.
    ///
    /// Responses are consumed in FIFO order. Panics if a method is called
    /// with no responses queued (to catch test setup errors).
    ///
    /// Supports error injection via `with_*_err` methods.
    pub struct MockPrompter {
        multi_select_responses: Mutex<VecDeque<anyhow::Result<Vec<usize>>>>,
        select_responses: Mutex<VecDeque<anyhow::Result<usize>>>,
        confirm_responses: Mutex<VecDeque<anyhow::Result<bool>>>,
        type_confirm_responses: Mutex<VecDeque<anyhow::Result<bool>>>,
    }

    impl MockPrompter {
        /// Creates a new mock with empty response queues.
        #[must_use]
        pub fn new() -> Self {
            Self {
                multi_select_responses: Mutex::new(VecDeque::new()),
                select_responses: Mutex::new(VecDeque::new()),
                confirm_responses: Mutex::new(VecDeque::new()),
                type_confirm_responses: Mutex::new(VecDeque::new()),
            }
        }

        /// Queues a successful multi-select response.
        #[must_use]
        pub fn with_multi_select(self, indices: Vec<usize>) -> Self {
            self.multi_select_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .push_back(Ok(indices));
            self
        }

        /// Queues a multi-select error response.
        #[must_use]
        pub fn with_multi_select_err(self, error: &str) -> Self {
            self.multi_select_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .push_back(Err(anyhow::anyhow!("{}", error)));
            self
        }

        /// Queues a successful select response.
        #[must_use]
        pub fn with_select(self, index: usize) -> Self {
            self.select_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .push_back(Ok(index));
            self
        }

        /// Queues a select error response.
        #[must_use]
        pub fn with_select_err(self, error: &str) -> Self {
            self.select_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .push_back(Err(anyhow::anyhow!("{}", error)));
            self
        }

        /// Queues a successful confirm response.
        #[must_use]
        pub fn with_confirm(self, value: bool) -> Self {
            self.confirm_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .push_back(Ok(value));
            self
        }

        /// Queues a confirm error response.
        #[must_use]
        pub fn with_confirm_err(self, error: &str) -> Self {
            self.confirm_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .push_back(Err(anyhow::anyhow!("{}", error)));
            self
        }

        /// Queues a successful type-to-confirm response.
        #[must_use]
        pub fn with_type_confirm(self, value: bool) -> Self {
            self.type_confirm_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .push_back(Ok(value));
            self
        }

        /// Queues a type-to-confirm error response.
        #[must_use]
        pub fn with_type_confirm_err(self, error: &str) -> Self {
            self.type_confirm_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .push_back(Err(anyhow::anyhow!("{}", error)));
            self
        }

        /// Asserts all queued responses have been consumed.
        ///
        /// # Panics
        ///
        /// Panics if any response queue is non-empty.
        pub fn assert_all_consumed(&self) {
            let multi = self
                .multi_select_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .len();
            let select = self
                .select_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .len();
            let confirm = self
                .confirm_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .len();
            let type_confirm = self
                .type_confirm_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .len();

            let total = multi + select + confirm + type_confirm;
            assert!(
                total == 0,
                "MockPrompter has {} unconsumed responses (multi_select: {}, select: {}, confirm: {}, type_confirm: {})",
                total, multi, select, confirm, type_confirm
            );
        }
    }

    impl Default for MockPrompter {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Prompter for MockPrompter {
        fn multi_select(&self, _prompt: &str, _items: &[&str]) -> anyhow::Result<Vec<usize>> {
            self.multi_select_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .pop_front()
                .expect("MockPrompter: no multi_select response queued")
        }

        fn select(&self, _prompt: &str, _items: &[&str]) -> anyhow::Result<usize> {
            self.select_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .pop_front()
                .expect("MockPrompter: no select response queued")
        }

        fn confirm(&self, _prompt: &str, _default: bool) -> anyhow::Result<bool> {
            self.confirm_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .pop_front()
                .expect("MockPrompter: no confirm response queued")
        }

        fn type_to_confirm(&self, _prompt: &str, _expected: &str) -> anyhow::Result<bool> {
            self.type_confirm_responses
                .lock()
                .expect("MockPrompter mutex poisoned")
                .pop_front()
                .expect("MockPrompter: no type_to_confirm response queued")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_multi_select_returns_configured_indices() {
        let mock = MockPrompter::new().with_multi_select(vec![0, 2]);

        assert_eq!(mock.multi_select("Select", &["a", "b", "c"]).unwrap(), vec![0, 2]);
        mock.assert_all_consumed();
    }

    #[test]
    fn test_mock_multi_select_error_injection() {
        let mock = MockPrompter::new().with_multi_select_err("user cancelled");

        let result = mock.multi_select("Select", &["a"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("user cancelled"));
    }

    #[test]
    #[should_panic(expected = "no multi_select response queued")]
    fn test_mock_multi_select_panics_when_exhausted() {
        let mock = MockPrompter::new();
        let _ = mock.multi_select("Select", &["a"]);
    }

    #[test]
    fn test_mock_select_returns_configured_index() {
        let mock = MockPrompter::new().with_select(1);

        assert_eq!(mock.select("Choose", &["a", "b", "c"]).unwrap(), 1);
        mock.assert_all_consumed();
    }

    #[test]
    fn test_mock_select_error_injection() {
        let mock = MockPrompter::new().with_select_err("cancelled");

        assert!(mock.select("Choose", &["a"]).is_err());
    }

    #[test]
    fn test_mock_confirm_returns_configured_values() {
        let mock = MockPrompter::new()
            .with_confirm(true)
            .with_confirm(false);

        assert!(mock.confirm("First?", false).unwrap());
        assert!(!mock.confirm("Second?", true).unwrap());
        mock.assert_all_consumed();
    }

    #[test]
    fn test_mock_confirm_error_injection() {
        let mock = MockPrompter::new().with_confirm_err("terminal error");

        let result = mock.confirm("?", true);
        assert!(result.is_err());
    }

    #[test]
    fn test_mock_type_confirm_returns_configured_values() {
        let mock = MockPrompter::new()
            .with_type_confirm(true)
            .with_type_confirm(false);

        assert!(mock.type_to_confirm("Type 'delete'", "delete").unwrap());
        assert!(!mock.type_to_confirm("Type 'delete'", "delete").unwrap());
        mock.assert_all_consumed();
    }

    #[test]
    fn test_mock_responses_consumed_in_order() {
        let mock = MockPrompter::new()
            .with_multi_select(vec![0])
            .with_multi_select(vec![1, 2]);

        assert_eq!(mock.multi_select("", &["a", "b", "c"]).unwrap(), vec![0]);
        assert_eq!(mock.multi_select("", &["a", "b", "c"]).unwrap(), vec![1, 2]);
        mock.assert_all_consumed();
    }

    #[test]
    fn test_mock_assert_all_consumed_passes_when_empty() {
        let mock = MockPrompter::new()
            .with_confirm(true)
            .with_select(0);

        let _ = mock.confirm("?", false);
        let _ = mock.select("?", &["a"]);

        mock.assert_all_consumed(); // Should not panic
    }

    #[test]
    #[should_panic(expected = "unconsumed responses")]
    fn test_mock_assert_all_consumed_panics_with_remaining() {
        let mock = MockPrompter::new()
            .with_confirm(true)
            .with_confirm(false);

        let _ = mock.confirm("?", false);
        // Second confirm not consumed

        mock.assert_all_consumed();
    }
}
