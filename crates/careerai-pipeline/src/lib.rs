//! Pipeline orchestration: discover / match / tailor / render / apply.
//!
//! Owns the linear-state-machine entry points the CLI and scheduler both
//! drive. Lives in its own crate (rather than `careerai-core`) because the
//! implementation crates it pulls in — `careerai-db`, `careerai-sources`,
//! `careerai-match`, `careerai-tailor`, `careerai-render`, `careerai-submit`,
//! `careerai-llm`, `careerai-profile` — all already depend on `careerai-core`
//! for shared types. Putting orchestration in core would create a cycle.
//!
//! Architectural rule: this crate is the *only* place where the pipeline
//! stages get composed end-to-end. Both `careerai-cli` (for one-shot
//! subcommands) and `careerai-scheduler` (for cron ticks) call into here.
//! Neither of them should reach past this layer into the implementation
//! crates directly.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{info, warn};

use careerai_core::config::CoreConfig;
use careerai_core::state::ListingState;
use careerai_db::models::{NewArtifact, NewListing};
use careerai_db::{pool_from_path, queries, SqlitePool};
use careerai_llm::mock::MockLlm;
use careerai_match::{
    classify, flatten_profile, rank_all, score_histogram, split_at_threshold, Decision,
    FilterRules, JaccardScorer,
};
use careerai_profile::Profile;
use careerai_render::render_application;
use careerai_sources::{
    GreenhouseSource, LeverSource, NaukriSource, RawListing, RemoteOkSource, RemotiveSource, Source,
};
use careerai_tailor::model::{CoverLetter, ResumeView};
use careerai_tailor::tailor_for_listing;

pub async fn open_pool(root: &Path) -> Result<SqlitePool> {
    let path = root.join("data").join("careerai.sqlite");
    pool_from_path(&path)
        .await
        .with_context(|| format!("open db at {}", path.display()))
}

/// Build the list of configured source adapters from `CoreConfig`.
fn build_sources(cfg: &CoreConfig) -> Vec<Arc<dyn Source>> {
    let mut out: Vec<Arc<dyn Source>> = Vec::new();
    for company in &cfg.sources.greenhouse.companies {
        out.push(Arc::new(GreenhouseSource::new(company.clone())));
    }
    for company in &cfg.sources.lever.companies {
        out.push(Arc::new(LeverSource::new(company.clone())));
    }
    if cfg.sources.remotive.enabled {
        let mut s = RemotiveSource::new();
        if let Some(cat) = &cfg.sources.remotive.category {
            s = s.with_category(cat.clone());
        }
        out.push(Arc::new(s));
    }
    if cfg.sources.remoteok.enabled {
        out.push(Arc::new(RemoteOkSource::new()));
    }
    if cfg.sources.naukri.enabled {
        let mut s = NaukriSource::new();
        if !cfg.sources.naukri.keywords.is_empty() {
            s = s.with_keywords(cfg.sources.naukri.keywords.clone());
        }
        if let Some(loc) = &cfg.sources.naukri.location {
            s = s.with_location(loc.clone());
        }
        if let Some(n) = cfg.sources.naukri.max_results {
            s = s.with_max_results(n);
        }
        out.push(Arc::new(s));
    }
    out
}

pub async fn discover_all(
    root: &Path,
    cfg: &CoreConfig,
    source_filter: &[String],
) -> Result<DiscoveryReport> {
    let pool = open_pool(root).await?;
    let mut sources = build_sources(cfg);
    if !source_filter.is_empty() {
        sources.retain(|s| source_filter.iter().any(|f| f == s.name()));
    }
    if sources.is_empty() {
        warn!("no sources enabled — edit config/default.yaml or config/local.yaml");
        return Ok(DiscoveryReport::default());
    }

    let mut report = DiscoveryReport::default();
    for source in sources {
        let name = source.name();
        match source.discover().await {
            Ok(listings) => {
                info!(source = name, count = listings.len(), "discovered");
                report.fetched += listings.len();
                for raw in listings {
                    let new = NewListing {
                        source: raw.source,
                        external_id: raw.external_id,
                        title: raw.title,
                        company: raw.company,
                        location: raw.location,
                        url: raw.url,
                        description: raw.description,
                        raw_json: raw.raw_json,
                    };
                    match queries::insert_or_ignore(&pool, &new).await {
                        Ok((_, true)) => report.new_rows += 1,
                        Ok((_, false)) => report.duplicates += 1,
                        Err(e) => {
                            warn!(error = %e, "persist failed");
                            report.errors += 1;
                        }
                    }
                }
            }
            Err(e) => {
                warn!(source = name, error = %e, "source failed");
                report.errors += 1;
            }
        }
    }
    Ok(report)
}

