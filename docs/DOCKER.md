# Docker

`careerai` ships as a single statically-mostly Rust binary. The container
image is a multi-stage build: a `rust:1.78-bookworm` builder compiles the
workspace in release mode, and a `debian:bookworm-slim` runtime adds only
the external rendering tools (`pandoc`, `weasyprint`). No Python or Node.js
is added as project code — `weasyprint` is the configured PDF engine and
is installed as a system package, the same way `pandoc` is.

## Build

```bash
docker compose build         # uses ./Dockerfile
# or, without compose:
docker build -t careerai .
```

The builder pins Rust 1.78 (the project MSRV). The repo's
`rust-toolchain.toml` pins `channel = "stable"`, which would otherwise make
rustup fetch latest stable inside the builder; the Dockerfile removes it so
the build uses the image's 1.78.0 toolchain.

## Run

```bash
docker compose up -d          # daemon + dashboard
docker compose logs -f
docker compose down
```

The container's default command is `careerai daemon`. The entrypoint
(`docker-entrypoint.sh`) starts the daemon as PID 1 and, in the background,
launches the read-only dashboard bound to `0.0.0.0:3000` so it is reachable
via the published port. The dashboard is a **separate process** from the
daemon — see the main README "Dashboard" section.

## First-time setup

On a fresh volume, generate the default config + profile templates onto the
bind-mounted host directories:

```bash
docker compose run --rm careerai init
```

This writes `config/default.yaml`, `config/rules.example.yaml`,
`config/.env.example`, and `profile/profile.example.yaml` under `./config`
and `./profile` on the host. Then:

1. Copy `profile/profile.example.yaml` → `profile/profile.yaml` and fill it in
   (or import a resume: `docker compose run --rm careerai profile import resume.pdf`).
2. Put overrides in `config/local.yaml` (companies, score threshold, sources).
3. Set secrets in the environment (see below), then `docker compose up -d`.

## Volume layout

`CAREERAI_ROOT=/data`. All state lives under it. The compose file
bind-mounts three host directories:

| Container path        | Host mount   | Contents                                            |
|-----------------------|--------------|-----------------------------------------------------|
| `/data`               | `./data`     | SQLite DB, LLM cache, logs, render artifacts        |
| `/data/config`        | `./config`   | `default.yaml`, `local.yaml` overrides, rules       |
| `/data/profile`       | `./profile`  | `profile.yaml`                                      |

The app nests its own `data/` subdirectory under `CAREERAI_ROOT`, so the
SQLite database lives at `/data/data/careerai.sqlite` — i.e.
`./data/data/careerai.sqlite` on the host. The LLM cache is at
`./data/data/cache/llm/` and render artifacts at `./data/artifacts/`.

## Environment variables

| Variable                              | Default | Purpose                                                          |
|---------------------------------------|---------|------------------------------------------------------------------|
| `CAREERAI_ROOT`                       | `/data` | App root: where the DB, cache, config, profile live.             |
| `ANTHROPIC_API_KEY`                   | unset   | Anthropic API key for the `live-llm-api` LLM backend.             |
| `CAREERAI_DASHBOARD_TOKEN`            | unset   | When set, every dashboard request must carry this bearer token.  |
| `CAREERAI_DASHBOARD_ALLOW_NON_LOOPBACK` | `1`   | Set in the image so the dashboard can bind `0.0.0.0` in Docker.   |
| `CAREERAI_DASHBOARD_PORT`             | `3000`  | Port the background dashboard binds inside the container.        |

Pass secrets via the host environment or a sibling `.env` file (compose
reads it automatically). `docker-compose.yml` forwards
`ANTHROPIC_API_KEY` and `CAREERAI_DASHBOARD_TOKEN` from the host.

## Dashboard

The dashboard is read-only and has **no authentication by default**. Inside
the container it binds `0.0.0.0:3000` (so the published port is reachable),
which the loopback-only guard would normally refuse —
`CAREERAI_DASHBOARD_ALLOW_NON_LOOPBACK=1` (baked into the image) permits
this. **Set `CAREERAI_DASHBOARD_TOKEN`** before exposing port 3000 to any
non-local network; with it set, every request must send
`Authorization: Bearer <token>` (or `x-careerai-token: <token>`).

Open `http://localhost:3000` after `docker compose up -d`.

## Using the CLI inside the container

The entrypoint passes any arguments straight to `careerai`, so one-shot
subcommands work without starting the dashboard:

```bash
docker compose run --rm careerai discover --source greenhouse
docker compose run --rm careerai match
docker compose run --rm careerai tailor <listing-id>
docker compose run --rm careerai digest --since 24h
docker compose exec careerai status        # against the running daemon's DB
```

To run the daemon only (no background dashboard), bypass the entrypoint:

```bash
docker compose run --rm --entrypoint careerai careerai daemon
```

To start just the dashboard against an existing volume:

```bash
docker compose run --rm --service-ports careerai status serve --port 3000 --bind 0.0.0.0
```

## Notes

- `pandoc` (DOCX + PDF) and `weasyprint` (PDF engine) are installed in the
  runtime image; both are required for resume rendering.
- The image does **not** include Chromium. The LinkedIn/Naukri browser
  submitters (built via the `browser` feature) need a browser at runtime and
  will not function in this image; ATS HTTP submitters and discovery work
  normally. Auto-submit stays off by default (dry-run).
- The container runs as root so it can write to bind-mounted volumes
  regardless of the host UID. To run as non-root, add `user: "<uid>:<gid>"`
  in `docker-compose.yml` matching the host user that owns `./data`.
