# careerai-match

Filter rules + scoring for discovered listings. Decides which
listings move from `discovered` → `shortlisted`.

## Boundary

| Owns | Never does |
|---|---|
| `FilterRules`, `classify(rules, listing) -> Decision` | LLM calls (this is the *non-LLM* match path) |
| `flatten_profile(profile)` — token-bag for the scorer | DB queries |
| `Scorer` trait + `JaccardScorer` impl | Network |
| `rank_all`, `split_at_threshold`, `score_histogram` | Cron scheduling |

## Two-stage match

1. **Filter** (`classify`) — hard rules: location, must-include
   skills, work-auth eligibility. Returns `Decision::Keep` or
   `Decision::Reject(reason)`. Rejections become `FilteredOut`.
2. **Score** (`Scorer::score`) — `JaccardScorer` computes token
   Jaccard between flattened profile and listing description.
   Listings above `score_threshold` move to `shortlisted`.

`match --tune` skips the persist step and prints a
`score_histogram` so the operator can pick a sensible threshold.

## Configuration

```yaml
match:
  score_threshold: 0.003       # below = filtered_out (Jaccard floor ~0.003)
  notify_threshold: 0.05       # >= notify_threshold fires HighScoreMatch
  must_include_skills: []      # AND-ed with the listing description
  embedding_model: "BAAI/bge-small-en-v1.5"  # reserved for BGE swap
```

`embedding_model` anticipates a BGE-cosine scorer to replace
`JaccardScorer`. The trait surface is unchanged for that swap.

## Tests

```bash
cargo test -p careerai-match
```

Unit tests cover every filter rule's accept/reject case and the
Jaccard score math. `tests/threshold_calibration_it.rs` asserts
`notify_threshold > score_threshold` in the embedded defaults
template — a regression guard for the off-by-decimal bug fixed
in PR #34.