/// Run discovery for a single configured source. Used by the scheduler's
/// per-source cron tick — each tick fires this for exactly one source.
///
/// Internally delegates to `discover_all` with a one-element filter so the
/// adapter-construction logic stays in one place. Returns an empty report
/// (with no error) if `source` doesn't match any enabled adapter, matching
/// the behavior of `discover_all` with an unmatched filter.
pub async fn discover_one(root: &Path, cfg: &CoreConfig, source: &str) -> Result<DiscoveryReport> {
    let filter = [source.to_owned()];
    discover_all(root, cfg, &filter).await
}

#[derive(Debug, Default)]
pub struct DiscoveryReport {
    pub fetched: usize,
    pub new_rows: usize,
    pub duplicates: usize,
    pub errors: usize,
}

pub async fn match_all(root: &Path, cfg: &CoreConfig, tune: bool) -> Result<MatchReport> {
    let pool = open_pool(root).await?;
    let profile = load_profile(root)?;
    let rules = FilterRules::load(root).context("load rules")?;

    let discovered = queries::list_by_state(&pool, ListingState::Discovered, 10_000)
        .await
        .context("list discovered")?;
    info!(count = discovered.len(), "matching against profile");

    let raws: Vec<RawListing> = discovered
        .iter()
        .map(|l| RawListing {
            source: l.source.clone(),
            external_id: l.external_id.clone(),
            title: l.title.clone(),
            company: l.company.clone(),
            location: l.location.clone(),
            url: l.url.clone(),
            description: l.description.clone(),
            raw_json: l.raw_json.clone(),
        })
        .collect();

    // Apply hard filters first. When not in tune mode, rejected listings are
    // transitioned to `FilteredOut` so they are skipped on subsequent runs.
    let mut post_filter: Vec<(&careerai_db::models::Listing, RawListing)> = Vec::new();
    let mut filtered_out = 0usize;
    for (db_row, raw) in discovered.iter().zip(raws) {
        match classify(&raw, cfg, &rules) {
            Decision::Keep => post_filter.push((db_row, raw)),
            Decision::Reject(reason) => {
                filtered_out += 1;
                if !tune {
                    queries::transition(&pool, &db_row.id, ListingState::FilteredOut, Some(reason))
                        .await?;
                }
            }
        }
    }

    let profile_text = flatten_profile(&profile);
    let raws_only: Vec<RawListing> = post_filter.iter().map(|(_, r)| r.clone()).collect();
    let ranked = rank_all(&JaccardScorer, &profile_text, &raws_only);

    if tune {
        // Tuning mode: don't persist, just return the histogram so the CLI
        // can print it.
        return Ok(MatchReport {
            filtered_out,
            shortlisted: 0,
            also_filtered: 0,
            histogram: score_histogram(&ranked),
        });
    }

    let threshold = cfg.matching.score_threshold;
    let (keep, drop) = split_at_threshold(ranked, threshold);

    for scored in &keep {
        let db_row = post_filter
            .iter()
            .find(|(_, r)| {
                r.source == scored.listing.source && r.external_id == scored.listing.external_id
            })
            .map(|(d, _)| d)
            .context("bug: scored listing missing from post_filter map")?;
        queries::set_score(&pool, &db_row.id, f64::from(scored.score)).await?;
        queries::transition(
            &pool,
            &db_row.id,
            ListingState::Shortlisted,
            Some(&format!("score={:.3}", scored.score)),
        )
        .await?;
    }

    for scored in &drop {
        let db_row = post_filter
            .iter()
            .find(|(_, r)| {
                r.source == scored.listing.source && r.external_id == scored.listing.external_id
            })
            .map(|(d, _)| d)
            .context("bug: below-threshold listing missing")?;
        queries::set_score(&pool, &db_row.id, f64::from(scored.score)).await?;
        queries::transition(
            &pool,
            &db_row.id,
            ListingState::FilteredOut,
            Some(&format!("below threshold ({:.3})", scored.score)),
        )
        .await?;
    }

    Ok(MatchReport {
        filtered_out,
        shortlisted: keep.len(),
        also_filtered: drop.len(),
        histogram: [(0.0, 0); 10],
    })
}

