# Dashboard + systemd autostart — design

Date: 2026-04-29
Status: Draft (awaiting review)
Owners: Kamal Pandey (single-user pipeline)

## Problem

Two operator gaps in v0.1.1-mcp:

1. **No autostart.** `careerai daemon` runs in the foreground; if the user
   reboots, closes the terminal, or logs out, the cron-driven discovery /
   match / render loop stops. Surviving the laptop's life cycle requires a
   service supervisor.
2. **No at-a-glance overview.** Pipeline state lives in SQLite and is
   readable only via `careerai inspect` / `careerai shortlist`. There is no
   "is the daemon healthy, what's queued, what should I touch next" view —
   the operator has to compose CLI calls to answer those questions.

This spec adds both. Out of scope: clickable JD links as a stand-alone
feature (folded into the dashboard for free), per-listing detail pages,
multi-user auth, remote access.

## Goals / non-goals

**Goals**

- `careerai daemon` survives reboot and logout via a systemd user service
  the operator can install with one command.
- A single-page dashboard at `http://127.0.0.1:8787` (port configurable)
  showing: KPI strip, pipeline funnel, "next steps" punch list. Renders in
  under 200 ms on a 10k-listing DB.
- JD links on every listing card open in a new tab.
- The dashboard is read-only — no DB writes from the web surface.
- Every command is reversible (`service uninstall`, Ctrl-C the server).

**Non-goals**

- No auth, TLS, or external binding in this iteration. `127.0.0.1` only.
  Migration to "real domain + always-on in daemon" (option A from the
  brainstorm) is a future spec.
- No live updates / WebSockets / SSE. Page reloads via `<meta refresh>`.
- No JS framework, bundler, or CDN dependency. Offline-first.

## High-level architecture

Two independent units of work, sharing no runtime path:

```
┌──────────────────────────────────┐    ┌──────────────────────────────────┐
│  systemd user service            │    │  careerai status --serve         │
│  ~/.config/systemd/user/         │    │                                  │
│    careerai.service              │    │  starts axum HTTP server         │
│                                  │    │  bound to 127.0.0.1:<port>       │
│  ExecStart=careerai daemon       │    │  reads SQLite read-only          │
│  Restart=on-failure              │    │  renders Tera templates          │
│                                  │    │  exits on Ctrl-C                 │
└────────────────┬─────────────────┘    └────────────────┬─────────────────┘
                 │                                       │
                 └────────── shared SQLite DB ───────────┘
                            (CAREERAI_ROOT/data.db)
```

