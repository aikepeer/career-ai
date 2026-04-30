# careerai-notify

Best-effort multi-channel notification pipeline. Slack, Telegram,
email, ntfy.sh.

## Boundary

| Owns | Never does |
|---|---|
| `Pipeline::from_config(&cfg.notify)` builder | Pipeline orchestration |
| `Notifier` trait + per-channel impls | DB queries |
| `NotifyEvent` event enum (high-score match, source unreachable, cookie expiring, rate-limit exhausted, manual review needed, ...) | LLM calls |
| `Severity` enum (`Info` < `Warning` < `Error`) + min-severity floor | Cron scheduling |
| Best-effort fan-out: every channel runs concurrently, errors logged + swallowed |  |

## Channels

| Channel | Cost | Setup |
|---|---|---|
| Slack | free | Incoming webhook + `webhook_url_env: CAREERAI_SLACK_WEBHOOK` |
| Telegram | free | Bot token in OS keyring + numeric `chat_id` |
| Email | free | SMTP relay; password in OS keyring |
| ntfy.sh | free | Random topic name; phone push via the ntfy app |

All secrets come from env vars or the OS keyring. Inline secrets
in YAML are rejected with a parse error.

## Safety

* `Pipeline::fire` is **best-effort**: a broken webhook can't kill
  the daemon tick. Channels run in `tokio::spawn` tasks; panics
  surface as `JoinError::Panic` warn-logs.
* Events below `min_severity` are silently dropped (with a debug
  log).
* `tracing` redaction filter scrubs cookie / token / API-key
  shapes from emitted events; a regex test asserts captured logs
  contain no known-secret patterns.

## Adding a channel

1. Implement `Notifier` for `XNotifier`.
2. Add a `pub struct XConfig` to `config.rs` (with sensible defaults).
3. Wire it into `Pipeline::from_config`.
4. Document setup in [`docs/NOTIFICATIONS.md`](../../docs/NOTIFICATIONS.md).

## Tests

```bash
cargo test -p careerai-notify
```

Channel notifiers are tested against `wiremock` (Slack, Telegram,
ntfy) and a captured-Notifier mock (email). The redaction test is
in `tests/secret_redaction_it.rs`.
