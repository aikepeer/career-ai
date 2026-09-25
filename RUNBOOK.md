# CareerAI operator runbook

This runbook covers the local SQLite daemon, operator-confirmed ATS actions,
interview preparation, and the loopback-only hosted Docker smoke stack. It does
not authorize live submissions by itself. Live writes require the configured
source gate, a valid credential, and an explicit operator decision for the
specific listing.

## One-time local setup

Install the CLI with the capabilities used by the configured workflow:

```bash
cargo install --path crates/careerai-cli --features live-llm-cli,live-llm-api,browser
careerai init
```

`careerai init` creates the per-user configuration and data roots. Unless
explicitly overridden, the paths are:

- config: `$XDG_CONFIG_HOME/career-ai` or `~/.config/career-ai`
- data: `$XDG_DATA_HOME/career-ai` or `~/.local/share/career-ai`

Import a profile or copy an already-reviewed profile into the generated
profile directory:

```bash
careerai profile import /path/to/resume.pdf
# or
careerai profile import /path/to/resume.docx
```

Use the existing configured backend and configured ATS targets. Do not invent
company slugs, listing IDs, or provider model names in `config/local.yaml`.
Inspect the generated configuration first, then add only the target sources and
locations you have permission to use.

## Credentials

Credentials are handles, not values, in this runbook. Store them in `kepr` or
the OS keychain used by the selected adapter. Retrieve a value only for the
single command that needs it:

```bash
kepr get <CREDENTIAL_NAME> --quiet
```

Never paste a secret into chat, commit it, put it in YAML, or include it in a
command that can be recorded in shell history. If a required handle is absent,
create it in `kepr` before enabling the adapter; the application fails closed
when the credential is unavailable.

For browser-assisted sources, refresh the interactive cookie through the CLI
rather than copying cookie values:

```bash
careerai cookies refresh
```

Keep LinkedIn `interactive_only: true` (the default). The daemon may prepare a
draft, but an operator must review the browser form before clicking submit.

## Configure and inspect before operating

Edit the generated `config/local.yaml` and retain the existing configured ATS
targets. A safe starting point keeps writes disabled:

```yaml
submit:
  auto_submit: false
  per_source:
    greenhouse:
      enabled: false
    lever:
      enabled: false
    ashby:
      enabled: false
    linkedin:
      enabled: false
  linkedin:
    interactive_only: true
```

Set the current remote LLM backend through the existing configuration and its
credential handle. The `prep` and remote tailoring paths do not silently fall
back to local generation when remote mode is required.

Run discovery, matching, and a dry-run before enabling any source write:

```bash
careerai discover
careerai match
careerai tailor --strategy llm
careerai apply --dry-run
careerai digest
```

Review the audit trail and generated artifacts. Confirm the listing URL,
employer, resume diff, cover letter, and source gate for each application.

## Live ATS operation

Only after reviewing a specific dry-run application:

1. Confirm the exact existing target and listing ID or URL.
2. Confirm the source's credential handle is present in `kepr`.
3. Confirm the target permits this automation and the account is not flagged.
4. Enable only that source's `submit.per_source.<source>.enabled` gate.
5. Set `submit.auto_submit: true` only for the bounded operation.
6. Run one application, inspect the resulting event, then disable the gate.

Do not enable LinkedIn autonomous writes. Use the review flow instead:

```bash
careerai review
```

The kill switch is immediate:

```bash
systemctl --user stop careerai.service
# or press Ctrl-C in the foreground daemon
careerai inspect <application-id>
```

After a source reports a restriction or ambiguous remote outcome, stop and do
not retry the same submission. Resolve the audit state manually first.

## Daily and weekly workflow

Daily:

```bash
careerai digest
careerai review
```

For a confirmed employer response, record it explicitly. Inbox polling does
not infer this transition automatically:

```bash
careerai mark-responded <application-id> --note "Recruiter email received"
```

Generate an interview prep sheet with the employer page plus explicitly
allowlisted selected-news domains. The command rejects non-HTTPS or
non-allowlisted URLs:

```bash
careerai prep <application-id> \
  --news-url https://employer.example/news/release \
  --news-domain selected-news.example
```

Weekly, review the recent audit events and tune only configuration values:

```bash
careerai inspect <application-id>
careerai digest --since 7d
```

Refresh browser cookies when the digest reports imminent expiry. Keep source
rate limits and quiet hours enabled.

## Local hosted Docker smoke deployment

The compose file runs `careerai-hosted` with Postgres. It is separate from the
local SQLite daemon and publishes only loopback addresses.

Use an ephemeral key for a disposable smoke run:

```bash
MASTER_KEY="$(openssl rand -hex 32)" docker compose up -d --build
curl --fail http://127.0.0.1:3000/v1/health
curl --fail http://127.0.0.1:3000/v1/ready
docker compose ps
docker compose logs --tail=100 api
docker compose down
```

For a run whose encrypted exports must survive restart, supply the same stable
master key through a local secret manager or a wrapper. Never commit it to
`.env` or source control. Keep the published ports loopback-only during local
smoke testing. The container does not include Chromium and is not evidence that
ATS live submission is configured or authorized.

Optional monitoring remains loopback-only:

```bash
MASTER_KEY="$(openssl rand -hex 32)" docker compose --profile monitoring up -d
```

## Troubleshooting

| Symptom | Action |
|---|---|
| `careerai` cannot find config | Run `careerai init`; check `XDG_CONFIG_HOME` and `XDG_DATA_HOME`. |
| Remote LLM credential failure | Verify the configured credential handle with `kepr`; do not paste the value. |
| Cookie-expiry warning | Run `careerai cookies refresh`, then repeat a dry-run. |
| ATS returns 401/403 | Stop writes, verify permission and credential scope, and do not retry blindly. |
| LinkedIn selector or modal mismatch | Stop the review, keep `interactive_only: true`, save the screenshot, and update selectors only after a fixture reproduction. |
| Docker `ready` fails | Run `docker compose ps` and inspect Postgres/API logs; preserve the same `MASTER_KEY` across restarts. |
| Docker port already in use | Stop the conflicting local service or change the loopback host port intentionally; do not bind publicly by default. |