/// Per-source match wrapper used by the scheduler's cron tick.
///
/// Matching is naturally global — `match_all` walks every `discovered`-state
/// row regardless of source, and the filter/score logic doesn't read the
/// `source` column. So `match_one` just delegates to `match_all`. The
/// `_source` argument is accepted for symmetry with `discover_one` and to
/// give the scheduler a place to attach span context per tick. Keeping match
/// global also means a tick that fired discover for source A still rescores
/// any listings from source B that arrived earlier — desirable when the
/// profile or rules have been edited between ticks.
pub async fn match_one(root: &Path, cfg: &CoreConfig, _source: &str) -> Result<MatchReport> {
    match_all(root, cfg, false).await
}

#[derive(Debug, Default)]
pub struct MatchReport {
    pub filtered_out: usize,
    pub shortlisted: usize,
    pub also_filtered: usize,
    pub histogram: [(f32, usize); 10],
}

pub async fn shortlist_show(root: &Path, limit: i64) -> Result<Vec<careerai_db::Listing>> {
    let pool = open_pool(root).await?;
    let rows = queries::list_by_state(&pool, ListingState::Shortlisted, limit)
        .await
        .context("list shortlisted")?;
    Ok(rows)
}

fn load_profile(root: &Path) -> Result<Profile> {
    let path = root.join("profile").join("profile.yaml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    Profile::from_yaml(&text).context("parse profile yaml")
}

#[derive(Debug)]
pub struct TailoredOutcome {
    pub application_id: String,
    pub listing_title: String,
    pub company: String,
}

#[derive(Debug)]
pub struct RenderedOutcome {
    pub application_id: String,
    pub resume_md: PathBuf,
    pub resume_docx: PathBuf,
    pub resume_pdf: PathBuf,
    pub cover_md: PathBuf,
    pub cover_docx: PathBuf,
    pub bytes: BTreeMap<PathBuf, u64>,
}

/// Resolve the LLM fixtures directory. Honors the `CAREERAI_LLM_FIXTURES_DIR`
/// environment override, else falls back to `<root>/data/cache/llm/fixtures`.
fn fixtures_dir(root: &Path) -> PathBuf {
    std::env::var("CAREERAI_LLM_FIXTURES_DIR").map_or_else(
        |_| root.join("data").join("cache").join("llm").join("fixtures"),
        PathBuf::from,
    )
}

/// Tailor a shortlisted listing into an application row + persisted payload.
///
/// Uses a `MockLlm` sourced from `CAREERAI_LLM_FIXTURES_DIR` (default
/// `<root>/data/cache/llm/fixtures`). A live provider is only available with
/// `cargo build --features live-llm` and `CAREERAI_LLM_LIVE=1`.
pub async fn tailor_one(
    root: &Path,
    cfg: &CoreConfig,
    listing_id: &str,
) -> Result<TailoredOutcome> {
    let pool = open_pool(root).await?;

    let listing = match queries::find_by_id(&pool, listing_id).await {
        Ok(l) => l,
        Err(careerai_db::DbError::NotFound(_)) => {
            anyhow::bail!("listing not found: {listing_id}");
        }
        Err(e) => return Err(e).context("fetch listing"),
    };

    if listing.state != ListingState::Shortlisted.as_str() {
        anyhow::bail!(
            "listing {listing_id} is in state '{}'; expected 'shortlisted'",
            listing.state
        );
    }

    let profile = load_profile(root)?;

    // Build the LLM. Default: MockLlm from a fixtures dir. A live provider
    // can be wired in future via the `live-llm` feature; the CLI re-exports
    // that feature so `cargo build -p careerai-cli --features live-llm`
    // compiles the rig-core dependency.
    #[cfg(feature = "live-llm")]
    {
        // Intentionally a no-op today: constructing a RigLlm requires env
        // credentials (ANTHROPIC_API_KEY / OPENAI_API_KEY) and prompt_cache
        // wiring. Tracked for a follow-up wave.
        if std::env::var("CAREERAI_LLM_LIVE").ok().as_deref() == Some("1") {
            anyhow::bail!(
                "live-llm runtime path not wired yet; unset CAREERAI_LLM_LIVE and point \
                 CAREERAI_LLM_FIXTURES_DIR at a fixtures directory for now"
            );
        }
    }

    let fixtures = fixtures_dir(root);
    if !fixtures.is_dir() {
        anyhow::bail!(
            "no LLM fixtures at {}; either set CAREERAI_LLM_FIXTURES_DIR or enable \
             --features live-llm (not yet available in this CLI build)",
            fixtures.display()
        );
    }
    let llm = MockLlm::from_dir(&fixtures)
        .with_context(|| format!("load llm fixtures from {}", fixtures.display()))?;

    info!(
        target = "tailor",
        listing_id = %listing.id,
        title = %listing.title,
        company = %listing.company,
        "tailoring listing"
    );

    let outcome = tailor_for_listing(&pool, &llm, &listing.id, &profile, &cfg.llm)
        .await
        .context("tailor_for_listing")?;

    Ok(TailoredOutcome {
        application_id: outcome.application_id,
        listing_title: listing.title,
        company: listing.company,
    })
}

/// Render a tailored application to DOCX + PDF on disk, attach artifact rows,
/// and transition both listing and application to `rendered`.
pub async fn render_one(
    root: &Path,
    cfg: &CoreConfig,
    application_id: &str,
) -> Result<RenderedOutcome> {
    let pool = open_pool(root).await?;

    let application = match queries::find_application_by_id(&pool, application_id).await {
        Ok(a) => a,
        Err(careerai_db::DbError::NotFound(_)) => {
            anyhow::bail!("application not found: {application_id}");
        }
        Err(e) => return Err(e).context("fetch application"),
    };

    if application.state != "tailored" {
        anyhow::bail!(
            "application {application_id} is in state '{}'; expected 'tailored'",
            application.state
        );
    }

    let payload = match queries::find_payload_by_application_id(&pool, application_id).await {
        Ok(p) => p,
        Err(careerai_db::DbError::NotFound(_)) => {
            anyhow::bail!("application payload not found: {application_id}");
        }
        Err(e) => return Err(e).context("fetch application payload"),
    };

    let resume_view: ResumeView =
        serde_json::from_str(&payload.resume_view_json).context("deserialize resume_view_json")?;
    let cover_letter = CoverLetter {
        body: payload.cover_letter_text.clone(),
    };

    let listing = queries::find_by_id(&pool, &application.listing_id)
        .await
        .context("fetch listing for application")?;
    let profile = load_profile(root)?;

    // Resolve artifacts_dir against `root` if it's relative so the CLI and
    // integration tests share the same on-disk layout regardless of cwd.
    let mut render_cfg = cfg.render.clone();
    if render_cfg.artifacts_dir.is_relative() {
        render_cfg.artifacts_dir = root.join(&render_cfg.artifacts_dir);
    }

    let artifacts = render_application(
        &render_cfg,
        &application.id,
        &resume_view,
        &cover_letter,
        &profile.personal.name,
        &listing.company,
    )
    .await
    .context("render_application")?;

    // Attach artifact rows for each rendered file. `bytes` is the canonical
    // size map returned from render; we look each path up there.
    for (kind, path) in [
        ("resume_md", &artifacts.resume_md),
        ("resume_docx", &artifacts.resume_docx),
        ("resume_pdf", &artifacts.resume_pdf),
        ("cover_md", &artifacts.cover_md),
        ("cover_docx", &artifacts.cover_docx),
    ] {
        let size = artifacts.bytes.get(path).copied().unwrap_or_default();
        queries::attach_artifact(
            &pool,
            &application.id,
            &NewArtifact {
                kind: kind.to_string(),
                path: path.to_string_lossy().into_owned(),
                bytes: i64::try_from(size).unwrap_or(i64::MAX),
            },
        )
        .await
        .with_context(|| format!("attach_artifact {kind}"))?;
    }

    queries::transition(
        &pool,
        &application.listing_id,
        ListingState::Rendered,
        Some(&format!("app={}", application.id)),
    )
    .await
    .context("transition listing to rendered")?;
    queries::set_application_state(&pool, &application.id, "rendered")
        .await
        .context("set application state=rendered")?;

    Ok(RenderedOutcome {
        application_id: application.id,
        resume_md: artifacts.resume_md,
        resume_docx: artifacts.resume_docx,
        resume_pdf: artifacts.resume_pdf,
        cover_md: artifacts.cover_md,
        cover_docx: artifacts.cover_docx,
        bytes: artifacts.bytes,
    })
}

// --- apply / applied / inspect (M4 wave 2) ---------------------------------

/// One row of `apply --all` output. One `AppliedOutcome` is emitted per
/// application the CLI attempted to submit, regardless of whether the
/// per-call result was success, skip, or dry-run.
#[derive(Debug)]
pub struct AppliedOutcome {
    pub application_id: String,
    pub source: String,
    pub outcome: careerai_submit::SubmitOutcome,
}

/// Structured payload for `careerai inspect <application_id>`.
#[derive(Debug)]
pub struct InspectReport {
    pub application: careerai_db::Application,
    pub listing_title: String,
    pub listing_company: String,
    pub listing_source: String,
    pub events: Vec<careerai_db::Event>,
    pub artifacts: Vec<careerai_db::Artifact>,
}

/// Build a per-call `SubmitConfig` honoring the optional CLI override.
///
/// When `override_auto_submit` is `Some(true)` the call is forced live;
/// `Some(false)` forces dry-run; `None` passes the config's stored value
/// through unchanged.
fn effective_submit_cfg(
    cfg: &CoreConfig,
    override_auto_submit: Option<bool>,
) -> careerai_core::config::SubmitConfig {
    let mut submit = cfg.submit.clone();
    if let Some(v) = override_auto_submit {
        submit.auto_submit = v;
    }
    submit
}

/// Submit a single prepared application (state `rendered` or `prepared`).
///
/// Gating, state transitions, and artifact loading all live inside
/// `careerai_submit::submit_application`; this wrapper only opens the pool
/// and overlays the `--auto-submit` flag on top of the config.
pub async fn apply_one(
    root: &Path,
    cfg: &CoreConfig,
    application_id: &str,
    auto_submit_override: Option<bool>,
) -> Result<AppliedOutcome> {
    let pool = open_pool(root).await?;

    // Pre-fetch surfaces the typed errors from careerai-db; the CLI's
    // exit-code mapper downcasts to `DbError::NotFound` directly. We do
    // NOT bail!() into a stringly-typed error here — that was previously
    // breaking exit-code classification once any `.context(...)` wrapper
    // ran upstream.
    let application = queries::find_application_by_id(&pool, application_id).await?;
    let listing = queries::find_by_id(&pool, &application.listing_id).await?;

    let submit_cfg = effective_submit_cfg(cfg, auto_submit_override);
    let outcome = careerai_submit::submit_application(&pool, &submit_cfg, root, application_id)
        .await
        .context("submit_application")?;

    Ok(AppliedOutcome {
        application_id: application.id,
        source: listing.source,
        outcome,
    })
}

/// Iterate every application currently in state `rendered` or `prepared`
/// and submit each one. Per-application failures are logged and skipped —
/// `apply --all` deliberately doesn't abort the batch on the first error.
pub async fn apply_all(
    root: &Path,
    cfg: &CoreConfig,
    source_filter: Option<&str>,
    auto_submit_override: Option<bool>,
) -> Result<Vec<AppliedOutcome>> {
    let pool = open_pool(root).await?;

    // Both "rendered" and "prepared" are eligible per submit_application's
    // BadState guard. Use the JOIN-based query so a `--source` filter is
    // pushed into SQL — previously the CLI fetched every listing per row
    // (O(N) round-trips) and filtered client-side.
    let mut eligible: Vec<careerai_db::Application> = Vec::new();
    for state in ["rendered", "prepared"] {
        let rows =
            queries::list_applications_by_state_and_source(&pool, state, source_filter, 1_000)
                .await
                .with_context(|| format!("list applications in state '{state}'"))?;
        eligible.extend(rows);
    }

    // Drop the local pool so `apply_one` opens its own — matches the
    // established convention in `tailor_render_it.rs` and avoids holding a
    // WAL writer across the loop.
    drop(pool);

    let mut out = Vec::with_capacity(eligible.len());
    for app in eligible {
        match apply_one(root, cfg, &app.id, auto_submit_override).await {
            Ok(o) => out.push(o),
            Err(e) => {
                warn!(application_id = %app.id, error = %e, "apply_one failed, continuing batch");
            }
        }
    }
    Ok(out)
}

/// List applications already submitted, newest first. Optional `source`
/// filter matches against the linked listing's source.
pub async fn applied_show(
    root: &Path,
    source_filter: Option<&str>,
    limit: i64,
) -> Result<Vec<careerai_db::Application>> {
    let pool = open_pool(root).await?;
    queries::list_applications_by_state_and_source(&pool, "submitted", source_filter, limit)
        .await
        .context("list submitted applications")
}

/// Gather everything needed to render `careerai inspect <id>`.
pub async fn inspect_show(root: &Path, application_id: &str) -> Result<InspectReport> {
    let pool = open_pool(root).await?;

    // Same typed-error pattern as apply_one: don't bail!() into strings.
    let application = queries::find_application_by_id(&pool, application_id).await?;
    let listing = queries::find_by_id(&pool, &application.listing_id).await?;
    let events = queries::events_for(&pool, &listing.id)
        .await
        .context("events_for listing")?;
    let artifacts = queries::list_artifacts(&pool, &application.id)
        .await
        .context("list_artifacts")?;

    Ok(InspectReport {
        application,
        listing_title: listing.title,
        listing_company: listing.company,
        listing_source: listing.source,
        events,
        artifacts,
    })
}
