//! Slack incoming-webhook channel.
//!
//! Block Kit JSON with a severity-colored attachment. 5s timeout via
//! `reqwest`. Webhook URL is resolved from the env var named in
//! `SlackConfig::webhook_url_env`; the URL itself is never logged.

use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

use crate::{
    config::SlackConfig, error::NotifyError, severity::Severity, Notifier, NotifyEvent,
    HTTP_TIMEOUT,
};

#[derive(Debug)]
pub struct SlackNotifier {
    webhook_url: String,
    channel: Option<String>,
    client: Client,
}

impl SlackNotifier {
    /// Build from config. Returns `Err` if the env var named by
    /// `webhook_url_env` is unset; callers in `Pipeline::from_config`
    /// log that as a `warn` and skip the channel.
    pub fn from_config(cfg: &SlackConfig) -> Result<Self, NotifyError> {
        if cfg.webhook_url_env.trim().is_empty() {
            return Err(NotifyError::Config(
                "slack: webhook_url_env must name an env variable".into(),
            ));
        }
        let webhook_url = std::env::var(&cfg.webhook_url_env).map_err(|_| {
            NotifyError::SecretMissing(format!(
                "slack: env var `{}` is unset",
                cfg.webhook_url_env
            ))
        })?;
        if webhook_url.trim().is_empty() {
            return Err(NotifyError::SecretMissing(format!(
                "slack: env var `{}` is empty",
                cfg.webhook_url_env
            )));
        }
        let client = Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| NotifyError::Http(e.to_string()))?;
        Ok(Self {
            webhook_url,
            channel: cfg.channel.clone(),
            client,
        })
    }

    /// Test-only constructor that bypasses env-var resolution. Hidden
    /// from rustdoc and only used by integration tests in this crate.
    #[doc(hidden)]
    #[must_use]
    pub fn __test_new(webhook_url: String, channel: Option<String>, client: Client) -> Self {
        Self {
            webhook_url,
            channel,
            client,
        }
    }

    /// Build a Block Kit payload for `event` at `severity`.
    pub(crate) fn build_payload(
        event: &NotifyEvent,
        severity: Severity,
        channel: Option<&str>,
    ) -> serde_json::Value {
        let title = event.title();
        let summary = event.summary();
        let mut payload = json!({
            "text": title,
            "attachments": [{
                "color": severity.slack_color(),
                "blocks": [
                    {
                        "type": "header",
                        "text": { "type": "plain_text", "text": title, "emoji": true }
                    },
                    {
                        "type": "section",
                        "text": { "type": "mrkdwn", "text": summary }
                    },
                    {
                        "type": "context",
                        "elements": [
                            {
                                "type": "mrkdwn",
                                "text": format!("severity: *{}*", severity.tag())
                            }
                        ]
                    }
                ]
            }]
        });
        if let Some(ch) = channel {
            payload["channel"] = json!(ch);
        }
        payload
    }
}

#[async_trait]
impl Notifier for SlackNotifier {
    fn name(&self) -> &'static str {
        "slack"
    }

    async fn notify(&self, event: &NotifyEvent, severity: Severity) -> Result<(), NotifyError> {
        let body = Self::build_payload(event, severity, self.channel.as_deref());
        let resp = self
            .client
            .post(&self.webhook_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| NotifyError::Channel {
                channel: "slack",
                // reqwest errors include the URL only when the user
                // builds with `error-for-status` patterns; in our
                // configuration the message stays generic. We still
                // strip the URL defensively.
                reason: scrub(&e.to_string()),
            })?;
        if !resp.status().is_success() {
            return Err(NotifyError::Channel {
                channel: "slack",
                reason: format!("HTTP {}", resp.status()),
            });
        }
        Ok(())
    }
}

/// Belt-and-suspenders: if reqwest ever embeds the webhook URL in an
/// error string, drop the URL token. Cheap, defensive.
fn scrub(s: &str) -> String {
    s.split_whitespace()
        .filter(|tok| !tok.contains("hooks.slack.com"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn ev() -> NotifyEvent {
        NotifyEvent::HighScoreMatch {
            listing_id: "abc-123".into(),
            title: "Senior Rust Engineer".into(),
            company: "Acme".into(),
            score: 0.92,
        }
    }

    #[test]
    fn payload_includes_severity_color_and_title() {
        let v = SlackNotifier::build_payload(&ev(), Severity::Critical, None);
        let attach = &v["attachments"][0];
        assert_eq!(attach["color"], "#d72631");
        let header = &attach["blocks"][0]["text"]["text"];
        assert!(
            header.as_str().unwrap().contains("Senior Rust Engineer"),
            "header missing job title: {header}"
        );
    }

    #[test]
    fn payload_sets_channel_override_when_configured() {
        let v = SlackNotifier::build_payload(&ev(), Severity::Info, Some("#alerts"));
        assert_eq!(v["channel"], "#alerts");
    }

    #[tokio::test]
    async fn slack_posts_to_webhook_and_handles_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/services/X/Y/Z"))
            .respond_with(|req: &Request| {
                let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
                assert!(body["attachments"][0]["color"].as_str().unwrap().starts_with('#'));
                ResponseTemplate::new(200)
            })
            .expect(1)
            .mount(&server)
            .await;

        let n = SlackNotifier {
            webhook_url: format!("{}/services/X/Y/Z", server.uri()),
            channel: None,
            client: Client::builder().timeout(HTTP_TIMEOUT).build().unwrap(),
        };
        n.notify(&ev(), Severity::Warning).await.unwrap();
    }

    #[tokio::test]
    async fn slack_returns_channel_error_on_non_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let n = SlackNotifier {
            webhook_url: format!("{}/hook", server.uri()),
            channel: None,
            client: Client::builder().timeout(HTTP_TIMEOUT).build().unwrap(),
        };
        let err = n.notify(&ev(), Severity::Warning).await.unwrap_err();
        match err {
            NotifyError::Channel { channel, .. } => assert_eq!(channel, "slack"),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
