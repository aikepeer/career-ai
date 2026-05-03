//! Events fired by the pipeline. Adding a new variant is non-breaking:
//! channels render via the `Display`-shaped `summary()` + `title()`
//! helpers below, so unknown variants degrade to a generic line rather
//! than failing the build of every channel.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Events fired by the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotifyEvent {
    /// LinkedIn `li_at` cookie within 48h of expiry. Detected by
    /// `careerai digest`.
    CookieExpiringSoon { provider: String, hours_left: u64 },
    /// A discovery source returned an error mid-run (HTTP failure, DNS,
    /// auth, etc.).
    SourceUnreachable { source: String, reason: String },
    /// Cookie or token-based auth failed during a discovery / submit
    /// pass — distinct from a transport failure because the recovery is
    /// always "refresh the credential."
    AuthFailureMidRun { provider: String, action: String },
    /// Submitter encountered an unknown form field; operator must
    /// review the captured screenshot.
    ManualReviewNeeded {
        application_id: String,
        reason: String,
        screenshot_path: Option<PathBuf>,
    },
    /// Match score above the per-config threshold — same-day applies
    /// often pay off in this band.
    HighScoreMatch {
        listing_id: String,
        title: String,
        company: String,
        score: f32,
    },
    /// Governor permit denied because the daily cap is exhausted or the
    /// quiet-hours window is active. `next_window_seconds` is `0` when
    /// unknown.
    RateLimitExhausted {
        source: String,
        next_window_seconds: u64,
    },
    /// (M7 — wired in a future PR) the application got a response from
    /// the ATS / recruiter; classification is "reply" / "rejection" /
    /// "interview" / "unknown".
    ApplicationResponded {
        application_id: String,
        classification: String,
    },
}

impl NotifyEvent {
    /// Short human-readable title used by every channel.
    #[must_use]
    pub fn title(&self) -> String {
        match self {
            Self::CookieExpiringSoon { provider, .. } => {
                format!("[career-ai] {provider} cookie expiring soon")
            }
            Self::SourceUnreachable { source, .. } => {
                format!("[career-ai] source unreachable: {source}")
            }
            Self::AuthFailureMidRun { provider, .. } => {
                format!("[career-ai] auth failure mid-run: {provider}")
            }
            Self::ManualReviewNeeded { application_id, .. } => {
                format!("[career-ai] manual review needed: {application_id}")
            }
            Self::HighScoreMatch { title, company, .. } => {
                format!("[career-ai] high-score match: {title} @ {company}")
            }
            Self::RateLimitExhausted { source, .. } => {
                format!("[career-ai] rate-limit exhausted: {source}")
            }
            Self::ApplicationResponded {
                application_id,
                classification,
                ..
            } => format!("[career-ai] application {application_id} -> {classification}"),
        }
    }

    /// Plain-text body used by every channel that doesn't use a
    /// channel-native rich format.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::CookieExpiringSoon {
                provider,
                hours_left,
            } => format!(
                "{provider} cookie expires in ~{hours_left}h. Run \
                 `careerai cookies refresh {provider}` to renew."
            ),
            Self::SourceUnreachable { source, reason } => {
                format!("Discovery source {source} failed: {reason}")
            }
            Self::AuthFailureMidRun { provider, action } => format!(
                "Auth failure for {provider} during {action}. \
                 Refresh the credential and re-run."
            ),
            Self::ManualReviewNeeded {
                application_id,
                reason,
                screenshot_path,
            } => match screenshot_path {
                Some(p) => format!(
                    "Application {application_id} needs manual review: \
                     {reason}. Screenshot: {}",
                    p.display()
                ),
                None => format!("Application {application_id} needs manual review: {reason}"),
            },
            Self::HighScoreMatch {
                listing_id,
                title,
                company,
                score,
            } => format!(
                "High-score match {score:.2}: {title} @ {company} \
                 (listing {listing_id}). Same-day applies tend to convert."
            ),
            Self::RateLimitExhausted {
                source,
                next_window_seconds,
            } => {
                if *next_window_seconds == 0 {
                    format!("Rate-limit exhausted for {source}.")
                } else {
                    format!(
                        "Rate-limit exhausted for {source}. Next window in \
                         ~{next_window_seconds}s."
                    )
                }
            }
            Self::ApplicationResponded {
                application_id,
                classification,
            } => format!("Application {application_id} got response: {classification}."),
        }
    }
}
