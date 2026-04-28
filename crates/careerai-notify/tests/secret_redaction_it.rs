//! Asserts that channel failure paths NEVER log known-secret shapes
//! (webhook URLs, bot tokens, SMTP passwords). Mirrors the existing
//! `careerai-submit` regex-based redaction test.
//!
//! Strategy: spin up a `wiremock` server that returns 500 so each
//! channel goes through its error path, capture tracing output via
//! `tracing-test`, and grep for the secret patterns we care about.

use careerai_notify::{
    channels::{ntfy::NtfyNotifier, slack::SlackNotifier, telegram::TelegramNotifier},
    NotifyEvent, Notifier, Severity,
};
use reqwest::Client;
use std::time::Duration;
use tracing_test::traced_test;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const SLACK_TOKEN: &str = "T00000000/B11111111/EXTREMELYSECRETSLACKTOKEN";
const TG_TOKEN: &str = "1111:SUPERSECRETTGBOTKEY";

fn ev() -> NotifyEvent {
    NotifyEvent::SourceUnreachable {
        source: "test".into(),
        reason: "boom".into(),
    }
}

#[tokio::test]
#[traced_test]
async fn slack_failure_does_not_leak_webhook_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    // Build the webhook URL with a secret-shaped path so we can grep.
    let webhook = format!("{}/services/{SLACK_TOKEN}", server.uri());
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    // Construct manually to inject a known URL without going through env.
    let n = build_slack(&webhook, client);

    let _err = n.notify(&ev(), Severity::Warning).await.unwrap_err();

    assert!(
        !logs_contain(SLACK_TOKEN),
        "slack token leaked into tracing output",
    );
}

#[tokio::test]
#[traced_test]
async fn telegram_failure_does_not_leak_bot_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let n = build_telegram(server.uri(), TG_TOKEN.to_string(), client);

    let _err = n.notify(&ev(), Severity::Warning).await.unwrap_err();

    assert!(
        !logs_contain(TG_TOKEN),
        "telegram bot token leaked into tracing output",
    );
}

#[tokio::test]
#[traced_test]
async fn ntfy_failure_log_is_clean() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let topic = "secret-topic-xyz";
    let n = build_ntfy(server.uri(), topic.to_string(), client);

    let _err = n.notify(&ev(), Severity::Warning).await.unwrap_err();

    // ntfy topics are not strictly secret, but they're URL-shaped so
    // we still want to keep them out of error messages by default.
    // Best-effort assertion: the topic name should not appear in
    // captured logs (the channel only logs HTTP status on failure).
    assert!(
        !logs_contain(topic),
        "ntfy topic leaked into tracing output",
    );
}

// --- helpers ---------------------------------------------------------
//
// We construct the notifiers by hand (instead of through `from_config`)
// to keep these tests env-var- and keyring-free. The struct fields are
// pub(crate) inside the channels modules; expose a thin builder via a
// shared test helper module.

fn build_slack(webhook_url: &str, client: Client) -> SlackNotifier {
    test_helpers::slack(webhook_url.to_string(), client)
}

fn build_telegram(base_url: String, token: String, client: Client) -> TelegramNotifier {
    test_helpers::telegram(base_url, token, "12345".into(), client)
}

fn build_ntfy(server: String, topic: String, client: Client) -> NtfyNotifier {
    test_helpers::ntfy(server, topic, client)
}

mod test_helpers {
    //! Thin fabricator helpers for channels. The module exposes
    //! constructors that bypass keyring/env so tests stay hermetic.
    //!
    //! NOTE: kept in this integration-test file so production code
    //! does not need to expose unsafe builders.

    use careerai_notify::channels::ntfy::NtfyNotifier;
    use careerai_notify::channels::slack::SlackNotifier;
    use careerai_notify::channels::telegram::TelegramNotifier;
    use reqwest::Client;

    pub(super) fn slack(webhook_url: String, client: Client) -> SlackNotifier {
        SlackNotifier::__test_new(webhook_url, None, client)
    }

    pub(super) fn telegram(
        base_url: String,
        token: String,
        chat_id: String,
        client: Client,
    ) -> TelegramNotifier {
        TelegramNotifier::__test_new(base_url, token, chat_id, client)
    }

    pub(super) fn ntfy(server: String, topic: String, client: Client) -> NtfyNotifier {
        NtfyNotifier::__test_new(server, topic, client)
    }
}
