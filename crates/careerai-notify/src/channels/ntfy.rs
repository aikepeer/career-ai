//! `ntfy.sh` push channel.
//!
//! Plain HTTP POST to `https://ntfy.sh/<topic>` with the message body.
//! Free, no auth, instant on phone via the ntfy app — the cheapest
//! "push to phone" path. Title goes in the `Title` header, severity in
//! `Priority` (1..5 per ntfy spec), and `Tags` so the phone shows a
//! relevant emoji.

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::Client;

use crate::{
    config::NtfyConfig, error::NotifyError, severity::Severity, Notifier, NotifyEvent,
    HTTP_TIMEOUT,
};

const DEFAULT_SERVER: &str = "https://ntfy.sh";

#[derive(Debug)]
pub struct NtfyNotifier {
    server: String,
    topic: String,
    client: Client,
}

impl NtfyNotifier {
    pub fn from_config(cfg: &NtfyConfig) -> Result<Self, NotifyError> {
        if cfg.topic.trim().is_empty() {
            return Err(NotifyError::Config("ntfy: topic is required".into()));
        }
        let server = cfg
            .server
            .as_deref()
            .map_or_else(|| DEFAULT_SERVER.to_string(), str::to_string);
        let client = Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| NotifyError::Http(e.to_string()))?;
        Ok(Self {
            server,
            topic: cfg.topic.clone(),
            client,
        })
    }

    /// Test-only constructor that bypasses config validation.
    #[doc(hidden)]
    #[must_use]
    pub fn __test_new(server: String, topic: String, client: Client) -> Self {
        Self {
            server,
            topic,
            client,
        }
    }

    fn headers_for(event: &NotifyEvent, severity: Severity) -> Result<HeaderMap, NotifyError> {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        h.insert(
            HeaderName::from_static("title"),
            HeaderValue::from_str(&header_safe(&event.title()))
                .map_err(|e| NotifyError::Config(format!("ntfy title invalid: {e}")))?,
        );
        h.insert(
            HeaderName::from_static("priority"),
            HeaderValue::from_static(match severity {
                Severity::Info => "2",
                Severity::Warning => "3",
                Severity::Critical => "5",
            }),
        );
        h.insert(
            HeaderName::from_static("tags"),
            HeaderValue::from_static(match severity {
                Severity::Info => "information_source",
                Severity::Warning => "warning",
                Severity::Critical => "rotating_light",
            }),
        );
        Ok(h)
    }
}

#[async_trait]
impl Notifier for NtfyNotifier {
    fn name(&self) -> &'static str {
        "ntfy"
    }

    async fn notify(&self, event: &NotifyEvent, severity: Severity) -> Result<(), NotifyError> {
        let url = format!("{}/{}", self.server.trim_end_matches('/'), self.topic);
        let headers = Self::headers_for(event, severity)?;
        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .body(event.summary())
            .send()
            .await
            .map_err(|e| NotifyError::Channel {
                channel: "ntfy",
                reason: e.to_string(),
            })?;
        if !resp.status().is_success() {
            return Err(NotifyError::Channel {
                channel: "ntfy",
                reason: format!("HTTP {}", resp.status()),
            });
        }
        Ok(())
    }
}

/// HTTP headers reject control characters and non-ASCII bytes. Strip
/// them to keep the title delivery robust.
fn header_safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii() && !c.is_control() {
                c
            } else {
                ' '
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn ev() -> NotifyEvent {
        NotifyEvent::CookieExpiringSoon {
            provider: "linkedin".into(),
            hours_left: 12,
        }
    }

    #[test]
    fn header_safe_strips_non_ascii() {
        let s = header_safe("café");
        assert_eq!(s, "caf ");
    }

    #[test]
    fn priority_rises_with_severity() {
        let h_info = NtfyNotifier::headers_for(&ev(), Severity::Info).unwrap();
        let h_crit = NtfyNotifier::headers_for(&ev(), Severity::Critical).unwrap();
        assert_eq!(h_info["priority"], "2");
        assert_eq!(h_crit["priority"], "5");
    }

    #[tokio::test]
    async fn ntfy_posts_to_topic_with_priority_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/my-topic"))
            .respond_with(|req: &Request| {
                let prio = req
                    .headers
                    .get("priority")
                    .map(|v| v.to_str().unwrap().to_string())
                    .unwrap_or_default();
                assert!(!prio.is_empty(), "missing priority header");
                ResponseTemplate::new(200)
            })
            .expect(1)
            .mount(&server)
            .await;

        let n = NtfyNotifier {
            server: server.uri(),
            topic: "my-topic".into(),
            client: Client::builder().timeout(HTTP_TIMEOUT).build().unwrap(),
        };
        n.notify(&ev(), Severity::Critical).await.unwrap();
    }
}
