//! Pipeline state machine.
//!
//! Every listing moves through these states in order. `events` rows are
//! written on every transition (see `careerai-db`).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ListingState {
    Discovered,
    FilteredOut,
    Shortlisted,
    Tailored,
    Rendered,
    Prepared,
    Drafted,
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

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::FilteredOut => "filtered_out",
            Self::Shortlisted => "shortlisted",
            Self::Tailored => "tailored",
            Self::Rendered => "rendered",
            Self::Prepared => "prepared",
            Self::Drafted => "drafted",
            Self::Submitted => "submitted",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::Responded => "responded",
        }
    }
}

impl fmt::Display for ListingState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown listing state: {0}")]
pub struct ListingStateParseError(pub String);

impl FromStr for ListingState {
    type Err = ListingStateParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "discovered" => Ok(Self::Discovered),
            "filtered_out" => Ok(Self::FilteredOut),
            "shortlisted" => Ok(Self::Shortlisted),
            "tailored" => Ok(Self::Tailored),
            "rendered" => Ok(Self::Rendered),
            "prepared" => Ok(Self::Prepared),
            "drafted" => Ok(Self::Drafted),
            "submitted" => Ok(Self::Submitted),
            "skipped" => Ok(Self::Skipped),
            "failed" => Ok(Self::Failed),
            "responded" => Ok(Self::Responded),
            other => Err(ListingStateParseError(other.to_string())),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn terminal_states_are_marked_terminal() {
        assert!(ListingState::Submitted.is_terminal());
        assert!(ListingState::Failed.is_terminal());
        assert!(!ListingState::Discovered.is_terminal());
        assert!(!ListingState::Shortlisted.is_terminal());
    }

    #[test]
    fn roundtrip_via_string() {
        for s in [
            ListingState::Discovered,
            ListingState::FilteredOut,
            ListingState::Shortlisted,
            ListingState::Tailored,
            ListingState::Rendered,
            ListingState::Prepared,
            ListingState::Drafted,
            ListingState::Submitted,
            ListingState::Skipped,
            ListingState::Failed,
            ListingState::Responded,
        ] {
            let back: ListingState = s.as_str().parse().unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn drafted_serde_roundtrip() {
        let s = ListingState::Drafted;
        let yaml = serde_yaml::to_string(&s).unwrap();
        assert_eq!(yaml.trim(), "drafted");
        let back: ListingState = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn drafted_as_str_is_drafted() {
        assert_eq!(ListingState::Drafted.as_str(), "drafted");
    }

    #[test]
    fn drafted_parses_from_str() {
        assert_eq!(
            "drafted".parse::<ListingState>().unwrap(),
            ListingState::Drafted
        );
    }
}
