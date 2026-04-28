//! Email channel via SMTP (using `lettre`).
//!
//! SMTP password is read from the OS keyring under service `career-ai`
//! and key `smtp/<smtp_host>/<smtp_username>`. Inline secrets in the
//! YAML config are intentionally unsupported.
//!
//! `Notifier::notify` builds the message and hands it to a `lettre`
//! `AsyncSmtpTransport`. STARTTLS by default; the `starttls_disabled`
//! escape hatch exists for plain-text local SMTP testing only.

use async_trait::async_trait;
use lettre::message::{header::ContentType, Mailbox};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::{
    config::EmailConfig, error::NotifyError, severity::Severity, Notifier, NotifyEvent,
    HTTP_TIMEOUT,
};

const KEYRING_SERVICE: &str = "career-ai";

/// Parsed SMTP config. NOTE: hand-rolled `Debug` below — derived Debug
/// would print `smtp_password` verbatim. Same for `EmailNotifier`.
#[derive(Clone)]
struct ParsedConfig {
    smtp_host: String,
    smtp_port: u16,
    smtp_username: String,
    /// Held in memory only. Never logged.
    smtp_password: String,
    from: Mailbox,
    to: Vec<Mailbox>,
    starttls_disabled: bool,
}

impl std::fmt::Debug for ParsedConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedConfig")
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("smtp_username", &self.smtp_username)
            .field("smtp_password", &"<redacted>")
            .field("from", &self.from)
            .field("to", &self.to)
            .field("starttls_disabled", &self.starttls_disabled)
            .finish()
    }
}

pub struct EmailNotifier {
    parsed: ParsedConfig,
    /// Cached transport — building one is non-trivial (TLS context,
    /// resolver) and a daemon may fire many notifications. Reuse keeps
    /// connection state warm and avoids per-call setup.
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl std::fmt::Debug for EmailNotifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Delegate to ParsedConfig::Debug, which redacts the password.
        // The transport itself does not implement Debug.
        f.debug_struct("EmailNotifier")
            .field("parsed", &self.parsed)
            .finish()
    }
}

impl EmailNotifier {
    pub fn from_config(cfg: &EmailConfig) -> Result<Self, NotifyError> {
        if cfg.smtp_host.trim().is_empty() {
            return Err(NotifyError::Config("email: smtp_host is required".into()));
        }
        if cfg.smtp_username.trim().is_empty() {
            return Err(NotifyError::Config(
                "email: smtp_username is required".into(),
            ));
        }
        if cfg.from.trim().is_empty() {
            return Err(NotifyError::Config("email: from is required".into()));
        }
        if cfg.to.is_empty() {
            return Err(NotifyError::Config(
                "email: at least one to address required".into(),
            ));
        }
        let key = keyring_key(&cfg.smtp_host, &cfg.smtp_username);
        let entry = keyring::Entry::new(KEYRING_SERVICE, &key).map_err(|e| {
            NotifyError::SecretMissing(format!("email: keyring entry build failed: {e}"))
        })?;
        let pass = entry.get_password().map_err(|e| {
            NotifyError::SecretMissing(format!("email: keyring entry `{key}` not readable: {e}"))
        })?;

        let from: Mailbox = cfg
            .from
            .parse()
            .map_err(|e| NotifyError::Config(format!("email: from address invalid: {e}")))?;
        let mut to_list = Vec::with_capacity(cfg.to.len());
        for addr in &cfg.to {
            to_list
                .push(addr.parse::<Mailbox>().map_err(|e| {
                    NotifyError::Config(format!("email: to `{addr}` invalid: {e}"))
                })?);
        }

        let parsed = ParsedConfig {
            smtp_host: cfg.smtp_host.clone(),
            smtp_port: cfg.smtp_port,
            smtp_username: cfg.smtp_username.clone(),
            smtp_password: pass,
            from,
            to: to_list,
            starttls_disabled: cfg.starttls_disabled,
        };
        let transport = build_transport(&parsed)?;
        Ok(Self { parsed, transport })
    }

