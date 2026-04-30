# MCP tool manifests

`careerai-mcp` exposes 8 tools over stdio JSON-RPC for any MCP-aware
client (Claude Code, the career-ai plugin, custom orchestrators).
This file documents each tool's input schema, output schema, and
error semantics.

Source of truth: [`crates/careerai-mcp/src/schema.rs`](../crates/careerai-mcp/src/schema.rs).
Each tool's `Args` and `Result` structs derive `schemars::JsonSchema`
so the same shapes you see below are advertised to the LLM at
runtime via `tools/list`.

## Tool index

| Tool | Read/write | Required args | Default safety |
|---|---|---|---|
| `careerai_profile_status` | read | (none) | — |
| `careerai_discover` | write (DB) | (none) | runs every enabled source |
| `careerai_shortlist` | read | (none) | limit defaults to 50 |
| `careerai_tailor` | write (DB) + LLM call | `listing_id` | constrained-diff guardrails apply |
| `careerai_render` | write (filesystem) | `application_id` | requires pandoc on PATH |
| `careerai_apply` | write (DB), optionally network | `application_id` | **dry-run by default** |
| `careerai_inspect` | read | `application_id` | — |
| `careerai_digest` | read | `since` | — |

All write paths go through `careerai-pipeline` and inherit its
safety invariants — see [`ARCHITECTURE.md`](./ARCHITECTURE.md).

---

## careerai_profile_status

Lightweight introspection — does `profile/profile.yaml` exist,
parse, and validate?

**Input**

```json
{}
```

**Output**

```json
{
  "path": "/home/you/career-ai-data/profile/profile.yaml",
  "exists": true,
  "valid": true,
  "last_modified": "2026-04-29T18:42:11Z",
  "issues": []
}
```

`last_modified` is RFC 3339 UTC from the file's `mtime`. Populated
whenever the metadata is readable, even when YAML parse / validate
fails. `issues` is empty when `valid: true`.

**Errors:** can return MCP errors for filesystem failures —
`ProfileIo` (metadata unreadable, e.g. parent directory permission
denied) and `ProfileMissing` (file metadata says it exists but the
subsequent read failed). Truly absent or schema-invalid files
surface in-band as `exists: false` / `valid: false` rather than as
errors. Clients should still wrap the call in error handling and
treat `ProfileIo` / `ProfileMissing` distinctly from a clean
"profile not yet imported" response.

---

## careerai_discover

Pull new listings from configured sources. Idempotent — duplicates
are detected by `(source, external_id)`.

**Input**

```json
{
  "sources": ["greenhouse", "lever"]
}
```

`sources` is optional. Empty / omitted → run every source enabled
in `config/local.yaml`.

**Output**

```json
{
  "fetched": 42,
  "new_rows": 12,
  "duplicates": 30,
  "errors": 0
}
```

**Errors:** transport failures inside any source are logged + counted in `errors`; the call still succeeds with the partial result. Hard errors (config malformed, DB unreachable) surface as MCP errors.

---

## careerai_shortlist

Return shortlisted listings, score-desc.

**Input**

```json
{
  "limit": 50,
  "min_score": 0.05
}
```

`limit` defaults to 50, capped at 500. `min_score` is optional;
omitted means "use the configured `match.score_threshold`".

**Output**

```json
{
  "entries": [
    {
      "listing_id": "0193c2f1-e0a2-7000-...",
      "title": "Senior ML Engineer",
      "company": "Acme Robotics",
      "url": "https://boards.greenhouse.io/acme/jobs/12345",
      "source": "greenhouse",
      "score": 0.087
    }
  ]
}
```

**Errors:** DB unreachable.

---

## careerai_tailor

LLM-tailor the master resume to a shortlisted listing. Persists a
new `applications` row + `application_payloads` entry. Subject to
`careerai-tailor`'s constrained-diff guardrails.

**Input**

```json
{
  "listing_id": "0193c2f1-e0a2-7000-..."
}
```

**Output**

```json
{
  "application_id": "0193c2f1-...",
  "diff_summary": "Reworded 4 bullets, reordered experience, emphasized 3 projects."
}
```

**Errors:** `listing_id` not found, listing not in `shortlisted` state, LLM provider unavailable, `validate_diff` rejected the LLM output (constrained-grammar violation), or `forbid_invented_entities` rejected a reword (invented proper noun / number / year / employer).

---

## careerai_render

Emit DOCX + PDF artifacts for a tailored application via pandoc.
Writes under `data/artifacts/<application_id>/`.

**Input**

```json
{
  "application_id": "0193c2f1-..."
}
```

**Output**

```json
{
  "application_id": "0193c2f1-...",
  "docx_path": "/home/you/.../resume.docx",
  "pdf_path": "/home/you/.../resume.pdf",
  "cover_docx_path": "/home/you/.../cover.docx"
}
```

**Errors:** `application_id` not found, application not in `tailored` state, pandoc not on PATH, PDF engine (default `weasyprint`) not on PATH.

---

## careerai_apply

Submit a prepared application. **Defaults to dry-run.** Live
submission requires explicit opt-in AND a confirm token AND
the per-source `submit_enabled` gate in config.

