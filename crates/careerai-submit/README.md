# careerai-submit

Submission gateway. Owns the **dry-run-by-default safety
invariant** and per-source rate limiting.

## Boundary

| Owns | Never does |
|---|---|
| `Submitter` trait + per-source impls (greenhouse_http, lever_http, ashby_http, naukri, linkedin) | Discovery (`careerai-sources` does that) |
| `submit_application(...)` dispatcher | Tailoring (`careerai-tailor` does that) |
| `governor` rate-limit token-buckets per source | Rendering (`careerai-render` does that) |
| Quiet-hours window enforcement | LLM calls |
| Cookie storage via `keyring` (`linkedin`, `naukri`) | Cron scheduling |
| Dry-run wrapper that captures `would_submit` events |  |

## Safety invariants

1. **Dry-run by default.** `auto_submit_override = None` ⇒ dry-run.
   Only `Some(true)` enables network writes. Even then, per-source
   `submit_enabled` gates apply.
2. **Rate-limit at the boundary.** Every submitter must
   `governor::Quota::check()` before any network call.
   Quiet-hours and daily caps are enforced at the boundary, not in
   the submitter body.
3. **Logged events redacted.** `tracing` events go through a
   secret-shape regex; a test in this crate asserts captured
   events contain no cookies / tokens / API keys.
4. **Per-source `submit_enabled = false` default.** LinkedIn and
   Naukri ship off; the operator must explicitly flip them.

## Module layout

```
src/
  lib.rs              SubmitOutcome, SubmitError, dispatcher
  http/               greenhouse, lever, ashby HTTP submitters
  linkedin.rs         CDP browser-automation submitter (feature `browser`)
  naukri.rs           CDP browser-automation submitter (feature `browser`)
  rate_limiter.rs     governor-backed buckets + quiet hours
  credentials.rs      keyring wrapper for cookies + API keys
  ats_http.rs         shared HTTP helpers
  ...
```

## Features

| Feature | Effect |
|---|---|
| `browser` | Build LinkedIn + Naukri CDP submitters via `chromiumoxide` + the checked-in `stealth-v2.js` |

Default build is HTTP-only.

## Tests

```bash
cargo test -p careerai-submit
```

`wiremock` mocks ATS endpoints. Browser submitters are tested
against captured HTML served by `tiny-http`. CI never hits real
LinkedIn / Naukri / ATS endpoints.

## Adding a new submitter

1. Implement `Submitter` for your `XSubmitter` struct.
2. Register it in `lib.rs`'s dispatch on `listing.source`.
3. Add a per-source `governor::Quota` to `rate_limiter.rs`.
4. Add a `wiremock` integration test asserting:
   - dry-run mode emits zero network writes
   - `submit_enabled = false` is honored even with `--auto-submit`
   - rate-limit denial returns `SubmitError::RateLimited` cleanly
