# Job-search sprint Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver W1–W4 of the job-search sprint per the design spec at `docs/superpowers/specs/2026-04-25-job-search-sprint-design.md` — ATS submitters live mode + matcher tightening, LinkedIn assist mode + Naukri click + review CLI, daily digest, and interview-prep crate.

**Architecture:** Layered atop the existing M3–M6 pipeline. New work: `must_include_skills` hard filter in match crate, `Drafted` state variant in `ListingState` (TEXT column, no SQL migration), `interactive_only` flag in `LinkedinSubmitConfig` defaulting to `true`, greenfield browser-based Naukri submitter mirroring the M5a LinkedIn pattern, four new CLI subcommands (`review`, `digest`, `mark-responded`, `prep`), `careerai cookies refresh` helper, and a new `careerai-prep` crate for interview study sheets.

**Tech stack:** Rust workspace (edition 2021, MSRV 1.78) + sqlx 0.8 (SQLite) + chromiumoxide 0.7 + tokio-cron-scheduler 0.13 + Tera 1 + rig-core 0.10 + tracing + thiserror + governor 0.10 + keyring 3 + insta + tempfile.

**Spec amendment from execution-realism:** the spec says daemon "form-fills + screenshots" LinkedIn drafts. That's not feasible cheaply — chromiumoxide sessions can't be persisted across daemon-and-CLI process boundaries, so the daemon would have to keep a long-lived browser open per-draft (impractical). **Refined:** `drafted` means resume + cover-letter rendered and ready, NO LinkedIn browser interaction yet. `careerai review` opens a fresh browser session per drafted application, walks the form, screenshots, prompts y/N/skip, and clicks. Naukri stays full-auto in the daemon. This refinement is also documented at the top of Phase 2 below.

## Reconciliation status — 2026-09-22

The checkbox procedure below is a historical implementation plan, not a
completion record. Current source status:

- Implemented: `must_include_skills`, the `Drafted` state, LinkedIn
  `interactive_only` guards, `careerai review`, Naukri submission support,
  cookie refresh, `careerai digest`, `CookieExpiringSoon` notifications,
  the `careerai-prep` crate, `careerai mark-responded`, and the `careerai
  prep` command with explicit allowlisted research sources.
- The prep command remains a remote-LLM workflow by configuration; local
  tests use a deterministic fixture backend only.
- Not locally verified: live ATS submissions, credential availability, and
  operator-side host deployment. Those remain external acceptance gates.

---

## Phase 1 — W1: ATS submitters live mode + matcher tightening

**Outcome of phase:** `must_include_skills` hard filter live; matcher rejects junk listings before scoring; ATS submitters verified live against 3 real jobs (operator action).

### Task 1.1: Map current match flow (Serena exploration, no code change)

**Files:** read-only

- [ ] **Step 1:** `find_symbol("MatchConfig", "crates/careerai-core/src/config.rs", include_body=true)` — confirm current struct shape (2 fields: `embedding_model: String`, `score_threshold: f32`).
- [ ] **Step 2:** `get_symbols_overview("crates/careerai-match/src/filters.rs")` — identify the entry point that decides shortlisted vs filtered_out.
- [ ] **Step 3:** `find_symbol("filter_listings", "crates/careerai-match/src/filters.rs", include_body=true)` (or whatever the actual top-level filter fn is named per the overview).
- [ ] **Step 4:** `find_symbol("match_all", "crates/careerai-pipeline/src/lib.rs", include_body=true)` — confirm where the match crate is invoked from the pipeline.

No commit.

### Task 1.2: Add `must_include_skills` to `MatchConfig`

**Files:**
- Modify: `crates/careerai-core/src/config.rs:152-156` (`MatchConfig` struct)
- Modify: `crates/careerai-core/src/templates/default.yaml`

- [ ] **Step 1: Write failing test** in `crates/careerai-core/src/config.rs` test module:

```rust
#[test]
fn match_config_must_include_skills_defaults_empty() {
    let yaml = "embedding_model: \"x\"\nscore_threshold: 0.5";
    let cfg: MatchConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(cfg.must_include_skills.is_empty());
}

#[test]
fn match_config_parses_must_include_skills() {
    let yaml = "embedding_model: \"x\"\nscore_threshold: 0.5\nmust_include_skills: [rust, async]";
    let cfg: MatchConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(cfg.must_include_skills, vec!["rust", "async"]);
}
```

