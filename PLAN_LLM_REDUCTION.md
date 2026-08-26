# Plan: Reducing LLM Dependency for Scale (100s of JDs)

## Problem Statement

The current pipeline calls the LLM **twice per job description**:
1. **Tailor call** — sends full profile YAML + JD, gets back a JSON diff (keep/reword/drop/move_before per bullet)
2. **Cover letter call** — separate LLM call for a plain-text letter

For **100 JDs** that's **200 LLM calls**, each sending the full profile (which can be 2-4K tokens) plus the JD. Even with:
- On-disk response cache (keyed by prompt_version + profile_hash + jd_hash + model)
- Anthropic prompt caching on the profile block

…different JDs produce different cache keys, so **every new JD = 2 fresh LLM calls**. At Claude Sonnet pricing (~$3/M input, $15/M output), 100 JDs with a 3K-token profile + 1K-token JD + 2K-token response ≈ **$4-6 per run**. That adds up fast across daily daemon runs.

## Current Architecture (as-is)

```
JD → [LLM: tailor diff] → validate (9 rules) → apply → ResumeView
                                   ↘ [LLM: cover letter] → CoverLetter
                                          ↓
                               [Tera → Markdown → Pandoc] → DOCX + PDF
```

**LLM does three things today:**
1. **Reorders** bullets (move_before) — *this is a ranking problem, solvable locally*
2. **Drops** weak bullets — *this is a scoring problem, solvable locally*
3. **Rewords** bullets + summary — *this genuinely needs language understanding*
4. **Drafts** cover letter — *this genuinely needs language generation*

The key insight: **only rewording and cover-letter drafting actually need the LLM.** Reordering and dropping are ranking/scoring problems that the match crate already has infrastructure for.

## Strategies

### Strategy 1: Rule-Based Reorder + Drop (No LLM)
**Eliminates the need for LLM to do move_before/drop operations.**

Use the existing `Scorer` trait + `fastembed` embeddings to score each bullet against the JD. Bullets that score below a threshold get dropped. Remaining bullets get reordered by score descending. This produces the same structural transformation the LLM does today, but deterministically and in milliseconds.

**What the LLM is no longer needed for:** bullet reordering, bullet dropping.
**What still needs LLM:** bullet rewording (optional), summary rewording (optional), cover letter.

**Implementation:** New `LocalTailor` struct that implements bullet scoring + reordering + dropping without any LLM call. The diff ops become: `drop` for low-score bullets, `keep` for the rest, `move_before` to sort by relevance. No `reword` ops — bullets keep their original text.

### Strategy 2: Pre-Computed Bullet Variants (1 LLM call per profile, not per JD)
**Eliminates per-JD rewording entirely.**

Instead of asking the LLM to reword bullets for each JD, pre-compute **N variants** of each bullet once (during profile import or a one-time "compile" step). Each variant emphasizes a different skill/domain. At tailor time, pick the best variant per bullet via keyword/embedding match against the JD — no LLM call.

Example:
- Original: "Built distributed pipeline processing 10M events/day"
- Variant A (streaming): "Architected streaming pipeline processing 10M events/day with backpressure handling"
- Variant B (reliability): "Hardened event pipeline to 99.99% uptime processing 10M events/day"
- Variant C (scale): "Scaled pipeline from 1M to 10M events/day with zero data loss"

At runtime, score each variant against the JD and pick the best match. All variants pass the existing guardrails because they're generated once and validated upfront.

**Cost:** 1 LLM call per profile (generate variants) instead of 1 per JD.
**What the LLM is no longer needed for:** per-JD rewording.

### Strategy 3: Template-Based Cover Letters (No LLM for most JDs)
**Eliminates per-JD cover letter LLM calls.**

Pre-compute 3-5 cover letter "skeletons" per profile, each emphasizing a different domain/skill area. Each skeleton has slot-fill markers for company name, job title, and 2-3 keyword phrases extracted from the JD. At runtime:
1. Score the JD against the profile's domains
2. Pick the best-matching skeleton
3. Extract key phrases from the JD (title, top 3 keywords)
4. Fill slots via string templating

Only fall back to LLM for JDs that don't match any pre-computed skeleton (low-confidence edge case).

**Cost:** 3-5 LLM calls per profile (generate skeletons) instead of 1 per JD.
**What the LLM is no longer needed for:** per-JD cover letter drafting (for 90%+ of JDs).

### Strategy 4: JD Clustering (1 LLM call per cluster)
**Reduces LLM calls by grouping similar JDs.**

