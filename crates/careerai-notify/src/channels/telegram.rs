//! Telegram bot channel.
//!
//! Posts to `https://api.telegram.org/bot<token>/sendMessage`. Bot
//! token is read from the OS keyring under service `career-ai`, key
//! configured via `TelegramConfig::bot_token_keyring`. Reliable, free,
//! instant on phone.

use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

use crate::{
    config::TelegramConfig, error::NotifyError, severity::Severity, Notifier, NotifyEvent,
    HTTP_TIMEOUT,
};

const KEYRING_SERVICE: &str = "career-ai";
/// Telegram cap is 4096; we leave headroom for HTML tags.
const MAX_MESSAGE_LEN: usize = 3500;

pub struct TelegramNotifier {
    /// API base URL (`https://api.telegram.org` in prod). Stored
    /// without the bot token suffix so wiremock tests can override it.
    base_url: String,
    /// Bot token. Held in memory only; never logged.
    bot_token: String,
    chat_id: String,
    client: Client,
}

impl std::fmt::Debug for TelegramNotifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Bot token is a bearer credential — never echo it via Debug.
        f.debug_struct("TelegramNotifier")
            .field("base_url", &self.base_url)
            .field("bot_token", &"<redacted>")
            .field("chat_id", &self.chat_id)
            .finish_non_exhaustive()
    }
}

impl TelegramNotifier {
    pub fn from_config(cfg: &TelegramConfig) -> Result<Self, NotifyError> {
        if cfg.bot_token_keyring.trim().is_empty() {
            return Err(NotifyError::Config(
                "telegram: bot_token_keyring must be set".into(),
            ));
        }
        if cfg.chat_id.trim().is_empty() {
            return Err(NotifyError::Config("telegram: chat_id is required".into()));
        }
        let entry = keyring::Entry::new(KEYRING_SERVICE, &cfg.bot_token_keyring).map_err(|e| {
            NotifyError::SecretMissing(format!("telegram: keyring entry build failed: {e}"))
        })?;
        let token = entry.get_password().map_err(|e| {
            NotifyError::SecretMissing(format!(
                "telegram: keyring entry `{}` not readable: {e}",
                cfg.bot_token_keyring
            ))
        })?;
        if token.trim().is_empty() {
            return Err(NotifyError::SecretMissing(format!(
                "telegram: keyring entry `{}` is empty",
                cfg.bot_token_keyring
            )));
        }
        let client = Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| NotifyError::Http(e.to_string()))?;
        Ok(Self {
            base_url: "https://api.telegram.org".into(),
            bot_token: token,
            chat_id: cfg.chat_id.clone(),
            client,
        })
    }

    /// Test-only constructor that bypasses keyring lookup.
    #[doc(hidden)]
    #[must_use]
    pub fn __test_new(
        base_url: String,
        bot_token: String,
        chat_id: String,
        client: Client,
    ) -> Self {
        Self {
            base_url,
            bot_token,
            chat_id,
            client,
        }
    }

    /// Format the message body as HTML. Telegram parses a small subset
    /// of HTML; we use `<b>` for the title and a plain second
    /// paragraph for the summary. Severity is prefixed in brackets so
    /// the line is greppable on a phone.
    pub(crate) fn render_html(event: &NotifyEvent, severity: Severity) -> String {
        let mut out = format!(
            "<b>[{}] {}</b>\n{}",
            severity.tag(),
            html_escape(&event.title()),
            html_escape(&event.summary()),
        );
        if out.len() > MAX_MESSAGE_LEN {
            out.truncate(MAX_MESSAGE_LEN);
            out.push('…');
        }
        out
    }
}