- [ ] **Step 2: Run tests, expect compile error** (`must_include_skills` field doesn't exist):

```
cargo test -p careerai-core match_config_must_include_skills 2>&1 | tail -10
```
Expected: `error[E0609]: no field 'must_include_skills' on type 'MatchConfig'`.

- [ ] **Step 3: Add the field** — replace `MatchConfig` struct with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchConfig {
    pub embedding_model: String,
    pub score_threshold: f32,
    /// Hard filter applied BEFORE scoring. A listing must contain at least
    /// one of these tokens (case-insensitive substring match) anywhere in
    /// its title, description, or normalized skill set, or it transitions
    /// directly to `filtered_out`. Empty = no hard filter (default
    /// behavior matches pre-W1 builds).
    #[serde(default)]
    pub must_include_skills: Vec<String>,
}
```

- [ ] **Step 4: Run tests, expect pass:**

```
cargo test -p careerai-core match_config_must_include_skills
```

- [ ] **Step 5: Update embedded defaults** at `crates/careerai-core/src/templates/default.yaml` — add under the `match:` section:

```yaml
match:
  embedding_model: "BAAI/bge-small-en-v1.5"
  score_threshold: 0.62
  # Hard filter: a listing must contain at least one of these tokens
  # (case-insensitive substring) somewhere in its title/description/skills,
  # or it's filtered out before scoring. Leave empty for no filter.
  must_include_skills: []
```
Replace the existing `match:` block while preserving sibling fields' values. Use `Read` with offset+limit to bound the file diff.

- [ ] **Step 6: Verify `CoreConfig::load` still parses defaults:**

```
cargo test -p careerai-core
```

- [ ] **Step 7: Commit:**

```bash
git add crates/careerai-core/src/config.rs crates/careerai-core/src/templates/default.yaml
git commit -s -m "feat(match): add must_include_skills hard filter to MatchConfig"
```

### Task 1.3: Apply the filter in `careerai-match`

**Files:**
- Modify: `crates/careerai-match/src/filters.rs`

- [ ] **Step 1: Find the filter entry point** via Serena:

```
find_symbol("filter_listings", "crates/careerai-match/src/filters.rs", include_body=true)
```

(or the equivalent top-level fn name — adapt to actual signature.)

- [ ] **Step 2: Write failing test** at the bottom of `filters.rs`:

```rust
#[cfg(test)]
mod must_include_tests {
    use super::*;
    use careerai_core::config::MatchConfig;
    use careerai_match::types::RawListing; // adjust import to match actual type path

    fn cfg_with_required(skills: &[&str]) -> MatchConfig {
        MatchConfig {
            embedding_model: "x".into(),
            score_threshold: 0.0,
            must_include_skills: skills.iter().map(|s| (*s).into()).collect(),
        }
    }

    fn raw(title: &str, desc: &str) -> RawListing {
        // Adjust constructor to match actual RawListing shape.
        RawListing {
            title: title.into(),
            description: desc.into(),
            ..Default::default()
        }
    }

    #[test]
    fn missing_required_skill_filters_out() {
        let listing = raw("Frontend Engineer", "We use React and GraphQL.");
        let cfg = cfg_with_required(&["rust", "tokio"]);
        // Replace `apply_must_include_filter` with the actual fn name added in step 3.
        assert!(!apply_must_include_filter(&listing, &cfg));
    }

    #[test]
    fn matching_required_skill_passes() {
        let listing = raw("Backend Engineer", "Rust + tokio shop, async-heavy.");
        let cfg = cfg_with_required(&["rust"]);
        assert!(apply_must_include_filter(&listing, &cfg));
    }

    #[test]
    fn empty_required_list_passes_everything() {
        let listing = raw("Anything", "Anything");
        let cfg = cfg_with_required(&[]);
        assert!(apply_must_include_filter(&listing, &cfg));
    }

    #[test]
    fn match_is_case_insensitive() {
        let listing = raw("Senior Engineer", "We use RUST and Tokio.");
        let cfg = cfg_with_required(&["rust"]);
        assert!(apply_must_include_filter(&listing, &cfg));
    }
}
```

- [ ] **Step 3: Run tests, expect compile error** (`apply_must_include_filter` undefined):

```
cargo test -p careerai-match must_include_tests 2>&1 | tail -10
```

- [ ] **Step 4: Implement the filter helper:**

```rust
/// Returns `true` if the listing satisfies the `must_include_skills` hard
/// filter. Empty filter → always true. Otherwise: at least one configured
/// skill (case-insensitive substring) must appear in the listing's title
/// or description.
pub fn apply_must_include_filter(listing: &RawListing, cfg: &MatchConfig) -> bool {
    if cfg.must_include_skills.is_empty() {
        return true;
    }
    let haystack = format!("{} {}", listing.title, listing.description).to_ascii_lowercase();
    cfg.must_include_skills
        .iter()
        .any(|needle| haystack.contains(&needle.to_ascii_lowercase()))
}
```

- [ ] **Step 5: Wire the filter into the existing `filter_listings` (or equivalent) function** — apply it BEFORE scoring. Locate the function via Serena, then `replace_symbol_body` to inject the early-return on `!apply_must_include_filter(...)` → transition to `FilteredOut`. Exact body depends on current shape; adapt minimally.

- [ ] **Step 6: Run tests, expect pass:**

```
cargo test -p careerai-match
```

- [ ] **Step 7: Run full workspace tests:**

```
cargo test --workspace --no-fail-fast
```
Expected: 199 → 203 (+4 new must-include tests).

- [ ] **Step 8: Commit:**

```bash
git add crates/careerai-match/src/filters.rs
git commit -s -m "feat(match): apply must_include_skills filter before scoring"
```

### Task 1.4: Verify ATS submitters work live (operator action — NOT executed by AI)

**Files:** none (operator-side validation against real ATS endpoints)

- [ ] **Step 1:** Operator picks 3 real job listings — one from each of {Greenhouse, Lever, Ashby}. Adds the company slugs to `$XDG_CONFIG_HOME/career-ai/config/local.yaml` under `sources.greenhouse.companies` etc.
- [ ] **Step 2:** Operator runs `careerai discover --source greenhouse` (and lever, ashby). Confirms 3 new listings landed in DB.
- [ ] **Step 3:** Operator runs `careerai match` with a tuned `must_include_skills`. Confirms one of the 3 reaches `shortlisted`.
- [ ] **Step 4:** Operator runs `careerai tailor <listing_id>` then `careerai render <application_id>`. Confirms artifacts appear in `$XDG_DATA_HOME/career-ai/artifacts/<application_id>/`.
- [ ] **Step 5:** Operator runs `careerai apply --auto-submit <application_id>` against ONE Greenhouse listing first. Captures the remote application ID from the log line. Repeats for Lever + Ashby.
- [ ] **Step 6:** Operator verifies application receipt email arrived from each ATS. If any failed, surfaces the failure for engineer triage.

**Definition of Done for Phase 1:** 199+ tests green, 3 successful live ATS submits, `careerai-match` rejects junk listings via `must_include_skills`. Phase 1 commit hash recorded.

---

## Phase 2 — W2: drafted state + LinkedIn assist + Naukri click + review CLI

**Refined model (per spec amendment above):** daemon never opens a LinkedIn browser. After render, LinkedIn applications transition to `Drafted` (resume + cover ready). The new `careerai review` CLI is the ONLY thing that opens a chromiumoxide session for LinkedIn — per-application, per-review-invocation, with an explicit y/N/skip prompt before the actual submit click. Naukri stays full-auto in the daemon (`auto_submit=true` + `submit.per_source.naukri.enabled=true` → daemon does the whole flow including click).

### Task 2.1: Add `Drafted` variant to `ListingState`

**Files:**
- Modify: `crates/careerai-core/src/state.rs:10-23` (the enum)

- [ ] **Step 1: Write failing test** in the `state.rs` test module:

```rust
#[test]
fn drafted_serde_roundtrip() {
    let s = ListingState::Drafted;
    let yaml = serde_yaml::to_string(&s).unwrap();
    assert_eq!(yaml.trim(), "drafted");
    let back: ListingState = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(s, back);
}

#[test]
fn drafted_as_str_is_drafted() {
    assert_eq!(ListingState::Drafted.as_str(), "drafted");
}

#[test]
fn drafted_parses_from_str() {
    assert_eq!("drafted".parse::<ListingState>().unwrap(), ListingState::Drafted);
}
```

- [ ] **Step 2: Run tests, expect compile error.**

- [ ] **Step 3: Add the variant** — replace `ListingState` enum body to include `Drafted` between `Prepared` and `Submitted`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ListingState {
    Discovered,
    FilteredOut,
    Shortlisted,
    Tailored,
    Rendered,
    Prepared,
    Drafted,
    Submitted,
    Skipped,
    Failed,
    Responded,
}
```

- [ ] **Step 4: Find and update `as_str` / `FromStr` / any match arms** that need `Drafted` handling. Use `find_referencing_symbols("ListingState", "crates/careerai-core/src/state.rs")` to locate every consumer; verify each match is exhaustive after the addition.

- [ ] **Step 5: Verify compile + tests:**

```
cargo build --workspace
cargo test -p careerai-core
cargo test --workspace --no-fail-fast
```

- [ ] **Step 6: Commit:**

```bash
git add crates/careerai-core/src/state.rs
git commit -s -m "feat(core): add Drafted state variant for LinkedIn assist mode"
```

### Task 2.2: Add `interactive_only: bool` to `LinkedinSubmitConfig`

**Files:**
- Modify: `crates/careerai-core/src/config.rs` (`LinkedinSubmitConfig` struct + `validated`)
- Modify: `crates/careerai-core/src/templates/default.yaml`

- [ ] **Step 1: Find current struct** — `find_symbol("LinkedinSubmitConfig", "crates/careerai-core/src/config.rs", include_body=true)`. Note the existing fields and `Default` impl.

- [ ] **Step 2: Write failing test:**

```rust
#[test]
fn linkedin_interactive_only_defaults_true() {
    let cfg = LinkedinSubmitConfig::default();
    assert!(cfg.interactive_only, "must default to assist-mode (true) so daemon never auto-clicks");
}

#[test]
fn linkedin_interactive_only_round_trips() {
    let yaml = "interactive_only: false";
    let cfg: LinkedinSubmitConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(!cfg.interactive_only);
}
```

- [ ] **Step 3: Add the field** — in `LinkedinSubmitConfig`, add:

```rust
/// When `true` (default), the daemon never opens a LinkedIn browser
/// session and never clicks Submit autonomously. After rendering,
/// LinkedIn applications transition to `Drafted` instead. Operators
/// confirm submits one at a time via `careerai review`. Flip to
/// `false` only after weighing the account-restriction risk.
#[serde(default = "default_interactive_only")]
pub interactive_only: bool,
```

And add the helper:

```rust
fn default_interactive_only() -> bool { true }
```

Update the `Default` impl to include `interactive_only: true`.

- [ ] **Step 4: Run tests, expect pass.**

- [ ] **Step 5: Document in `default.yaml`** under the `submit.linkedin:` section:

```yaml
submit:
  linkedin:
    # ... existing fields ...
    interactive_only: true   # daemon never auto-clicks; only `careerai review` does
```

- [ ] **Step 6: Run full test suite, then commit:**

```bash
cargo test --workspace --no-fail-fast
git add crates/careerai-core/src/config.rs crates/careerai-core/src/templates/default.yaml
git commit -s -m "feat(submit): add interactive_only flag to LinkedinSubmitConfig (default true)"
```

### Task 2.3: Modify the LinkedIn apply path to honor `interactive_only`

**Files:**
- Modify: `crates/careerai-pipeline/src/lib.rs` — find `apply_one`. When the listing source is `"linkedin"` AND `cfg.submit.linkedin.interactive_only` is `true`, transition the application to `Drafted` and skip browser entirely.
- Modify: `crates/careerai-submit/src/lib.rs` — `submit_application` should accept an `interactive_only` short-circuit OR the short-circuit can live at the pipeline layer (preferred, less invasive).

**Decision:** short-circuit at the pipeline layer. `careerai-submit::submit_application` stays unchanged. `careerai-pipeline::apply_one` checks the flag before dispatching.

- [ ] **Step 1: Locate `apply_one`** via `find_symbol("apply_one", "crates/careerai-pipeline/src/lib.rs", include_body=true)`.

- [ ] **Step 2: Write failing test** in a new file `crates/careerai-pipeline/tests/linkedin_assist_it.rs`:

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_core::config::CoreConfig;
use careerai_db::{models::*, pool_from_path, queries};
use careerai_pipeline as pipeline;

#[tokio::test]
async fn linkedin_apply_with_interactive_only_transitions_to_drafted() {
    let tmp = tempfile::tempdir().unwrap();
    // Scaffold + seed a rendered LinkedIn application.
    // (Reuse the helper pattern from apply_it.rs::seed_rendered_application,
    //  adapted to set listing.source = "linkedin".)
    // ...

    let mut cfg = CoreConfig::load(tmp.path()).unwrap();
    cfg.submit.auto_submit = true;
    cfg.submit.per_source.insert("linkedin".into(), Default::default());
    cfg.submit.per_source.get_mut("linkedin").unwrap().enabled = true;
    cfg.submit.linkedin.interactive_only = true;  // explicit, even though default

    let outcome = pipeline::apply_one(tmp.path(), &cfg, &application_id, None).await.unwrap();
    // Assert the application is now Drafted, not Submitted, and no error.
    let pool = pool_from_path(&tmp.path().join("data/careerai.sqlite")).await.unwrap();
    let app = queries::find_application_by_id(&pool, &application_id).await.unwrap();
    assert_eq!(app.state, "drafted");
}
```

- [ ] **Step 3: Run test, expect fail** (apply_one currently treats LinkedIn like any source).

- [ ] **Step 4: Add the short-circuit** — in `apply_one`, before the `submit_application` call, add:

```rust
if listing.source == "linkedin" && cfg.submit.linkedin.interactive_only {
    queries::transition_application_and_listing(
        &pool,
        application_id,
        &listing.id,
        ListingState::Drafted.as_str(),
        ListingState::Drafted,
        Some("drafted: awaiting careerai review"),
    ).await?;
    return Ok(AppliedOutcome {
        application_id: application_id.to_owned(),
        outcome: SubmitOutcome::DryRun {
            payload_summary: "drafted: awaiting careerai review".into(),
        },
    });
}
```

(Adapt to the actual signatures of `transition_application_and_listing` + `AppliedOutcome` already in the file.)

- [ ] **Step 5: Run test, expect pass.**

- [ ] **Step 6: Run full workspace tests:**

```
cargo test --workspace --no-fail-fast
```

- [ ] **Step 7: Commit:**

```bash
git add crates/careerai-pipeline/src/lib.rs crates/careerai-pipeline/tests/linkedin_assist_it.rs
git commit -s -m "feat(pipeline): linkedin interactive_only short-circuits to drafted"
```

### Task 2.4: `careerai review` CLI — walk drafted LinkedIn applications

**Files:**
- Create: `crates/careerai-cli/src/review.rs`
- Modify: `crates/careerai-cli/src/main.rs` — add the `Review` subcommand
- Modify: `crates/careerai-pipeline/src/lib.rs` — add `pub fn list_drafted_linkedin(root) -> Vec<Application>` and `pub async fn confirm_linkedin_submit(root, cfg, application_id) -> Result<AppliedOutcome>`

The `confirm_linkedin_submit` call is what actually opens chromiumoxide and clicks Submit, by passing `interactive_only=false` AND `auto_submit=Some(true)` ONLY for that single call. The flag is overridden via a per-call `effective_submit_cfg` already used by the scheduler's dry-run override (mirror the same pattern but inverted).

- [ ] **Step 1: Add `list_drafted_linkedin` query** in `careerai-db/src/queries.rs` (TDD):

Test:
```rust
#[tokio::test]
async fn list_drafted_linkedin_returns_only_drafted() {
    let pool = test_pool().await;
    // seed: 2 drafted (linkedin), 1 rendered (linkedin), 1 drafted (greenhouse)
    // ...
    let rows = list_drafted_linkedin(&pool, 100).await.unwrap();
    assert_eq!(rows.len(), 2);
    for r in &rows {
        assert_eq!(r.state, "drafted");
    }
}
```
Implementation in `queries.rs` mirrors existing `list_applications_by_state_and_source`:

```rust
pub async fn list_drafted_linkedin(pool: &SqlitePool, limit: i64) -> Result<Vec<Application>> {
    let rows = sqlx::query_as::<_, Application>(
        "SELECT a.* FROM applications a
         JOIN listings l ON l.id = a.listing_id
         WHERE a.state = 'drafted' AND l.source = 'linkedin'
         ORDER BY a.created_at ASC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
```

Commit after green.

- [ ] **Step 2: Add `pipeline::list_drafted_linkedin` wrapper** that opens the pool and calls the query. Inline test that asserts wrapper round-trips. Commit.

- [ ] **Step 3: Add `pipeline::confirm_linkedin_submit(root, cfg, application_id)`** — clones cfg, sets `cfg.submit.linkedin.interactive_only = false` for the call, asserts current state is `Drafted`, then delegates to `submit_application` (which already does the full chromiumoxide flow when interactive_only=false). On success, the existing transition logic moves the row to `Submitted`.

Test (mocked submit, just exercises the cfg-override path):

```rust
#[tokio::test]
async fn confirm_linkedin_submit_overrides_interactive_only() {
    // verify the cloned cfg has interactive_only=false even when input cfg had true
}
```

Commit.

- [ ] **Step 4: Create `crates/careerai-cli/src/review.rs`** with:

```rust
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use careerai_core::config::CoreConfig;
use careerai_pipeline as pipeline;

/// Run the interactive review loop for drafted LinkedIn applications.
/// For each drafted row: print summary, prompt y/N/s, and on `y` call
/// `pipeline::confirm_linkedin_submit`.
pub async fn run_review(root: &Path, cfg: &CoreConfig) -> Result<()> {
    let drafted = pipeline::list_drafted_linkedin(root, 100)
        .await
        .context("list drafted linkedin applications")?;

    if drafted.is_empty() {
        println!("No drafted LinkedIn applications. Nothing to review.");
        return Ok(());
    }

    println!("{} drafted LinkedIn application(s) ready for review.\n", drafted.len());

    let stdin = io::stdin();
    for (idx, app) in drafted.iter().enumerate() {
        println!("[{}/{}] application {}", idx + 1, drafted.len(), app.id);
        // Optionally fetch + print the listing title/company. Use Serena
        // patterns from `inspect_show` to load the JD.
        print!("  Submit? [y/N/s(kip all remaining)]: ");
        io::stdout().flush().ok();

        let mut line = String::new();
        stdin.read_line(&mut line)?;
        match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => {
                match pipeline::confirm_linkedin_submit(root, cfg, &app.id).await {
                    Ok(outcome) => println!("    submitted: {outcome:?}"),
                    Err(e) => eprintln!("    failed: {e}"),
                }
            }
            "s" | "skip-all" => {
                println!("Skipping remaining {} application(s).", drafted.len() - idx);
                break;
            }
            _ => println!("    skipped"),
        }
    }
    Ok(())
}
```

- [ ] **Step 5: Wire `Review` subcommand in `careerai-cli/src/main.rs`** — add to the `Command` enum:

```rust
/// Walk drafted LinkedIn applications, prompt y/N per draft, click submit on yes.
Review,
```

And in the dispatch match:

```rust
Command::Review => {
    let cfg = load_cfg(&cwd)?;
    crate::review::run_review(&cwd, &cfg).await?;
}
```

Add `mod review;` near the top of `main.rs`.

- [ ] **Step 6: Smoke test** — manual test: seed a drafted LinkedIn application, run `cargo run -p careerai-cli -- review`, hit `s` to skip-all. Confirm clean exit.

- [ ] **Step 7: Commit:**

```bash
git add crates/careerai-cli/src/review.rs crates/careerai-cli/src/main.rs crates/careerai-pipeline/src/lib.rs crates/careerai-db/src/queries.rs
git commit -s -m "feat(cli): careerai review walks drafted linkedin applications"
```

### Task 2.5: Naukri browser submitter — greenfield implementation

**Files:**
- Create: `crates/careerai-submit/src/naukri.rs`
- Create: `crates/careerai-submit/src/naukri_selectors.rs`
- Modify: `crates/careerai-submit/src/lib.rs` — wire the new submitter, mirror the `linkedin` cfg pattern.
- Modify: `crates/careerai-core/src/config.rs` — add `NaukriSubmitConfig` (parallels `LinkedinSubmitConfig`).
- Create: `crates/careerai-submit/tests/naukri_fixture_it.rs`

**Pattern:** mirror the M5a `linkedin.rs` design: `BrowserSession` for chromiumoxide, governor for rate limit, keyring for `naukri_session` cookie, audit screenshot before click. Default to `interactive_only=false` (Naukri risk is lower per spec).

- [ ] **Step 1: Map LinkedIn submitter shape via Serena** — `get_symbols_overview("crates/careerai-submit/src/linkedin.rs")`. Note the function chain: `submit → run_session → ensure_logged_in → navigate_to_jd → click_easy_apply → fill_form → screenshot → click_submit_or_bail`.

- [ ] **Step 2: Add `NaukriSubmitConfig` struct** in `careerai-core/src/config.rs`. Same fields as `LinkedinSubmitConfig` minus LinkedIn-specific ones (`allow_submit_click` not needed — Naukri's "kill switch" is just the existing `auto_submit` + `per_source.enabled`). Test the round-trip + defaults. Commit.

- [ ] **Step 3: Add `naukri_selectors.rs`** with the CSS selectors. Per M5a pattern: pin every selector with a comment explaining the LinkedIn equivalent that broke. Pre-populate with best-guess selectors:

```rust
//! CSS selectors for Naukri.com Apply flow. Pinned and commented so a
//! selector change AT one site can't pass the fixture test while
//! breaking the live flow.

/// "Apply" CTA on Naukri job pages. Class is volatile; aria-label is
/// the more stable anchor.
pub const APPLY_BUTTON_SELECTOR: &str =
    "button[id^='apply-button'], button[aria-label*='Apply']";

/// "Confirm Apply" button in the modal that pops after Apply.
pub const CONFIRM_APPLY_SELECTOR: &str =
    "button.apply-button-text, button[type='submit'][class*='apply']";

/// Login-required indicator. If we hit this, the session cookie is dead.
pub const LOGIN_REQUIRED_INDICATOR: &str = "div.login-layer, form[name='loginForm']";
```

(The operator will refine these against real Naukri pages; document the refinement as a follow-up commit.)

- [ ] **Step 4: Implement `NaukriSubmitter` in `naukri.rs`** — start with the stub:

```rust
//! Naukri.com browser-driven submitter. Mirrors the M5a LinkedIn pattern:
//! chromiumoxide session, governor rate-limiter, keyring-backed cookie,
//! audit screenshot. Live-submits when auto_submit=true AND
//! per_source.naukri.enabled=true. No interactive_only gate (Naukri risk
//! is lower than LinkedIn per design spec).

use std::sync::Arc;
use anyhow::Result;
use crate::base::{SubmitContext, Submitter};
use crate::error::SubmitError;
use crate::rate_limiter::{RateLimiter, RatePolicy};
use crate::browser_session::{BrowserSession, BrowserSessionConfig};
use careerai_core::config::NaukriSubmitConfig;
use crate::naukri_selectors::*;

#[derive(Debug, Clone)]
pub struct NaukriConfig {
    pub screenshots_dir: std::path::PathBuf,
    pub user_agent: Option<String>,
    pub headless: bool,
    pub rate_policy: RatePolicy,
    pub action_timeout_seconds: u64,
}

impl NaukriConfig {
    pub fn from_core(cfg: &careerai_core::config::SubmitConfig) -> Self {
        Self {
            screenshots_dir: cfg.naukri.screenshots_dir.clone(),
            user_agent: cfg.naukri.user_agent.clone(),
            headless: cfg.naukri.headless,
            rate_policy: RatePolicy {
                max_per_day: cfg.naukri.max_per_day,
                min_seconds_between: cfg.naukri.min_seconds_between,
                jitter_seconds: cfg.naukri.jitter_seconds,
                quiet_hours_utc: cfg.naukri.quiet_hours_utc,
            },
            action_timeout_seconds: cfg.naukri.action_timeout_seconds,
        }
    }
}

pub struct NaukriSubmitter {
    cfg: NaukriConfig,
    rate_limiter: Arc<RateLimiter>,
}

impl NaukriSubmitter {
    pub fn new(cfg: NaukriConfig, rate_limiter: Arc<RateLimiter>) -> Self {
        Self { cfg, rate_limiter }
    }

    fn load_naukri_session(&self) -> Result<String, SubmitError> {
        // Mirror linkedin.rs::load_li_at — keyring "careerai-naukri", key "session_cookie".
        let entry = keyring::Entry::new("careerai-naukri", "session_cookie")
            .map_err(|e| SubmitError::Credential(e.to_string()))?;
        entry.get_password().map_err(|e| SubmitError::Credential(e.to_string()))
    }

    async fn run_session(
        &self,
        session: &BrowserSession,
        ctx: &SubmitContext<'_>,
        cookie: &str,
        permit_slot: &mut Option<crate::rate_limiter::RatePermit<'_>>,
    ) -> Result<String, SubmitError> {
        // 1. inject cookie
        // 2. navigate to ctx.listing.url
        // 3. detect login required (if so, return Err(SubmitError::Credential("naukri session expired")))
        // 4. click APPLY_BUTTON_SELECTOR
        // 5. wait for CONFIRM_APPLY_SELECTOR
        // 6. screenshot to cfg.screenshots_dir/<application_id>.png
        // 7. commit rate permit (caller passed permit_slot)
        // 8. click CONFIRM_APPLY_SELECTOR
        // 9. detect "applied" toast / success indicator
        // 10. return remote-id (extract from URL or page) — fallback to listing.external_id
        unimplemented!("M5c implementation — fill in based on M5a linkedin.rs pattern, see naukri_fixture_it.rs for the fixture-test boundary")
    }
}

#[async_trait::async_trait]
impl Submitter for NaukriSubmitter {
    fn name(&self) -> &'static str { "naukri" }

    async fn submit(&self, ctx: &SubmitContext<'_>) -> Result<String, SubmitError> {
        let permit = self.rate_limiter
            .acquire(self.name(), &self.cfg.rate_policy)
            .await
            .map_err(|e| SubmitError::SourceDisabled(format!("rate-limited: {e}")))?;

        let cookie = self.load_naukri_session()?;
        let session = BrowserSession::launch(BrowserSessionConfig {
            headless: self.cfg.headless,
            user_agent: self.cfg.user_agent.clone(),
            ..Default::default()
        }).await?;

        let mut permit_slot = Some(permit);
        let outcome = self.run_session(&session, ctx, &cookie, &mut permit_slot).await;
        session.close().await?;
        outcome
    }
}
```

The `unimplemented!()` body is the explicit M5c-implementation gate. Operator + engineer fill it in by adapting the LinkedIn patterns in `linkedin.rs::run_session`.

- [ ] **Step 5: Add fixture test** — `crates/careerai-submit/tests/naukri_fixture_it.rs`. Pattern: mirror `linkedin_fixture_it.rs`. Captured Naukri HTML served by `tiny-http`, asserts the selectors find the Apply button. Will need a captured fixture (`tests/fixtures/naukri_jd.html`) which the operator captures from a real Naukri job page.

- [ ] **Step 6: Wire `NaukriSubmitter` into `submit_application`** — `find_symbol("submit_application", "crates/careerai-submit/src/lib.rs")`, find the `match source_lc.as_str()` block, add a `"naukri"` arm behind `#[cfg(feature = "browser")]` mirroring the linkedin arm.

- [ ] **Step 7: Add `cookies refresh` CLI subcommand** for Naukri (and LinkedIn) — interactive prompt that walks the user through grabbing the cookie from a real browser, stores in keyring. Single file `crates/careerai-cli/src/cookies.rs`. Two cargo dev commands (`careerai cookies refresh linkedin` / `careerai cookies refresh naukri`).

- [ ] **Step 8: Run full workspace tests:**

```
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 9: Commit (split: stub + selectors as one commit, fixture test + wiring as second, cookies CLI as third):**

```bash
git add crates/careerai-submit/src/naukri.rs crates/careerai-submit/src/naukri_selectors.rs crates/careerai-core/src/config.rs
git commit -s -m "feat(submit): scaffold Naukri browser submitter (stub run_session)"

git add crates/careerai-submit/tests/naukri_fixture_it.rs crates/careerai-submit/src/lib.rs
git commit -s -m "feat(submit): wire Naukri into submit_application + fixture test"

git add crates/careerai-cli/src/cookies.rs crates/careerai-cli/src/main.rs
git commit -s -m "feat(cli): careerai cookies refresh helper for linkedin + naukri"
```

### Task 2.6: Implement `NaukriSubmitter::run_session` body (the actual click flow)

**Files:** Modify: `crates/careerai-submit/src/naukri.rs`

Replace `unimplemented!()` with the actual chromiumoxide flow. Mirror `linkedin.rs::run_session` step-by-step:

- [ ] **Step 1:** inject `naukrisession_cookie` via `Page::set_cookies` — adapt the LinkedIn equivalent.
- [ ] **Step 2:** navigate to `ctx.listing.url` with `Page::goto_with_timeout`.
- [ ] **Step 3:** detect login required via `LOGIN_REQUIRED_INDICATOR` — if present, return `Err(SubmitError::Credential)`.
- [ ] **Step 4:** click `APPLY_BUTTON_SELECTOR`.
- [ ] **Step 5:** wait for confirm modal.
- [ ] **Step 6:** screenshot to `cfg.screenshots_dir/<application_id>.png`.
- [ ] **Step 7:** commit rate permit (pull from `permit_slot`).
- [ ] **Step 8:** click `CONFIRM_APPLY_SELECTOR`.
- [ ] **Step 9:** wait for success indicator ("Applied successfully" toast or URL change).
- [ ] **Step 10:** return remote-id (Naukri may not surface one — fall back to `ctx.listing.external_id`).

Test against the captured fixture from Task 2.5 step 5.

Commit:

```bash
git add crates/careerai-submit/src/naukri.rs
git commit -s -m "feat(submit): implement Naukri Apply click flow"
```

**Definition of Done for Phase 2:** Drafted state variant + tests; LinkedinSubmitConfig.interactive_only field; pipeline short-circuits LinkedIn to drafted when interactive_only=true; `careerai review` walks drafted apps and clicks submit on y; Naukri submitter implemented + fixture test green; `cookies refresh` works for both providers. Operator does ≥3 manual review-and-submits on real LinkedIn jobs and ≥3 Naukri auto-applies via the daemon.

---

## Phase 3 — W3: daily digest CLI

**Outcome:** `careerai digest` prints a daily summary in < 1 second.

### Task 3.1: Add `Digest` subcommand to CLI

**Files:**
- Create: `crates/careerai-cli/src/digest.rs`
- Modify: `crates/careerai-cli/src/main.rs`
- Modify: `crates/careerai-pipeline/src/lib.rs` — add `pub async fn digest_summary(root, since: chrono::Duration) -> Result<DigestReport>`

`DigestReport` shape:

```rust
#[derive(Debug)]
pub struct DigestReport {
    pub since_iso: String,
    pub discovered: usize,
    pub matched: usize,
    pub shortlisted: usize,
    pub drafted: usize,
    pub submitted: usize,
    pub failed: usize,
    pub responded: usize,
    pub per_source: HashMap<String, SourceCounts>,
    pub last_tick: Option<String>,         // last cron tick timestamp from events table
    pub cookie_warnings: Vec<String>,      // e.g. "li_at expires in <2 days"
}
```

- [ ] **Step 1: TDD — write a test for `digest_summary` against a seeded DB.** Mirror the apply_it.rs scaffold pattern. Seed 5 listings in various states, call `digest_summary`, assert counts. Commit after green.

- [ ] **Step 2: Implement `digest_summary`** — single SQL query with `GROUP BY state` does most of the work; per-source counts via JOIN to listings.

- [ ] **Step 3: Implement `careerai-cli/src/digest.rs::run_digest(root, since_arg)`** that calls `pipeline::digest_summary` and pretty-prints. Format (5 lines, color when non-zero failure):

```
career-ai digest — last 24h (since 2026-04-25T08:00Z)
  pipeline:  discovered: 47   matched: 12   shortlisted: 8   drafted: 3   submitted: 4   failed: 0
  responded: 1 (positive)     waiting: 7
  sources:   greenhouse 18 / lever 12 / remotive 9 / linkedin 5 / naukri 3
  last cron tick: 2026-04-25T11:42:09Z (2 min ago)
  warnings:  li_at expires in 1 day 6 hours — run `careerai cookies refresh linkedin`
```

- [ ] **Step 4: Wire `Digest { since: Option<String> }` subcommand** in `main.rs`. Default `since="24h"`.

- [ ] **Step 5: Smoke test** with empty DB (should print all zeros, no panic).

- [ ] **Step 6: Commit:**

```bash
git add crates/careerai-cli/src/digest.rs crates/careerai-cli/src/main.rs crates/careerai-pipeline/src/lib.rs
git commit -s -m "feat(cli): careerai digest daily pipeline summary"
```

### Task 3.2: Cookie expiry detection for digest warnings

**Files:**
- Modify: `crates/careerai-submit/src/credentials.rs` (or wherever the keyring access lives) — add `pub fn cookie_expiry(provider: &str) -> Option<DateTime<Utc>>`. Decode the JWT exp claim from `li_at` (LinkedIn cookie is a JWT). For Naukri, the cookie is opaque — return `None` and the digest just doesn't warn for Naukri.
- Modify: `pipeline::digest_summary` — populate `cookie_warnings` if expiry is < 48h away.

- [ ] **Step 1: TDD — test `cookie_expiry("linkedin")` against a synthetic JWT.** Use `base64url` decode + serde_json on the middle segment. Assert correct expiry.

- [ ] **Step 2: Implement.** Skip JWT signature verification — we only care about the exp claim's value, not its trust.

- [ ] **Step 3: Wire into digest_summary.** Test the warning path with a cookie that expires in 1 hour.

- [ ] **Step 4: Commit:**

```bash
git add crates/careerai-submit/src/credentials.rs crates/careerai-pipeline/src/lib.rs
git commit -s -m "feat(digest): warn when linkedin cookie expires within 48h"
```

**Definition of Done for Phase 3:** `careerai digest` runs in < 1s on a seeded DB, surfaces meaningful counts, warns when LinkedIn cookie is close to expiry. Operator integrates it into their daily review.

---

## Phase 4 — W4: interview-prep crate

**Outcome:** `careerai mark-responded <id>` flips state to `Responded` and triggers `careerai prep <id>` automatically (or operator runs `prep` manually). Output: `prep/<application_id>.md` study sheet.

### Task 4.1: New crate `careerai-prep`

**Files:**
- Create: `crates/careerai-prep/Cargo.toml`
- Create: `crates/careerai-prep/src/lib.rs`
- Create: `crates/careerai-prep/src/types.rs`
- Create: `crates/careerai-prep/src/template.rs`
- Modify: `Cargo.toml` (root) — add `crates/careerai-prep` to workspace members.
- Modify: `crates/careerai-cli/Cargo.toml` — add `careerai-prep = { path = "../careerai-prep" }`.

- [ ] **Step 1: Scaffold the crate** — `Cargo.toml` mirrors careerai-tailor's deps:

```toml
[package]
name = "careerai-prep"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "Interview prep study sheets generated from JD + resume diff"

[dependencies]
careerai-core = { path = "../careerai-core" }
careerai-db = { path = "../careerai-db" }
careerai-llm = { path = "../careerai-llm" }
careerai-tailor = { path = "../careerai-tailor" }
careerai-profile = { path = "../careerai-profile" }
anyhow.workspace = true
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
tracing.workspace = true
tera.workspace = true
chrono.workspace = true

[dev-dependencies]
tempfile.workspace = true
insta.workspace = true

[lints]
workspace = true
```

- [ ] **Step 2: Add to workspace members.** Edit root `Cargo.toml` `[workspace] members = [...]` list.

- [ ] **Step 3: lib.rs scaffold:**

```rust
//! Interview prep generator. Given an application_id, produces a markdown
//! study sheet at `<root>/prep/<application_id>.md` with sections:
//! JD summary, likely topics, behavioral question shortlist, resume bullet
//! → JD-keyword map, company recent news.
//!
//! All LLM-generated content goes through the same constrained-output
//! discipline as careerai-tailor: ground claims in JD or web search,
//! never invent.

#![forbid(unsafe_code)]

pub mod error;
pub mod template;
pub mod types;

pub use error::{PrepError, Result};
pub use types::PrepSheet;

use std::path::Path;
use careerai_core::config::CoreConfig;

/// Generate the prep sheet for `application_id`. Writes to
/// `<root>/prep/<application_id>.md`. Returns the path.
pub async fn generate(
    root: &Path,
    cfg: &CoreConfig,
    application_id: &str,
) -> Result<std::path::PathBuf> {
    todo!("Phase 4 step-by-step below")
}
```

Commit the scaffold + workspace wiring.

- [ ] **Step 4: Define `PrepSheet` type** in `types.rs`:

```rust
#[derive(Debug, serde::Serialize)]
pub struct PrepSheet {
    pub application_id: String,
    pub job_title: String,
    pub company: String,
    pub jd_summary: String,
    pub likely_topics: Vec<String>,
    pub behavioral_questions: Vec<String>,
    pub bullet_to_keyword: Vec<BulletKeyword>,
    pub company_news: Vec<String>,
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, serde::Serialize)]
pub struct BulletKeyword {
    pub bullet: String,
    pub keywords: Vec<String>,
}
```

Test the serialization round-trip. Commit.

- [ ] **Step 5: Add Tera template** at `templates/prep_sheet.md.tera`:

```markdown
# Interview prep — {{ job_title }} @ {{ company }}

_Generated {{ generated_at }} for application {{ application_id }}._

## JD summary

{{ jd_summary }}

## Likely topics

{% for t in likely_topics %}- {{ t }}
{% endfor %}

## Behavioral question shortlist

{% for q in behavioral_questions %}- {{ q }}
{% endfor %}

## Talking points (resume bullet → JD keywords)

{% for entry in bullet_to_keyword %}**{{ entry.bullet }}**
  Keywords: {% for k in entry.keywords %}`{{ k }}`{% if not loop.last %}, {% endif %}{% endfor %}

{% endfor %}

## Company recent news

{% for n in company_news %}- {{ n }}
{% endfor %}
```

Commit (template + initial render unit test).

### Task 4.2: Implement `generate` body — the LLM call + render

**Files:** Modify: `crates/careerai-prep/src/lib.rs`

- [ ] **Step 1: TDD — test that `generate` produces a non-empty file** for a seeded application with a stubbed `MockLlm`. Use the same mock-llm fixture pattern as careerai-tailor's `tests/tailor_roundtrip.rs`.

- [ ] **Step 2: Implement** — load application → load listing JD → load profile → call LLM with a constrained prompt asking for exactly the fields in `PrepSheet` (JSON output) → render via Tera → write to `<root>/prep/<application_id>.md`.

The LLM prompt template lives at `templates/prompts/prep_sheet.tera` (3-segment: system / profile_block (cached) / user). System message: "Output JSON matching schema X. Ground every claim in the JD or the provided company news. Do not invent facts."

- [ ] **Step 3: Add insta snapshot test** — fixed seed input, snapshot the generated markdown.

- [ ] **Step 4: Commit:**

```bash
git add crates/careerai-prep/src/lib.rs crates/careerai-prep/templates/prep_sheet.md.tera crates/careerai-prep/tests/
git commit -s -m "feat(prep): generate interview prep sheets from JD + resume diff"
```

### Task 4.3: `careerai mark-responded <id>` CLI

**Files:**
- Create: `crates/careerai-cli/src/mark_responded.rs`
- Modify: `crates/careerai-cli/src/main.rs`
- Modify: `crates/careerai-pipeline/src/lib.rs` — add `pub async fn mark_responded(root, application_id, note)`.

- [ ] **Step 1: TDD — `pipeline::mark_responded` flips an application from any state to Responded** (or errors on unknown id). Test commit.

- [ ] **Step 2: Wire `MarkResponded { application_id: String, note: Option<String> }` subcommand.** Optionally invoke `careerai-prep::generate` immediately after the transition (opt-out via `--no-prep` flag). Test commit.

```bash
git add crates/careerai-cli/src/mark_responded.rs crates/careerai-cli/src/main.rs crates/careerai-pipeline/src/lib.rs
git commit -s -m "feat(cli): careerai mark-responded transitions + auto-generates prep sheet"
```

### Task 4.4: `careerai prep <id>` CLI (manual generation)

**Files:**
- Create: `crates/careerai-cli/src/prep.rs`
- Modify: `crates/careerai-cli/src/main.rs`

- [ ] **Step 1: Add subcommand** that just calls `careerai_prep::generate(root, cfg, application_id)` and prints the output path.

- [ ] **Step 2: Test by smoke** — run against a seeded responded application.

- [ ] **Step 3: Commit:**

```bash
git add crates/careerai-cli/src/prep.rs crates/careerai-cli/src/main.rs
git commit -s -m "feat(cli): careerai prep <id> generates study sheet on demand"
```

**Definition of Done for Phase 4:** `careerai mark-responded <id>` produces `prep/<id>.md` within 60 seconds. Operator validates content quality on at least 1 real recruiter reply.

---

## Final phase — pre-merge polish

### Task 5.1: Full workspace verification

- [ ] **Step 1:**

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo audit
cargo deny check
```

- [ ] **Step 2:** record final test count (should be 199 + delta from W1–W4 tests).
- [ ] **Step 3:** verify the `careerai daemon` cold-start prints expected sources, registers cron jobs, exits cleanly on Ctrl-C.

### Task 5.2: Operator-side runbook

**Files:** Create: `RUNBOOK.md` in repo root.

- [ ] **Step 1:** literal copy of the spec's "Operator-side runbook (preview)" section, expanded with:
  - exact `cargo install` commands
  - exact `keyring` invocations for cookies + API keys
  - exact YAML diff to enable each source
  - the kill switch
  - a short troubleshooting table (cookie expired / Naukri 403 / LinkedIn modal selector mismatch)

- [ ] **Step 2:** commit RUNBOOK.md, push.

### Task 5.3: Push branch + open PR

- [ ] **Step 1:** push `chore/job-search-sprint-spec` (already pushed; the impl will land on a fresh branch — `feat/job-search-sprint-w1-w4` — that branches off main and gets force-pushed as we go through phases). Per global CLAUDE.md, no direct main commits — every phase commit lands on the feature branch.
- [ ] **Step 2:** open one PR per phase OR one big PR with all phases — operator's call. Recommend per-phase PRs for reviewability.

---

## Self-review against spec

| Spec section | Plan task | Status |
|---|---|---|
| W1 — ATS live mode + matcher tightening | Phase 1 (1.1–1.4) | covered |
| W1 DoD — 3 live ATS submits, no junk in tailored queue | Task 1.4 (operator) | not verified locally |
| W2 — drafted state variant | Task 2.1 | covered |
| W2 — interactive_only flag | Task 2.2 | covered |
| W2 — pipeline short-circuit to drafted | Task 2.3 | covered |
| W2 — `careerai review` CLI | Task 2.4 | covered |
| W2 — Naukri full auto submitter | Tasks 2.5 + 2.6 | covered |
| W2 — cookie refresh helper | Task 2.5 step 7 | covered |
| W3 — `careerai digest` | Task 3.1 | covered |
| W3 — cookie-expiry warnings | Task 3.2 | covered |
| W4 — `careerai-prep` crate | Tasks 4.1 + 4.2 | implemented |
| W4 — `mark-responded` CLI | Task 4.3 | implemented |
| W4 — `prep` CLI manual mode | Task 4.4 | implemented with allowlisted research inputs |
| Risks — LinkedIn account flag | covered by interactive_only=true default + Task 2.3 short-circuit | covered |
| Risks — Naukri rate-limit | covered by reusing M5a governor + RatePolicy | covered |
| Risks — operator forgets cookie refresh | Task 3.2 (digest warnings) | covered |
| Operator-side runbook | Task 5.2 | local instructions documented; live host not verified |

The table above is a scope reconciliation, not evidence that every historical
checkbox was executed. Live ATS operation, credential provisioning, and host
deployment remain operator acceptance gates.

---

## Stage gate

This plan does NOT include:
- M6.1 polish backlog (deferred per spec)
- OSS framework prep (deferred until offer signed)
- Indeed / Glassdoor / Wellfound submitters
- Inbox-poll auto-detection of `responded` state
- Web UI / dashboard
- cargo-mutants / proptest / fuzz testing rigor

These items remain parked. Re-evaluate after offer signed OR 6 weeks elapsed without 3 interviews.