Before tailoring, cluster all shortlisted JDs by embedding similarity. Tailor once per cluster representative, then apply the same tailoring to all JDs in the cluster. Clusters with high cohesion (cosine > 0.85) share the same diff; borderline JDs get their own LLM call.

**Cost:** ~10-30 LLM calls for 100 JDs (depending on cluster cohesion) instead of 200.
**What the LLM is no longer needed for:** redundant tailoring of near-identical JDs.

### Strategy 5: Batched LLM Calls (1 call for N JDs)
**Reduces LLM call overhead by batching.**

Instead of 1 LLM call per JD, send a batch of 5-10 JDs in a single prompt and get back N diffs in one response. This amortizes the profile-block tokens across multiple JDs (the profile is sent once, not N times).

**Cost:** ~10-20 LLM calls for 100 JDs instead of 200.
**Trade-off:** Larger output window needed, higher chance of truncation/parse failure.

## Recommended Architecture (Hybrid)

Combining Strategies 1 + 2 + 3 + 4, the pipeline becomes:

```
                    ┌─ Phase 0: Profile Compile (1-time, ~5 LLM calls) ─┐
                    │  • Generate N bullet variants per bullet            │
                    │  • Generate 3-5 cover letter skeletons              │
                    │  • Validate all variants/skeletons through guardrails│
                    └────────────────────────────────────────────────────┘
                                         │
                    ┌─ Phase 1: Discover + Match (no LLM) ──────────────┐
                    │  • Existing source adapters pull JDs               │
                    │  • Jaccard/embedding scoring + threshold           │
                    │  • Shortlist top-N                                  │
                    └────────────────────────────────────────────────────┘
                                         │
                    ┌─ Phase 2: Cluster (no LLM) ───────────────────────┐
                    │  • Embed all shortlisted JDs                       │
                    │  • Cluster by cosine similarity (> 0.85)           │
                    │  • Pick cluster representatives                    │
                    └────────────────────────────────────────────────────┘
                                         │
                    ┌─ Phase 3: Tailor (LLM only for cluster reps) ─────┐
                    │  For each cluster representative:                   │
                    │  • Score bullets vs JD → reorder + drop (local)    │
                    │  • Select best variant per bullet (local)          │
                    │  • [Optional] LLM reword summary (1 call/cluster)   │
                    │  For non-representative JDs in cluster:             │
                    │  • Reuse cluster's diff, re-score variants locally  │
                    └────────────────────────────────────────────────────┘
                                         │
                    ┌─ Phase 4: Cover Letter (mostly no LLM) ───────────┐
                    │  • Match JD to best skeleton (local)               │
                    │  • Slot-fill company/title/keywords (local)         │
                    │  • [Fallback] LLM draft for low-confidence JDs      │
                    └────────────────────────────────────────────────────┘
                                         │
                    ┌─ Phase 5: Render (no LLM) ────────────────────────┐
                    │  • Existing Tera → Markdown → Pandoc pipeline       │
                    └────────────────────────────────────────────────────┘
```

### LLM Call Budget Comparison

| Scenario | Current | With Hybrid Plan |
|---|---|---|
| 1 JD | 2 calls | 0-1 calls (variants/skeletons pre-compiled) |
| 100 JDs | 200 calls | 5-15 calls (cluster reps + edge-case fallbacks) |
| 100 JDs, daily run | 200 calls/day | 0-5 calls/day (everything cached after first run) |

## Implementation Phases

### Phase 0: Bullet Scoring Engine (Strategy 1 foundation)
**Goal:** Score individual bullets against a JD without LLM.

**New crate: `careerai-bulletscore`** (or module in `careerai-match`)

- `BulletScorer` trait: `fn score(&self, bullet: &str, jd: &str) -> f32`
- `JaccardBulletScorer` — reuse existing tokenizer from `careerai-match::score`
- `EmbeddingBulletScorer` — fastembed cosine (when fastembed is wired)
- Batch API: `fn score_many(&self, bullets: &[String], jd: &str) -> Vec<f32>`

**Deliverables:**
- [ ] `BulletScorer` trait + `JaccardBulletScorer` impl
- [ ] Unit tests: high-score for relevant bullets, low-score for irrelevant
- [ ] Integration test: score all bullets in `profile.yaml` against a sample JD

### Phase 1: Local Tailor (Strategy 1)
**Goal:** Produce a `ResumeView` with reordered + dropped bullets, no LLM.

**New module: `careerai-tailor::local`**