**Input**

```json
{
  "application_id": "0193c2f1-...",
  "dry_run": true,
  "confirm": null
}
```

To enable real submission you must pass:

```json
{
  "application_id": "0193c2f1-...",
  "dry_run": false,
  "confirm": "I_UNDERSTAND_TOS_RISK"
}
```

The `confirm` token must equal the literal string
`"I_UNDERSTAND_TOS_RISK"`. The check is **runtime** in
`careerai_apply`'s server handler — `ApplyArgs.confirm` is
declared as an unconstrained `Option<String>` in the JSON Schema,
so client-side schema validation will NOT catch a wrong token.
Per-source `submit_enabled` in config still applies even with a
valid confirm token.

**Output**

```json
{
  "application_id": "0193c2f1-...",
  "source": "greenhouse",
  "outcome": "DryRun",
  "would_submit": "POST https://boards-api.greenhouse.io/v1/boards/.../applications  payload_size=12.4KB",
  "note": null
}
```

`outcome` is one of `Submitted` / `DryRun` / `Drafted` / `Skipped`. `would_submit` is filled only when `outcome == "DryRun"`. `note` carries the dry-run summary, the `Drafted` reason, the `Skipped` reason, or the remote id on a successful live submission.

**Errors:** application not found, application not in `rendered` state, source disabled, ATS upstream HTTP failure, unknown source, rate-limit exhausted.

---

## careerai_inspect

Show an application's row, state history, and rendered artifacts.

**Input**

```json
{
  "application_id": "0193c2f1-..."
}
```

**Output**

```json
{
  "application_id": "0193c2f1-...",
  "listing_title": "Senior ML Engineer",
  "listing_company": "Acme Robotics",
  "listing_source": "greenhouse",
  "state": "submitted",
  "events": [
    {"from_state": null, "to_state": "discovered", "note": null,            "created_at": "2026-04-29T10:00:00Z"},
    {"from_state": "discovered", "to_state": "shortlisted", "note": "score=0.087", "created_at": "2026-04-29T10:01:12Z"},
    {"from_state": "shortlisted", "to_state": "tailored", "note": null,    "created_at": "2026-04-29T10:02:48Z"},
    {"from_state": "tailored",    "to_state": "rendered", "note": null,    "created_at": "2026-04-29T10:03:11Z"},
    {"from_state": "rendered",    "to_state": "submitted", "note": "remote=app_abc123", "created_at": "2026-04-29T10:04:02Z"}
  ],
  "artifacts": [
    {"kind": "resume_docx", "path": "/.../resume.docx", "bytes": 24576},
    {"kind": "resume_pdf",  "path": "/.../resume.pdf",  "bytes": 51200},
    {"kind": "cover_docx",  "path": "/.../cover.docx",  "bytes": 18432}
  ]
}
```

**Errors:** application not found.

---

## careerai_digest

Pre-rendered markdown summary of pipeline activity over a time
window. Designed for the LLM to surface in chat.

**Input**

```json
{
  "since": "1d"
}
```

`since` accepts `<n>h` / `<n>d` / `<n>w` (e.g. `"24h"`, `"2w"`)
or a bare integer interpreted as hours.

**Output**

```json
{
  "markdown": "career-ai digest — last 24h ...",
  "since_iso": "2026-04-28T18:42:11Z",
  "discovered": 42,
  "matched": 23,
  "shortlisted": 11,
  "drafted": 0,
  "submitted": 1,
  "failed": 0,
  "responded": 0,
  "per_source": {"greenhouse": 25, "lever": 17},
  "last_tick": "2026-04-29T18:42:00Z"
}
```

`markdown` is the same body the CLI's `careerai digest` prints,
with terminal codes stripped. The structured fields below it let
the LLM render its own custom summary if preferred.

**Errors:** invalid `since` format, DB unreachable.

---

## Resources

The MCP server also exposes three resources (read-only):

| URI | Description |
|---|---|
| `careerai://profile` | Current `profile/profile.yaml` body |
| `careerai://shortlist/{date}` | **Currently returns the present-day shortlist regardless of `{date}`.** The path segment is parsed but ignored by `read_shortlist_resource`. Future work will honor it; until then, do not trust the URI as a historical slice — use it only as the live shortlist endpoint. |
| `careerai://artifacts/{application_id}` | Index of artifacts attached to the application |

Resource bodies use a compact projection (`CompactListing` for
shortlists) instead of the raw DB row to fit comfortably inside
LLM context windows. The free-form `description` (HTML/text JD body)
and `raw_json` (full ATS payload) fields are not included.

## Behavior contracts that don't change between versions

* `careerai_apply` defaults to dry-run forever. Removing this default
  would be a breaking change.
* `careerai_apply.confirm` literal value (`"I_UNDERSTAND_TOS_RISK"`) is
  stable. If we ever need to gate live submission behind a different
  acknowledgement, it'll be a new field, not a renamed one.
* Field names are snake_case across all schemas.
* `Option<…>` fields are `null`-when-absent. Schemas use
  `#[serde(skip_serializing_if = "Option::is_none")]` so they're
  also absent in serialized output when None.
