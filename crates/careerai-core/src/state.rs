//! Pipeline state machine.
//!
//! Every listing moves through these states in order. `events` rows are
//! written on every transition (see `careerai-db`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListingState {
    Discovered,
    FilteredOut,
    Shortlisted,
    Tailored,
    Rendered,
    Prepared,
    Submitted,
    Skipped,
    Failed,
    Responded,
}

impl ListingState {
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::FilteredOut | Self::Submitted | Self::Skipped | Self::Failed | Self::Responded
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_states_are_marked_terminal() {
        assert!(ListingState::Submitted.is_terminal());
        assert!(ListingState::Failed.is_terminal());
        assert!(!ListingState::Discovered.is_terminal());
        assert!(!ListingState::Shortlisted.is_terminal());
    }
}
