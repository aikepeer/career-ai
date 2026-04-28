//! Severity levels — ordered so `severity >= min_severity` is the
//! filter check used by `Pipeline::fire`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Routine signal. Default sink for `careerai notify test`.
    Info,
    /// Operator should look at this within a day (cookie expiring,
    /// rate-limit caps).
    #[default]
    Warning,
    /// Pipeline blocked / cookie already expired / submitter cannot
    /// proceed without a human.
    Critical,
}

impl Severity {
    /// Hex color for Slack attachments. Matches the spec:
    /// info=blue, warning=yellow, critical=red.
    #[must_use]
    pub fn slack_color(self) -> &'static str {
        match self {
            Self::Info => "#3aa3e3",
            Self::Warning => "#f2c744",
            Self::Critical => "#d72631",
        }
    }

    /// ASCII tag used in plaintext notifications.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warning => "WARN",
            Self::Critical => "CRIT",
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn severity_orders_low_to_high() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Critical);
    }

    #[test]
    fn severity_default_is_warning() {
        assert_eq!(Severity::default(), Severity::Warning);
    }

    #[test]
    fn severity_round_trips_json() {
        for s in [Severity::Info, Severity::Warning, Severity::Critical] {
            let j = serde_json::to_string(&s).unwrap();
            let back: Severity = serde_json::from_str(&j).unwrap();
            assert_eq!(s, back);
        }
    }
}