#[async_trait]
impl Notifier for TelegramNotifier {
    fn name(&self) -> &'static str {
        "telegram"
    }

    async fn notify(&self, event: &NotifyEvent, severity: Severity) -> Result<(), NotifyError> {
        let url = format!("{}/bot{}/sendMessage", self.base_url, self.bot_token);
        let body = json!({
            "chat_id": self.chat_id,
            "text": Self::render_html(event, severity),
            "parse_mode": "HTML",
            "disable_web_page_preview": true,
        });
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| NotifyError::Channel {
                channel: "telegram",
                reason: scrub_token(&e.to_string()),
            })?;
        if !resp.status().is_success() {
            return Err(NotifyError::Channel {
                channel: "telegram",
                reason: format!("HTTP {}", resp.status()),
            });
        }
        Ok(())
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Drop any `bot<token>` segment from a string before logging. A
/// Telegram bot token has the shape `<digits>:<alnum/_-/+>` — match
/// only when the substring after `bot` looks like a token to avoid
/// mangling unrelated words (`robot`, `sandbox`, etc.).
fn scrub_token(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for (idx, _) in s.match_indices("bot") {
        let after = &s[idx + 3..];
        // A token starts with at least one digit before the colon.
        let looks_like_token =
            after.chars().next().is_some_and(|c| c.is_ascii_digit()) && after.contains(':');
        if !looks_like_token {
            continue;
        }
        out.push_str(&s[last..idx]);
        out.push_str("bot<redacted>");
        // Token ends at the next `/`, whitespace, or quote.
        let stop = after
            .find(|c: char| c == '/' || c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(after.len());
        last = idx + 3 + stop;
    }
    out.push_str(&s[last..]);
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn ev() -> NotifyEvent {
        NotifyEvent::AuthFailureMidRun {
            provider: "linkedin".into(),
            action: "discover".into(),
        }
    }

    #[test]
    fn debug_output_redacts_bot_token() {
        let n = TelegramNotifier {
            base_url: "https://api.telegram.org".into(),
            bot_token: "1111:SUPERSECRETTGBOTKEY".into(),
            chat_id: "12345".into(),
            client: Client::builder().timeout(HTTP_TIMEOUT).build().unwrap(),
        };
        let dbg = format!("{n:?}");
        assert!(
            !dbg.contains("SUPERSECRETTGBOTKEY"),
            "bot token leaked into Debug output: {dbg}"
        );
        assert!(
            dbg.contains("<redacted>"),
            "missing redaction marker: {dbg}"
        );
    }

    #[test]
    fn scrub_token_ignores_words_that_just_start_with_bot() {
        // The earlier implementation matched any "bot" substring, which
        // mangled e.g. "robot.txt". Pin the regression: only match real
        // tokens (digits-then-colon).
        let s = "robot scrubbed sandbox bot12345:ABCDE/sendMessage and bot999:ZZ done";
        let out = scrub_token(s);
        assert!(out.contains("robot"), "innocent word mangled: {out}");
        assert!(out.contains("sandbox"), "innocent word mangled: {out}");
        assert!(!out.contains("12345:ABCDE"), "leaked: {out}");
        assert!(!out.contains("999:ZZ"), "leaked: {out}");
    }

    #[test]
    fn render_html_escapes_special_chars() {
        let evt = NotifyEvent::SourceUnreachable {
            source: "<weird>".into(),
            reason: "&boom".into(),
        };
        let html = TelegramNotifier::render_html(&evt, Severity::Critical);
        assert!(html.contains("[CRIT]"));
        assert!(!html.contains("<weird>"), "raw < not escaped: {html}");
        assert!(html.contains("&lt;weird&gt;"));
        assert!(html.contains("&amp;boom"));
    }

    #[test]
    fn scrub_token_removes_bot_segment() {
        let s = "failed POST https://api.telegram.org/bot12345:ABCDE/sendMessage timeout";
        let out = scrub_token(s);
        assert!(!out.contains("12345:ABCDE"), "leaked token: {out}");
        assert!(out.contains("bot<redacted>"));
    }

    #[tokio::test]
    async fn telegram_posts_to_send_message_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(|req: &Request| {
                let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
                assert_eq!(body["parse_mode"], "HTML");
                assert_eq!(body["chat_id"], "12345");
                ResponseTemplate::new(200).set_body_json(json!({"ok": true}))
            })
            .expect(1)
            .mount(&server)
            .await;

        let n = TelegramNotifier {
            base_url: server.uri(),
            bot_token: "secret-token".into(),
            chat_id: "12345".into(),
            client: Client::builder().timeout(HTTP_TIMEOUT).build().unwrap(),
        };
        n.notify(&ev(), Severity::Warning).await.unwrap();
    }
}