The daemon and the dashboard server are **separate processes**. The
dashboard does not depend on the daemon being up — it shows the current
DB state regardless. If the daemon is down, that fact surfaces as a
"daemon not running" indicator on the dashboard (deferred — see "Open
questions / future work").

## Component 1 — `careerai-dashboard` crate

New crate `crates/careerai-dashboard/`. Owns: server boot, route handlers,
templates, embedded CSS, read-only data fetchers.

### Crate boundary

| Owns | Never does |
|---|---|
| `axum` server, bind, port handling | DB writes |
| Tera templates + embedded CSS | Submission, browser automation |
| Read-only queries against `careerai-db` | Cron / scheduling |
| Listing → card view-model mapping | LLM calls |

### Module layout (each ≤ 300 LOC per project rule)

```
crates/careerai-dashboard/
  src/
    lib.rs           ─ public `run(opts: ServeOptions) -> Result<()>`
    routes.rs        ─ axum Router wiring
    handlers.rs      ─ `index`, `healthz`
    data.rs          ─ read-only query layer (wraps careerai-db)
    view.rs          ─ DTOs for templates (KpiStrip, FunnelColumn, NextStep)
    next_steps.rs    ─ rule engine for the punch list
    error.rs         ─ thiserror enum
  templates/
    layout.tera      ─ <html><head> with embedded <style>
    index.tera       ─ KPI strip + funnel + next-steps
    _kpi.tera        ─ partial
    _funnel.tera     ─ partial
    _next_steps.tera ─ partial
  static/
    style.css        ─ extracted CSS (compiled into binary via include_str!)
  Cargo.toml
```

### Routes

| Method | Path | Handler | Notes |
|---|---|---|---|
| GET | `/` | `index` | full dashboard render |
| GET | `/healthz` | `healthz` | `200 ok\n` plain text |

That's it. Single page, one health probe. No JSON API in this iteration —
add later if a separate frontend ever wants it.

### Data model (view DTOs)

```rust
pub struct KpiStrip {
    pub today_discovered: u64,
    pub shortlisted_active: u64,    // currently in `shortlisted` state
    pub applied_lifetime: u64,
    pub response_rate_pct: Option<f32>, // None if applied_lifetime == 0
}

pub struct FunnelColumn {
    pub state: ListingState, // enum reused from careerai-db
    pub label: String,       // "Discovered", "Shortlisted", ...
    pub count: u64,
    pub top: Vec<ListingCard>, // first 3 by score desc
}

pub struct ListingCard {
    pub title: String,
    pub company: String,
    pub score: Option<f32>,
    pub url: String,         // canonical_url || apply_url || ""
    pub posted_at: Option<DateTime<Utc>>,
}

pub struct NextStep {
    pub kind: NextStepKind,
    pub label: String,       // user-facing
    pub urgency: Urgency,    // Info | Warn | Action
}

pub enum NextStepKind {
    ReadyToTailor { count: u64 },
    ReadyToRender { count: u64 },
    ReadyToApply  { count: u64 },
    SourceStale   { source: String, age_hours: u64 },
    LinkedInCookieExpiring { days_left: u64 },
    ProfileStale  { age_days: u64 },
}
```

### Next-steps rule engine (`next_steps.rs`)

Pure function: `(snapshot: PipelineSnapshot) -> Vec<NextStep>`. Snapshot
is composed once per request from `data.rs`. Rules:

| Rule | Trigger | Urgency |
|---|---|---|
| `ReadyToTailor`  | `count(state=shortlisted) > 0` | Action |
| `ReadyToRender`  | `count(state=tailored) > 0`    | Action |
| `ReadyToApply`   | `count(state=rendered) > 0`    | Action |
| `SourceStale`    | `last_discover(source) > 24h`  | Warn   |
| `LinkedInCookieExpiring` | `cookie.expires_in < 7d` | Warn |
| `ProfileStale`   | `mtime(profile.yaml) > 30d`    | Info   |

Deterministic ordering: Action first, then Warn, then Info. Within tier,
by count desc / age desc. Easy to unit-test (no I/O in `next_steps.rs`).

### Render path

1. `handlers::index` calls `data::snapshot()` (single async function that
   issues all queries concurrently via `tokio::join!`).
2. Builds `IndexView { kpi, columns, next_steps }`.
3. Renders `index.tera` via a `Arc<Tera>` initialized at startup
   (`include_dir!`-loaded templates, no filesystem reads at request time).
4. Returns `Html(rendered)`.

Performance budget: 200 ms p95 on a 10k-listing DB. Queries are
indexed (`listings(state)`, `events(occurred_at)` already exist).

### CSS / visual design

- **Dark-mode-default** with CSS variables for theme. Light mode opt-in
  via `prefers-color-scheme: light` media query.
- **Type:** system stack (`-apple-system, Segoe UI, ...`) for body, monospace
  (`SF Mono, Menlo, ...`) for KPI numbers and counts.
- **Palette:** muted neutral background, single warm accent for "Action"
  next-steps and shortlisted/applied states. WCAG AA contrast.
- **Layout:** single-column on narrow viewports, three-section vertical
  stack — KPI strip → funnel kanban → next steps. Funnel scrolls
  horizontally on narrow screens.
- **Cards:** subtle border, no shadows, hover raises border accent. JD
  link is the entire card title (target=_blank, rel=noopener).
- **Auto-refresh:** `<meta http-equiv="refresh" content="60">` in `<head>`.
- **Total CSS:** target ≤ 200 LOC, single embedded `<style>` (no separate
  file shipped). `include_str!("static/style.css")` at compile time.

### Public API (lib.rs)

```rust
pub struct ServeOptions {
    pub port: u16,                 // default 8787
    pub bind: IpAddr,              // default 127.0.0.1, --bind hidden flag
    pub cfg: Arc<CoreConfig>,
    pub db: Arc<DbPool>,
}

pub async fn run(opts: ServeOptions) -> Result<(), DashboardError>;
```

`run` blocks until shutdown signal (SIGINT/SIGTERM on unix, Ctrl-C on
windows — reuse `careerai-scheduler::shutdown` patterns).

### Error handling

- Bind failures (`EADDRINUSE`, etc.) → `DashboardError::BindFailed { port,
  source }`, exit code 4.
- Template render errors are bugs — log + return 500 with a static
  fallback page; do not panic.
- DB errors → 500 + structured tracing log; the page renders a "data
  temporarily unavailable" banner instead of failing the whole render.

## Component 2 — `careerai service` subcommand

New top-level subcommand: `careerai service { install | status | uninstall }`.

### `service install`

Idempotent. Steps:

1. Resolve install path: `${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/careerai.service`.
2. If file already exists and content equals the rendered template, exit
   0 with "already installed". If content differs, prompt
   "Overwrite existing unit file? [y/N]" (suppressible with `--force`).
3. Resolve `careerai` binary path via `std::env::current_exe()`.
4. Render unit template (below) with the resolved path, write atomically
   (`tempfile::NamedTempFile::persist` — same pattern fixed in PR #30).
5. Run `systemctl --user daemon-reload`.
6. Prompt: "Run `loginctl enable-linger $USER` so the daemon survives
   logout? [Y/n]". On yes, run it (requires no sudo on user accounts in
   most distros; surface the error if it does). On no, print the command
   for the user to run later.
7. Print next steps:
   ```
   Installed: ~/.config/systemd/user/careerai.service
   Enable + start with:
       systemctl --user enable --now careerai
   Check status:
       careerai service status
   ```

**The install command does NOT auto-enable the service.** Enabling a
service is a deliberate operator action. Auto-enable would surprise
people who installed for inspection.

### Unit file template

```ini
[Unit]
Description=career-ai pipeline daemon
Documentation=https://github.com/justdoGIT/career-ai
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={{careerai_bin}} daemon
Restart=on-failure
RestartSec=10s
# Optional: keep secrets out of the unit file
EnvironmentFile=-%h/.config/careerai/env
# CAREERAI_ROOT is read from the env file above when the operator pins it;
# we deliberately do NOT hardcode a default here, since users can pick
# any path on `careerai init`. Falling back to the binary's default
# resolution keeps the unit file portable.
# Resource limits — keeps a runaway daemon from eating the laptop
MemoryMax=2G
CPUQuota=80%

[Install]
WantedBy=default.target
```

`EnvironmentFile=-` (note the leading `-`) makes the file optional —
service starts even if the env file does not exist.

### `service status`

Wraps `systemctl --user status careerai`. No reformatting in v1 —
operators expect the standard systemctl output.

### `service uninstall`

1. Run `systemctl --user disable --now careerai` (ignore "not loaded"
   error).
2. Remove `~/.config/systemd/user/careerai.service`.
3. Run `systemctl --user daemon-reload`.
4. Note: does NOT touch linger. Leave that to the operator.

### Cross-platform note

systemd is Linux-only. On macOS / Windows, `careerai service install`
exits with code 65 ("not implemented on this platform — see docs for
launchd / Task Scheduler"). Detection via `cfg!(target_os = "linux")`.
Documented in the help text. Future work, not this spec.

## Wiring into `careerai-cli`

New clap variants in `Command`:

```rust
Status { #[command(subcommand)] cmd: StatusCmd },
Service { #[command(subcommand)] cmd: ServiceCmd },

enum StatusCmd {
    Show,                                  // existing CLI status (unchanged)
    Serve {
        #[arg(long, default_value = "8787")] port: u16,
        #[arg(long, hide = true)]           // hidden — loopback only
        bind: Option<IpAddr>,
    },
}

enum ServiceCmd { Install { #[arg(long)] force: bool }, Status, Uninstall }
```

`status serve` calls `careerai_dashboard::run`. `service *` calls a new
`crates/careerai-cli/src/commands/service.rs` module.

## Configuration

New optional section in `config/default.yaml`:

```yaml
dashboard:
  port: 8787              # overrideable per-invocation by --port
  refresh_seconds: 60     # meta-refresh interval; 0 disables
```

CLI `--port` overrides the config value. No new env vars.

## Testing

| Layer | What we test |
|---|---|
| Unit | `next_steps.rs` rule engine — table-driven cases for every `NextStepKind` |
| Unit | `data.rs` query → DTO mapping with a tempfile SQLite + fixtures |
| Integration (`tests/dashboard_it.rs`) | Spawn `careerai_dashboard::run` on an ephemeral port, GET `/`, assert HTML contains expected sections (`role="kpi-strip"`, `role="funnel"`, `role="next-steps"`); GET `/healthz` returns 200 |
| Integration (`tests/service_it.rs`) | `careerai service install --force` to a temp `XDG_CONFIG_HOME`, assert file written, parse with `ini` crate, assert `ExecStart` is the test binary path; `service uninstall` removes it |
| Snapshot (`insta`) | Render the index template against a fixed snapshot DB; assert HTML fragment stable. Updated via `INSTA_UPDATE=always` |

TDD per project CLAUDE.md: each feature lands with a failing test first.

## Documentation

Updates in the same PR:

- `README.md` — add "Run as a service" section pointing at `careerai
  service install`; add "Dashboard" section with screenshot
- `CHANGELOG.md` — new Unreleased entry
- `docs/SERVICE.md` — full systemd walkthrough including linger,
  troubleshooting (`systemctl --user status`, journalctl), uninstall

## Security review

- **Bind:** loopback only (`127.0.0.1`). `--bind` flag is hidden. Even
  with `--bind 0.0.0.0`, no auth → loud warning printed to stderr.
- **HTTP surface:** read-only. No mutation routes. Nothing exposed that
  isn't already in the SQLite file.
- **Path traversal:** templates are loaded via `include_dir!` at compile
  time — no runtime filesystem reads from the request path.
- **Unit file:** `EnvironmentFile` is the only place secrets touch
  systemd; the unit itself is secret-free. `MemoryMax` + `CPUQuota`
  cap runaway behavior. Future: `ProtectSystem=strict`,
  `PrivateTmp=true` once we audit the daemon's filesystem touches.
- **Semgrep:** re-baseline after the new crate lands per global rule
  (≥ 20 changed files threshold).

## Risks

| Risk | Mitigation |
|---|---|
| `axum` adds compile time to the workspace | Quantify on first build; if ≥ 30s extra, gate behind `--features dashboard` (default-on) |
| Port collision with another local service | Clear `BindFailed` error + `--port` flag |
| systemd-not-installed (some minimal distros / WSL) | Detect at install time, exit with actionable error |
| `loginctl enable-linger` requires polkit on some distros | Detect failure, show the manual command |
| Template drift between `data.rs` DTOs and Tera vars | Type the DTOs; insta snapshot tests catch silent drift |
| `--bind` hidden but discoverable — operator binds to LAN by accident | Loud stderr warning when bind ≠ 127.0.0.1; also documented |

## Rollout

Single PR, `feat/dashboard-and-systemd`. Touches:

- new crate `crates/careerai-dashboard/`
- new module `crates/careerai-cli/src/commands/service.rs`
- updated `crates/careerai-cli/src/commands/status.rs` (existing) — add
  `serve` variant
- `crates/careerai-cli/Cargo.toml` — add `careerai-dashboard` dep
- `Cargo.toml` workspace — register new crate
- `crates/careerai-core/src/config.rs` — `DashboardConfig` field
- `config/default.yaml` (template) — new `dashboard:` block
- `README.md`, `CHANGELOG.md`, `docs/SERVICE.md`

Estimated 4–6 SSH-signed commits, ~1500 LOC total (most in the new
crate). Branch off `main`, merge after CI green + self-review.

## Open questions / future work

- **Always-on dashboard inside the daemon (option A from the brainstorm):**
  defer until there's a real domain or remote access need. At that
  point: spawn the dashboard as a tokio task inside the daemon, gate
  with auth (probably bearer token from keyring), introduce `dashboard:
  always_on: true` config flag.
- **JSON API:** out of scope for v1; add `/api/v1/snapshot` if a
  separate UI consumer ever appears.
- **PID-file detection of daemon health on the dashboard:** stretch
  goal — detect via `systemctl --user is-active careerai` or a PID
  file written by the daemon.
- **Per-listing detail page:** linked from cards; deferred.
- **Charts / sparklines (option B from the brainstorm):** if "is the
  daemon doing work" becomes a question, add a 30-day sparkline above
  the KPI strip.