- `pub fn tailor_local(profile: &Profile, listing: &Listing, scorer: &dyn BulletScorer, drop_threshold: f32) -> Result<ResumeView>`
- Scores every bullet against the JD
- Drops bullets below `drop_threshold` (respects: ≥1 bullet per experience entry)
- Reorders remaining bullets by score descending (within each entry)
- Selects best pre-computed variant per bullet (if variants exist)
- Returns a `ResumeView` — no `DiffDoc`, no LLM

**Safety:**
- No rewording → no entity guardrail violations possible
- Drop rule: ≥1 bullet per experience (same as validator rule 5)
- All text comes from profile → no invention possible

**Deliverables:**
- [ ] `tailor_local` function
- [ ] Config flag: `tailor.strategy = "local" | "llm" | "hybrid"`
- [ ] Integration test: produces valid `ResumeView` from `profile.yaml` + sample JD
- [ ] Benchmark: <10ms for 50-bullet profile (target: no network, pure CPU)

### Phase 2: Bullet Variant Compiler (Strategy 2)
**Goal:** Pre-compute N reworded variants per bullet, validated through guardrails.

**New module: `careerai-tailor::variants`**

- `pub struct BulletVariants { original: String, variants: Vec<String> }`
- `pub fn compile_variants(profile: &Profile, llm: &dyn Llm, cfg: &LlmConfig) -> Result<ProfileVariants>`
- LLM prompt: "For each bullet below, generate N variants emphasizing different skills. Each variant must preserve all numbers, employer names, and dates from the original."
- Each variant runs through `guardrails::forbid_invented_entities` before acceptance
- Variants stored in DB (new table) or on-disk JSON, keyed by `profile_hash`
- Cache key: `variants.{profile_hash}` — 1 LLM call per profile, cached forever

**Deliverables:**
- [ ] `BulletVariants` / `ProfileVariants` types
- [ ] LLM prompt template: `templates/prompts/compile_variants.tera`
- [ ] `compile_variants` function with guardrail validation per variant
- [ ] DB migration: `bullet_variants` table
- [ ] Integration test: compile variants for `profile.yaml`, verify all pass guardrails
- [ ] Variant selection in `tailor_local`: pick highest-scoring variant per bullet

### Phase 3: Cover Letter Skeletons (Strategy 3)
**Goal:** Pre-compute cover letter templates with slot-filling.

**New module: `careerai-tailor::cover_skeleton`**

- `pub struct CoverSkeleton { domain: String, body_template: String }`
- `pub fn compile_skeletons(profile: &Profile, llm: &dyn Llm, cfg: &LlmConfig) -> Result<Vec<CoverSkeleton>>`
- LLM prompt: "Generate 3-5 cover letter skeletons for different domains (embedded, backend, ML, etc.). Use {{company}}, {{title}}, {{keyword1}}, {{keyword2}}, {{keyword3}} as slot markers."
- Slot extraction: `fn extract_slots(listing: &Listing) -> CoverSlots` — pull company, title, top 3 keywords via tokenizer
- Slot filling: `fn fill_skeleton(skeleton: &CoverSkeleton, slots: &CoverSlots) -> String`
- Confidence scoring: `fn skeleton_confidence(jd: &str, skeleton: &CoverSkeleton) -> f32`
- Fallback: if best confidence < threshold, call LLM directly (current path)

## Review Comments:

Also maintain a database of jd's and their coverletters and resumes as welll.
This will help us in creating better penetrating resumes or different trends
which can be of value.
Also think of any such ideas which can generate income, please invest extra on
those ideas.

**Deliverables:**
- [ ] `CoverSkeleton` / `CoverSlots` types
- [ ] LLM prompt template: `templates/prompts/compile_skeletons.tera`
- [ ] `compile_skeletons` function
- [ ] `extract_slots` + `fill_skeleton` functions
- [ ] Confidence scoring + fallback threshold
- [ ] DB migration: `cover_skeletons` table
- [ ] Integration test: compile skeletons, fill for sample JD, verify ≤350 words

### Phase 4: JD Clustering (Strategy 4)
**Goal:** Group similar JDs to avoid redundant tailoring.

**New module: `careerai-match::cluster`**

- `pub fn cluster_jds(listings: &[Listing], threshold: f32) -> Vec<Cluster>`
- Uses embedding cosine similarity (or Jaccard as fallback)
- Each cluster has a representative (highest-scoring or centroid)
- Tailoring runs only on representatives; non-representatives reuse the diff

