//! Notification config. Lives under `notify:` in the workspace YAML.
//!
//! Secrets pattern:
//! - `*_env: ENV_VAR_NAME` reads from process env at channel-build
//!   time. Missing env var → channel disabled with a warn log.
//! - `*_keyring: <key>` reads from the OS keyring entry under the
//!   `career-ai` service. Missing entry → channel disabled.
//! - Inline secrets in YAML are intentionally unsupported.

use serde::{Deserialize, Serialize};

use crate::severity::Severity;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyConfig {
    /// Events with `severity < min_severity` are dropped before any
    /// channel is invoked. Defaults to `Warning` so a default install
    /// surfaces real problems but stays quiet for routine signals.
    pub min_severity: Severity,
    /// Per-channel toggles. A `None` entry disables that channel.
    pub channels: NotifyChannels,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyChannels {
    pub slack: Option<SlackConfig>,
    pub telegram: Option<TelegramConfig>,
    pub email: Option<EmailConfig>,
    pub ntfy: Option<NtfyConfig>,
}

/// Slack incoming-webhook channel.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SlackConfig {
    /// Env var holding the webhook URL. Resolved at startup; missing
    /// env var disables the channel with a warn log.
    pub webhook_url_env: String,
    /// Optional channel override. Slack normally uses the channel
    /// configured on the webhook itself; setting this overrides it.
    #[serde(default)]
    pub channel: Option<String>,
}

/// Telegram bot channel.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramConfig {
    /// Keyring entry under service `career-ai` holding the bot token.
    /// Example value: `"telegram/bot_token"`.
    pub bot_token_keyring: String,
    /// Numeric chat id (channel, group, or DM target). Stored as a
    /// string because Telegram supports negative IDs and `@channel`
    /// handles.
    pub chat_id: String,
}

/// Email channel via SMTP. Uses `lettre`. Credentials are looked up
/// from the OS keyring under service `career-ai`, key
/// `smtp/<smtp_host>/<smtp_username>`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailConfig {
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    pub smtp_username: String,
    /// `from` address used in the envelope. Must be a valid mailbox.
    pub from: String,
    /// Recipients. At least one required.
    pub to: Vec<String>,
    /// Disable TLS (testing only). Defaults to `false` — production
    /// always uses STARTTLS.
    #[serde(default)]
    pub starttls_disabled: bool,
}

fn default_smtp_port() -> u16 {
    587
}

/// `ntfy.sh` push channel — free, no auth, instant on phone via the
/// ntfy app. The cheapest "phone push" path.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NtfyConfig {
    /// Topic name. Use a private/random value (no auth).
    pub topic: String,
    /// Override the ntfy server. Defaults to `https://ntfy.sh`.
    #[serde(default)]
    pub server: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_yaml_parses_with_all_channels_disabled() {
        // Empty notify block — exercises every Default impl.
        let cfg: NotifyConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.channels.slack.is_none());
        assert!(cfg.channels.telegram.is_none());
        assert!(cfg.channels.email.is_none());
        assert!(cfg.channels.ntfy.is_none());
        assert_eq!(cfg.min_severity, Severity::Warning);
    }

    #[test]
    fn slack_only_yaml_parses() {
        let json = r#"{
            "min_severity": "info",
            "channels": {
                "slack": { "webhook_url_env": "FAKE" }
            }
        }"#;
        let cfg: NotifyConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.min_severity, Severity::Info);
        assert!(cfg.channels.slack.is_some());
        assert!(cfg.channels.telegram.is_none());
    }
}
