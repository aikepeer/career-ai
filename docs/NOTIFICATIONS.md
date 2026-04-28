# Notifications

career-ai pings the operator outside the CLI when something needs
attention — cookies expiring, a discovery source going unreachable,
a high-score match worth applying same-day, rate-limits exhausted,
etc. The notification pipeline is best-effort: a single broken
webhook never breaks a daemon tick, and channels are independent.

This document covers:

- [Quick start](#quick-start)
- [Channels](#channels)
  - [Slack](#slack)
  - [Telegram](#telegram)
  - [Email (SMTP)](#email-smtp)
  - [ntfy.sh](#ntfy)
- [Event reference — what gets sent when](#event-reference)
- [Severities](#severities)
- [Adding a custom channel](#adding-a-custom-channel)

## Quick start

1. Pick at least one channel. ntfy.sh is the cheapest path to a
   phone push (no auth, free).
2. Add a `notify.channels.<channel>` block to
   `config/local.yaml` — never edit `config/default.yaml`, and
   never inline secrets in YAML. Every channel resolves
   credentials from environment variables or the OS keyring.
3. Run:
   ```
   careerai notify test
   ```
   The command fires a synthetic `SourceUnreachable` event at
   `Severity::Info` through every configured channel and exits
   non-zero if no channels are wired.

## Channels

### Slack

Slack is the lightest-weight team-visible channel. Setup uses
Slack's incoming-webhook integration:

1. In Slack: **Apps → Incoming Webhooks → Add to Workspace**.
   Pick the channel that should receive alerts; Slack returns a
   webhook URL of the shape `https://hooks.slack.com/services/T.../B.../...`.
2. Export the URL in the shell that runs `careerai`:
   ```sh
   export CAREERAI_SLACK_WEBHOOK="https://hooks.slack.com/services/..."
   ```
3. In `config/local.yaml`:
   ```yaml
   notify:
     min_severity: warning
     channels:
       slack:
         webhook_url_env: CAREERAI_SLACK_WEBHOOK
         # channel: "#alerts"   # optional override
   ```
4. Verify: `careerai notify test`.

career-ai posts Block Kit JSON with a severity-colored attachment
(blue / yellow / red for info / warning / critical) and a
`severity: *WARN*` context line.

### Telegram

Telegram is reliable, free, and instant on phone via the official
client.

1. Create a bot: open `@BotFather` on Telegram, send `/newbot`,
   follow the prompts. BotFather returns a bot token of the form
   `1234567890:ABCdefGHIjklMNOpqrSTUvwxYZ`.
2. Get your chat id: send any message to the bot, then visit
   `https://api.telegram.org/bot<TOKEN>/getUpdates` in a browser.
   The numeric `chat.id` is your destination. (Group / channel
   ids are negative.)
3. Store the bot token in the OS keyring under service
   `career-ai`. On Linux (Secret Service) you can use
   `secret-tool`:
   ```sh
   secret-tool store --label="career-ai telegram" service career-ai username telegram/bot_token
   # paste the token, hit Enter
   ```
   On macOS use **Keychain Access → File → New Password Item**
   with account `telegram/bot_token` and the bot token as
   password.
4. In `config/local.yaml`:
   ```yaml
   notify:
     channels:
       telegram:
         bot_token_keyring: telegram/bot_token
         chat_id: "123456789"
   ```
5. Verify: `careerai notify test`.

The channel posts HTML-formatted messages and disables Telegram
URL previews to keep alerts compact.

### Email (SMTP)

Email is the most universal channel — works without a phone, plays
nicely with archive search, and survives outages of any single
chat platform. Configure any STARTTLS-capable SMTP relay
(Gmail with an app password, Fastmail, your own postfix, etc.):

1. Pick an SMTP relay. For Gmail, generate an
   [app password](https://myaccount.google.com/apppasswords)
   (Google blocks plain-password SMTP for non-app-password
   accounts).
2. Store the SMTP password in the OS keyring under service
   `career-ai`, key `smtp/<host>/<username>`. Example:
   ```sh
   secret-tool store --label="career-ai SMTP" \
     service career-ai \
     username smtp/smtp.gmail.com/career-ai@example.com
   ```
3. In `config/local.yaml`:
   ```yaml
   notify:
     channels:
       email:
         smtp_host: smtp.gmail.com
         smtp_port: 587
         smtp_username: career-ai@example.com
         from: career-ai@example.com
         to:
           - operator@example.com
   ```
4. Verify: `careerai notify test`.

The 5-second timeout applies to the whole SMTP exchange; if your
relay is slow, surface a self-hosted relay or pick a different
channel.

### ntfy

[ntfy.sh](https://ntfy.sh) is the cheapest "push to phone" path —
no auth, no signup, free. Install the [ntfy app](https://ntfy.sh/app)
on your phone and subscribe to a private/random topic name.

1. Pick a long random topic — anyone who knows the topic can read
   your alerts. Use `openssl rand -hex 12` or similar.
2. In `config/local.yaml`:
   ```yaml
   notify:
     channels:
       ntfy:
         topic: careerai-RANDOM-PRIVATE-TOPIC
         # server: https://ntfy.sh   # optional override for self-hosted
   ```
3. Subscribe to the topic in the ntfy phone app.
4. Verify: `careerai notify test`.

career-ai sets the ntfy `Priority` header per severity (info=2,
warning=3, critical=5), the `Title` header to the event title,
and the `Tags` header to a relevant emoji name so the phone shows
a recognisable icon.

## Event reference

career-ai fires the following events. Channels render each one via
the same `title()` + `summary()` shape; severity controls the
visual color (Slack), priority (ntfy), and ordering (filter).

| Event                  | Severity         | When fired                                                                                                       |
|------------------------|------------------|-------------------------------------------------------------------------------------------------------------------|
| `CookieExpiringSoon`   | warning/critical | `careerai digest` reads LinkedIn `li_at` and finds it within 48h of expiry (warning) or already broken (critical) |
| `SourceUnreachable`    | warning          | A `Source::discover()` returned an error mid-run (HTTP failure, DNS, schema drift)                                 |
| `AuthFailureMidRun`    | critical         | A cookie or token-based credential failed inside a discover/submit pass (M5 follow-up)                             |
| `ManualReviewNeeded`   | warning          | Submitter encountered an unknown form field; operator must review the captured screenshot (M5 follow-up)           |
| `HighScoreMatch`       | info             | Matcher shortlisted a listing with score ≥ `match.notify_threshold` (default 0.85)                                  |
| `RateLimitExhausted`   | warning          | A submit attempt was skipped because the daily cap or quiet-hours window was active                                 |
| `ApplicationResponded` | info             | M7 (future) — application got a recruiter / ATS response                                                          |

The default `min_severity` is `warning`, so only `HighScoreMatch`
and `ApplicationResponded` are silently dropped on a fresh
install — bump to `info` to see those, or to `critical` to see
only blocking events.

## Severities

| Tag      | When to use                                                                  |
|----------|------------------------------------------------------------------------------|
| info     | Routine signal worth seeing once. Default sink for `careerai notify test`.   |
| warning  | Operator should look at this within a day (cookie expiring, rate-limit caps). |
| critical | Pipeline blocked / cookie already expired / submitter cannot proceed.        |

`Severity` is `Info < Warning < Critical`, so `min_severity:
critical` drops every event below `Critical` *before* any channel
is invoked. Filter at the pipeline level rather than per-channel
so an operator's max-noise floor is consistent across Slack,
phone push, etc.

## Adding a custom channel

WhatsApp Business API is paid + complex onboarding and is
deliberately not bundled. To wire one (or any other channel —
Discord, Pushover, generic webhook), implement the `Notifier`
trait from `careerai_notify`:

```rust
use async_trait::async_trait;
use careerai_notify::{Notifier, NotifyError, NotifyEvent, Severity};

#[derive(Debug)]
struct MyChannel { /* ... */ }

#[async_trait]
impl Notifier for MyChannel {
    fn name(&self) -> &'static str { "my-channel" }

    async fn notify(&self, event: &NotifyEvent, severity: Severity)
        -> Result<(), NotifyError>
    {
        // Render `event.title()` + `event.summary()` to your channel,
        // honor the 5s `HTTP_TIMEOUT` constant for any outbound call,
        // and never log webhook URLs or tokens. Return `NotifyError`
        // on failure — `Pipeline::fire` swallows and warn-logs it.
        Ok(())
    }
}
```

See the existing channels under
`crates/careerai-notify/src/channels/` for the pattern. Tests
should use `wiremock` for HTTP-shaped channels and assert no
known-secret shape leaks into `tracing` output (see
`tests/secret_redaction_it.rs`).