**Deliverables:**
- [ ] `cluster_jds` function
- [ ] Cluster representative selection
- [ ] Diff reuse: store diff per cluster, apply to all members
- [ ] Config: `cluster.threshold` (default 0.85)
- [ ] Integration test: cluster 20 sample JDs, verify similar JDs group together

### Phase 5: Hybrid Pipeline Orchestration
**Goal:** Wire everything together with a strategy selector.

**New config in `careerai-core::config`:**
```yaml
tailor:
  strategy: "hybrid"  # "llm" | "local" | "hybrid"
  drop_threshold: 0.05
  variant_count: 3
  skeleton_count: 5
  skeleton_confidence_threshold: 0.6
cluster:
  enabled: true
  threshold: 0.85
```

**Pipeline flow (hybrid mode):**
1. If no compiled variants/skeletons exist for this profile → run Phase 0 compile (1-time)
2. Score + shortlist JDs (existing)
3. Cluster shortlisted JDs
4. For each cluster representative: `tailor_local` + LLM summary reword (optional)
5. For non-representatives: reuse cluster diff + re-score variants locally
6. Cover letter: fill skeleton (or LLM fallback)
7. Render (existing)

**Deliverables:**
- [ ] Config schema for strategy/thresholds
- [ ] Pipeline integration in `careerai-pipeline`
- [ ] CLI: `careerai tailor --strategy local <listing-id>`
- [ ] CLI: `careerai profile compile-variants` (triggers Phase 0)
- [ ] End-to-end integration test: 10 JDs through hybrid pipeline, verify <5 LLM calls
- [ ] Benchmark: 100 JDs through hybrid pipeline, measure LLM calls + wall time

### Phase 6: Batched LLM Calls (Strategy 5, optional optimization)
**Goal:** Reduce per-call overhead for edge-case LLM calls.

When the hybrid pipeline does need to call the LLM (summary reword, cover letter fallback), batch multiple JDs into a single call.

- New prompt: "Reword the summary for each of the following N job descriptions. Return a JSON array of N strings."
- Batch size: 5-10 JDs per call
- Fallback: if batch parse fails, retry per-JD

**Deliverables:**
- [ ] Batched summary reword prompt + parser
- [ ] Batched cover letter fallback prompt + parser
- [ ] Config: `llm.batch_size` (default 5)
- [ ] Integration test: batch-reword 5 summaries, verify all parse

## Testing Strategy

Each phase has:
- **Unit tests** — pure functions, no I/O
- **Integration tests** — use `MockLlm` or real fixtures
- **Guardrail regression tests** — variants/skeletons must pass `forbid_invented_entities`
- **Benchmark tests** — `criterion` benchmarks for scoring/clustering throughput

End-to-end test: run the full pipeline on 10 sample JDs with `strategy=local` and verify zero LLM calls. Run with `strategy=hybrid` and verify ≤3 LLM calls.

## Risk Mitigation

| Risk | Mitigation |
|---|---|
| Local tailoring produces worse reordering than LLM | A/B test: run both paths, compare bullet order quality manually. Keep LLM as fallback. |
| Bullet variants don't cover all JD domains | Increase `variant_count`, add "generic" variant that's always safe. |
| Cover letter skeletons sound templated | LLM polishes slot-filled output (1 quick call) instead of drafting from scratch. |
| Clustering groups dissimilar JDs | Conservative threshold (0.85), borderline JDs get their own cluster. |
| Batched LLM calls fail to parse | Retry per-JD on batch failure. Batch is optimization, not critical path. |

## Migration Path

The hybrid strategy is **additive** — the existing `tailor_for_listing` (LLM path) stays as the fallback. New code lives in new modules/crates. Config flag `tailor.strategy` controls which path runs. No breaking changes to existing CLI or DB schema (new tables are additive).

```
Phase 0-1: Local tailor works, can be tested independently
Phase 2-3: Variants/skeletons compile, stored in DB
Phase 4: Clustering works, can be tested independently
Phase 5: Hybrid pipeline wires it all together
Phase 6: Batch optimization (optional, nice-to-have)
```

## Success Metrics

- **LLM calls per 100 JDs:** 200 → <15 (93%+ reduction)
- **Wall time per 100 JDs:** ~30min → <2min (estimate, depends on embedding model)
- **Cost per 100 JDs:** ~$5 → ~$0.50
- **Tailoring quality:** A/B comparison shows local ≥ LLM for reorder/drop; LLM still wins for reword (but variants close the gap)
- **Cover letter quality:** Skeleton-fill ≥ LLM for 80%+ of JDs (measured by manual review)
