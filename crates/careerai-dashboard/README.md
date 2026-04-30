# careerai-dashboard

Read-only HTTP dashboard. KPI strip + kanban funnel + next-steps
punch list. Bound to `127.0.0.1:8787` by default.

## Boundary

| Owns | Never does |
|---|---|
| `axum` server boot, route registration, bind | DB writes (the surface is read-only) |
| Tera templates + embedded CSS | Submission, browser automation |
| Read-only queries against `careerai-db` (entity-mapped to view DTOs) | Cron scheduling |
| `daemon_health::probe()` (wraps `systemctl --user is-active careerai`) | LLM calls |
| `next_steps::compute(&snapshot)` rule engine | Argument parsing |

## Layout

```
src/
  lib.rs           public `run(opts)` entry
  routes.rs        axum Router wiring
  handlers.rs      `index`, `healthz`
  data.rs          read-only query layer
  view.rs          DTOs (KpiStrip, FunnelColumn, NextStep, ...)
  next_steps.rs    pure rule engine for the punch list
  daemon_health.rs `systemctl --user is-active` probe
  error.rs         thiserror enum
templates/
  index.tera       single-page render
static/
  style.css        embedded via include_str!
```

## Routes

| Method | Path | Returns |
|---|---|---|
| GET | `/` | full dashboard HTML |
| GET | `/healthz` | `200 ok\n` plain text |

No JSON API in v1.

## Safety

* Loopback-only by default. `--bind` flag is hidden; non-loopback
  bind logs a loud no-auth warning to stderr.
* Read-only — no DB writes, no network egress.
* No auth — single-user model. If exposed beyond loopback (which
  the project explicitly does not support today), every mutation
  surface stays absent.

## Operator usage

```bash
careerai status serve              # 127.0.0.1:8787
careerai status serve --port 9000  # alt port
```

Auto-refreshes every 60s via `<meta http-equiv="refresh">`. No JS,
no CDN, offline-first.

## Tests

```bash
cargo test -p careerai-dashboard
```

8 unit tests cover the next-steps rule engine + the daemon-health
probe (with stub `systemctl` script). 1 integration test spawns
the server on an ephemeral port and curls `/healthz` + `/`.
