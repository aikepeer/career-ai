# Implementation Plan — Career-AI Modern Dark Theme MVP Dashboard & Runit Service

> **Goal:** Transform `careerai-dashboard` into a modern, dark-themed MVP dashboard showcasing live job application funnels, system audit events, configuration/sources status, and an interview action center, served as a Void Linux `runit` background service.

---

## User Review Required

> [!IMPORTANT]
> - **Port Binding**: The dashboard will run on **`http://127.0.0.1:8787`** (loopback only by default).
> - **Runit Service**: A new runit service `careerai-dashboard` will be registered at `/etc/sv/careerai-dashboard` and activated in `/var/service/careerai-dashboard`.
> - **Cargo Build**: Per workspace rules, `cargo build --release` should be run in your terminal to build the release binary after code modifications.

---

## Delivery status — 2026-09-22

The implementation described below is present in the current working tree. The
dashboard now has the event/configuration/action/detail data layer, responsive
dark workspace, command palette and saved-search behavior, runit launch
support, server-side explorer filtering/pagination, and explicit refresh
failure handling. Follow-up cards include listing context and persisted
handling state.

Verification completed:

- `cargo test --release -p careerai-dashboard --lib`
- `cargo test --release -p careerai-dashboard --test browser_it -- --test-threads=1`
  (4 tests passed)
- `cargo test --release --workspace --all-targets --locked --no-fail-fast`
  (workspace passed; browser tests use isolated temporary profiles)

The remaining operator-only gate is installing/enabling the runit service and
checking `127.0.0.1:8787` on a target Void host. This plan does not claim that
privileged deployment step from repository tests.


## Proposed Changes

### Component 1: `careerai-db` & Backend Data Layer

#### [MODIFY] [`crates/careerai-db/src/queries/events.rs`](file:///home/miniblues/projects/career-ai/crates/careerai-db/src/queries/events.rs)
- Add query helper `list_recent_events(pool, limit, offset)` to fetch system audit logs from the `events` table (timestamp, event_type, listing_id, payload, severity).

#### [MODIFY] [`crates/careerai-dashboard/src/view.rs`](file:///home/miniblues/projects/career-ai/crates/careerai-dashboard/src/view.rs)
- Add view structs:
  - `EventLogItem`: Formatted system event with relative timestamp, severity badge, and details.
  - `ConfigSourceItem`: Active source status (name, type, company count, last sync).
  - `ConfigView`: Overview of matcher rules (`score_threshold`, `must_include_skills`), LLM provider status, rate limits, and active sources.
  - `ActionItem`: Actionable cards for `Drafted` applications (awaiting `careerai review`) and `Responded` applications (with links to `careerai-prep` sheets).
  - `ApplicationDetail`: Full metadata payload for application modal popovers.

#### [MODIFY] [`crates/careerai-dashboard/src/data.rs`](file:///home/miniblues/projects/career-ai/crates/careerai-dashboard/src/data.rs)
- Add data fetchers for `fetch_recent_events`, `fetch_config_view`, `fetch_action_center`, and `fetch_application_detail`.

---

### Component 2: `careerai-dashboard` HTTP Routes & REST API

#### [MODIFY] [`crates/careerai-dashboard/src/routes.rs`](file:///home/miniblues/projects/career-ai/crates/careerai-dashboard/src/routes.rs)
- Add API endpoints to `build_router()`:
  - `GET /` -> HTML single-page dashboard app.
  - `GET /api/v1/snapshot` -> JSON pipeline snapshot & KPIs.
  - `GET /api/v1/events` -> JSON system audit events list.
  - `GET /api/v1/config` -> JSON configuration & sources inspector.
  - `GET /api/v1/applications/:id` -> JSON application detail modal payload.

#### [MODIFY] [`crates/careerai-dashboard/src/handlers.rs`](file:///home/miniblues/projects/career-ai/crates/careerai-dashboard/src/handlers.rs)
- Implement `api_snapshot`, `api_events`, `api_config`, and `api_application_detail` JSON handlers.

---

### Component 3: Modern Dark-Theme UI & Templates

#### [MODIFY] [`crates/careerai-dashboard/templates/index.tera`](file:///home/miniblues/projects/career-ai/crates/careerai-dashboard/templates/index.tera)
- Complete redesign using HSL obsidian/slate dark theme:
  - **Header Bar**: Brand logo, daemon & LLM status pills, auto-refresh countdown indicator, tab navigation buttons.
  - **Tab 1 — Funnel & Applications**: KPI metrics strip, instant search input, filter pills by source/stage, scrollable Kanban columns with match score badges and click-to-view detail modals.
  - **Tab 2 — System Audit Log**: Real-time event feed with severity tags (Info, Warning, Error) and event search.
  - **Tab 3 — Config & Sources Inspector**: Grid of active job sources (Greenhouse, Lever, Ashby, LinkedIn, Naukri, etc.), match threshold visualization, `must_include_skills` pills, LLM provider status & cache stats.
  - **Tab 4 — Action & Interview Center**: Review queue for `Drafted` applications, `Responded` interview study sheets, and credential expiry alerts.

#### [MODIFY] [`crates/careerai-dashboard/static/style.css`](file:///home/miniblues/projects/career-ai/crates/careerai-dashboard/static/style.css)
- CSS custom properties (`--bg-primary: #0b0f19`, `--bg-card: #111827`, `--accent: #6366f1`, `--accent-cyan: #06b6d4`, etc.).
- Responsive flex/grid layout, smooth tab transitions, micro-interactions, modal styling, card hover effects.

---

### Component 4: Void Linux `runit` Service

#### [NEW] `/etc/sv/careerai-dashboard/run`
- Shell script executing `careerai status serve --port 8787` as `miniblues` user.

#### [NEW] `/etc/sv/careerai-dashboard/log/run`
- Runit logging script using `vlogger` or `socklog`.

#### Symlink Setup
- `sudo ln -s /etc/sv/careerai-dashboard /var/service/`

---

## Verification Plan

### Automated Verification
- Run dashboard unit tests:
  ```bash
  cargo test -p careerai-dashboard
  cargo test -p careerai-db
  ```

### Manual Verification
1. Access `http://127.0.0.1:8787` in a browser or test via `curl`:
   - `curl -s http://127.0.0.1:8787/api/v1/snapshot | jq .`
   - `curl -s http://127.0.0.1:8787/api/v1/events | jq .`
   - `curl -s http://127.0.0.1:8787/api/v1/config | jq .`
2. Verify `runit` service status:
   - `sudo sv status careerai-dashboard`