    /// Compose the `Message` for `event` at `severity`. Pure function;
    /// extracted for unit testing.
    pub(crate) fn build_message(&self, event: &NotifyEvent, severity: Severity) -> Message {
        let mut builder = Message::builder()
            .from(self.parsed.from.clone())
            .subject(format!("[{}] {}", severity.tag(), event.title()))
            .header(ContentType::TEXT_PLAIN);
        for to in &self.parsed.to {
            builder = builder.to(to.clone());
        }
        // unwrap is benign: header validity already enforced by the
        // builder; body is plain text.
        #[allow(clippy::expect_used)]
        builder
            .body(event.summary())
            .expect("lettre Message::body cannot fail for plain string body")
    }

}

fn build_transport(parsed: &ParsedConfig) -> Result<AsyncSmtpTransport<Tokio1Executor>, NotifyError> {
    let creds = Credentials::new(parsed.smtp_username.clone(), parsed.smtp_password.clone());
    let builder = if parsed.starttls_disabled {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&parsed.smtp_host)
            .port(parsed.smtp_port)
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&parsed.smtp_host)
            .map_err(|e| NotifyError::Config(format!("email: starttls relay: {e}")))?
            .port(parsed.smtp_port)
    };
    Ok(builder
        .credentials(creds)
        .timeout(Some(HTTP_TIMEOUT))
        .build())
}

#[async_trait]
impl Notifier for EmailNotifier {
    fn name(&self) -> &'static str {
        "email"
    }

    async fn notify(&self, event: &NotifyEvent, severity: Severity) -> Result<(), NotifyError> {
        let message = self.build_message(event, severity);
        self.transport
            .send(message)
            .await
            .map_err(|e| NotifyError::Channel {
                channel: "email",
                // lettre errors generally don't include credentials; we
                // use the Display impl, not Debug, to keep that
                // invariant.
                reason: e.to_string(),
            })?;
        Ok(())
    }
}

fn keyring_key(host: &str, username: &str) -> String {
    format!("smtp/{host}/{username}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn cfg() -> ParsedConfig {
        ParsedConfig {
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
            smtp_username: "user".into(),
            smtp_password: "ULTRA-SECRET-PASSWORD".into(),
            from: "career-ai@example.com".parse().unwrap(),
            to: vec!["op@example.com".parse().unwrap()],
            starttls_disabled: false,
        }
    }

    fn ev() -> NotifyEvent {
        NotifyEvent::ManualReviewNeeded {
            application_id: "app-1".into(),
            reason: "unknown form field".into(),
            screenshot_path: None,
        }
    }

    fn notifier_for(parsed: ParsedConfig) -> EmailNotifier {
        let transport = build_transport(&parsed).expect("test transport build");
        EmailNotifier { parsed, transport }
    }

    #[test]
    fn build_message_has_severity_tag_in_subject() {
        let n = notifier_for(cfg());
        let msg = n.build_message(&ev(), Severity::Critical);
        let raw = String::from_utf8(msg.formatted()).unwrap();
        assert!(
            raw.contains("Subject: [CRIT]"),
            "subject missing tag: {raw}"
        );
        assert!(raw.contains("manual review needed"));
        assert!(
            !raw.contains("ULTRA-SECRET-PASSWORD"),
            "password leaked into formatted message: {raw}"
        );
    }

    #[test]
    fn debug_output_redacts_smtp_password() {
        // Pin the invariant: the SMTP password must never appear in
        // `{:?}` output. A future `#[derive(Debug)]` regression on
        // `ParsedConfig` or `EmailNotifier` would flip this test red.
        let n = notifier_for(cfg());
        let dbg = format!("{n:?}");
        assert!(
            !dbg.contains("ULTRA-SECRET-PASSWORD"),
            "smtp password leaked into Debug output: {dbg}"
        );
        assert!(
            dbg.contains("<redacted>"),
            "expected redaction marker in Debug output: {dbg}"
        );
    }

    #[test]
    fn keyring_key_is_stable() {
        assert_eq!(
            keyring_key("smtp.gmail.com", "me"),
            "smtp/smtp.gmail.com/me"
        );
    }
}
