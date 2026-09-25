# Docker

`careerai-hosted` is the containerized HTTP API. The image is a
multi-stage build: a `rust:1.78-bookworm` builder compiles the workspace
in release mode, and a `debian:bookworm-slim` runtime contains only the
API binary and the packages needed by its healthcheck. Resume rendering
tools are not part of this hosted image.

The compose stack is intentionally loopback-only by default. It is for
local smoke deployment and self-hosted experiments, not an internet-facing
production deployment.

## Build

```bash
docker compose build api       # uses ./Dockerfile
# or, without compose:
docker build -t careerai-hosted .
```

The builder pins Rust 1.78 (the project MSRV). The repo's
`rust-toolchain.toml` is removed inside the builder stage so rustup does
not fetch a different toolchain.

## Run

For a local smoke deployment, generate a process-scoped master key and
start Postgres plus the hosted API:

```bash
MASTER_KEY="$(openssl rand -hex 32)" docker compose up -d --build
curl --fail http://127.0.0.1:3000/v1/health
curl --fail http://127.0.0.1:3000/v1/ready
docker compose logs -f api
docker compose down
```

The generated key is suitable only for an ephemeral smoke run. Keep the
same stable key across restarts when encrypted exports must remain
readable; provide it through a local secret manager or a shell wrapper,
never by committing it to `.env` or source control.

## First-time setup

The compose stack initializes Postgres automatically. The hosted image
does not include the local CLI entrypoint, profile bind mounts, or the
SQLite daemon workflow. Configure the hosted API through its environment
and database migrations, then verify both health endpoints before using
authenticated routes.

For monitoring, start the optional Prometheus service:

```bash
MASTER_KEY="$(openssl rand -hex 32)" docker compose --profile monitoring up -d
```

## State layout

| Service | Persistent state | Purpose |
|---------|------------------|---------|
| `postgres` | Docker volume `pgdata` | Hosted API database |
| `api` | Stateless container | HTTP API and migrations |

Do not delete `pgdata` while preserving a master key unless the data is
intentionally disposable. The master key must remain stable for encrypted
export data.

## Environment variables

| Variable | Compose value | Purpose |
|----------|---------------|---------|
| `DATABASE_URL` | Internal Postgres URL | Hosted API database |
| `MASTER_KEY` | Required, 64 hex characters | Encryption key for exports |
| `STRIPE_API_KEY` | Empty | Optional real billing integration |
| `STRIPE_WEBHOOK_SECRET` | Empty | Optional Stripe webhook verification |
| `CAREERAI_HOSTED_BIND` | `0.0.0.0:3000` | API bind address inside the container |
| `RUST_LOG` | `info,careerai_hosted=debug` | Structured log filter |

Compose binds API and Postgres to loopback addresses only. Change the
published bindings only after adding authentication, TLS, backups, and
an explicit network threat model.

## Health and troubleshooting

```bash
docker compose ps
docker compose logs --tail=100 api
curl --fail http://127.0.0.1:3000/v1/health
curl --fail http://127.0.0.1:3000/v1/ready
```

`health` confirms the process is alive. `ready` also checks the database
dependency. If `ready` fails, inspect the Postgres healthcheck and the API
logs before retrying.

## Scope notes

- This compose file runs `careerai-hosted`, not the local `careerai`
  daemon or dashboard. Use the systemd/user-service runbook for the local
  SQLite workflow.
- The image does not include Chromium. Browser submitters are not part of
  this hosted smoke stack.
- Live submission remains disabled unless the separately configured source
  gates, credentials, and operator confirmation permit it. A healthy
  container is not evidence that a live ATS submission is authorized.
- The Prometheus service is optional and should remain loopback-only during
  local testing.
