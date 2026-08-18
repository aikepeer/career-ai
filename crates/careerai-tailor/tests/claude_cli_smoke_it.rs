//! End-to-end smoke for `tailor_for_listing` driven by the `claude` CLI
//! subprocess backend (introduced in PR #21).
//!
//! The test:
//!   1. Writes a stub `claude` binary into a tempdir. The stub captures
//!      its argv to a sentinel file and emits a canned `claude --print`
//!      JSON payload whose `result` is a valid constrained-diff
//!      response that `careerai-tailor::schema::parse_and_validate`
//!      accepts.
//!   2. Points `CAREERAI_CLAUDE_BIN` at the stub and sets
//!      `CAREERAI_SKIP_CLI_PROBE=1` so backend resolution doesn't try
//!      to spawn a real authenticated `claude` ping.
//!   3. Builds a `LlmConfig` with `tailor_model =
//!      "anthropic/claude-sonnet-4-6"` and resolves
//!      `Backend::ClaudeCli(...)` via `Backend::resolve(
//!      BackendChoice::ClaudeCli, ...)`. The backend strips the
//!      `anthropic/` prefix; the stub's captured argv is asserted to
//!      contain just `claude-sonnet-4-6`.
//!   4. Runs `tailor_for_listing(...)` against a fixture `Profile` and
//!      seeded listing.
//!   5. Asserts the parsed `TailoredApplication` (`TailorOutcome`):
//!      a non-empty `application_id`, a `ResumeView` with the same
//!      number of experience entries as the input profile (the diff
//!      can only reorder/reword, never invent), and a non-empty cover
//!      letter body.
//!
//! Notes:
//! - The test is gated behind `tailor-claude-cli-smoke` so this crate
//!   continues to build on `main` before PR #21 lands. After PR #21,
//!   wire `tailor-claude-cli-smoke = ["careerai-llm/live-llm-cli"]`
//!   in `Cargo.toml` to flip the test on.
//! - `tailor_one` in `careerai-pipeline` is gated on `CAREERAI_LLM_LIVE=1`
//!   per PR #21 fix #1; that env-var gate lives at the pipeline layer.
//!   This test constructs a `Backend` directly and bypasses that gate.
//! - The `MockLlm`-based tests in `tests/tailor_roundtrip.rs` and
//!   `tests/safety_adversarial.rs` are unaffected by this file.

#![cfg(feature = "tailor-claude-cli-smoke")]

mod common;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::common::fixture_profile;
    use std::sync::Arc;

    use careerai_core::config::{BackendChoice, LlmConfig};
    use careerai_db::models::NewListing;
    use careerai_db::pool::pool_in_memory;
    use careerai_db::queries;
    use careerai_llm::backend::Backend;
    use careerai_llm::cache::Cache;
    use careerai_tailor::tailor_for_listing;

    /// A constrained-diff response shaped like a real claude response.
    /// Mirrors `tests/tailor_roundtrip.rs::CANNED_TAILOR_JSON`.
    const TAILOR_DIFF: &str = r#"{
        "prompt_version":"tailor.v1",
        "summary":{"op":"keep"},
        "ops":[
            {"path":"experience[0].bullets[1]","op":"move_before","target_path":"experience[0].bullets[0]"},
            {"path":"experience[0].bullets[0]","op":"reword","new_text":"Shipped Rust LLM pipeline cutting latency 35%."},
            {"path":"experience[1].bullets[0]","op":"keep"},
            {"path":"projects[0].bullets[0]","op":"keep"}
        ],
        "cover_letter":"I build things."
    }"#;

    const COVER_LETTER: &str = "Dear Hiring Team,\n\nI am applying.\n\nRegards.";

    /// Build a stub `claude` binary in `dir` that:
    /// - writes its argv (one arg per line) to `argv_sentinel`,
    /// - emits a canned `claude --print --output-format json` envelope
    ///   on stdout,
    /// - exits 0.
    ///
    /// The stub flips between two payloads based on a unique marker in
    /// the user prompt that arrives on stdin: any prompt containing
    /// the literal phrase `Draft a cover letter` (emitted by the
    /// cover-letter Tera template) returns the cover letter; otherwise
    /// the diff. Matching `cover_letter` would also match the tailor
    /// prompt's JSON schema (which lists a `cover_letter` field), so
    /// the discriminator must be a phrase that appears only in the
    /// cover-letter prompt body.
    ///
    /// Hermeticity: the JSON-escaped `result` string is generated in
    /// Rust via `serde_json::to_string` and embedded as a shell
    /// literal. The script is POSIX `/bin/sh` only — no `base64`, no
    /// `python3`, no other external executables.
    fn write_stub_binary(stub_path: &std::path::Path, argv_sentinel: &std::path::Path) {
        let diff_json = serde_json::to_string(TAILOR_DIFF).unwrap();
        let cover_json = serde_json::to_string(COVER_LETTER).unwrap();
        // The script reads stdin into $stdin, picks one of the two
        // pre-escaped JSON strings, and emits the wrapped envelope.
        let script = format!(
            r#"#!/bin/sh
# Capture argv (one per line) for the assertion.
: > '{sentinel}'
for a in "$@"; do printf '%s\n' "$a" >> '{sentinel}'; done

stdin=$(cat || true)

# Decide which canned response to return. Match only a phrase that is
# unique to the cover-letter prompt text, not the `cover_letter`
# schema field name that also appears in the tailor prompt.
case "$stdin" in
  *"Draft a cover letter"*) result_json='{cover_json}' ;;
  *) result_json='{diff_json}' ;;
esac

# Emit the claude --print --output-format json envelope. `result_json`
# is already a JSON-escaped string literal (produced by serde_json
# in Rust), so the stub stays POSIX-only.
cat <<__P__
{{"type":"result","is_error":false,"result":${{result_json}},"usage":{{"input_tokens":7,"output_tokens":11,"cache_read_input_tokens":0}}}}
__P__
"#,
            sentinel = argv_sentinel.display(),
            diff_json = diff_json,
            cover_json = cover_json,
        );
        std::fs::write(stub_path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(stub_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(stub_path, perms).unwrap();
        }
    }

    fn fixture_cfg(cache_dir: String) -> LlmConfig {
        LlmConfig {
            provider: String::new(),
            model: String::new(),
            // The whole point of this assertion: `anthropic/` must be
            // stripped before reaching the stub binary's argv.
            tailor_model: "anthropic/claude-sonnet-4-6".into(),
            cover_letter_model: "anthropic/claude-sonnet-4-6".into(),
            filter_model: String::new(),
            parse_resume_model: String::new(),
            cache_dir,
            api_base_url: None,
            api_key: None,
            max_retries: 1,
            timeout_seconds: 30,
            prompt_version: "tailor.v1".into(),
            // The claude CLI backend logs that it ignores this; setting
            // it true keeps the LlmRequest shape identical to the live
            // path so the cache key composition is exercised.
            anthropic_prompt_cache: true,
            // Field added by PR #21. On main the struct is missing this
            // field; see the gate at the top of this file.
            backend: BackendChoice::ClaudeCli,
        }
    }

    #[tokio::test]
    async fn tailor_runs_against_stub_claude_cli_and_strips_anthropic_prefix() {
        // 1. Spin up the stub binary.
        let dir = tempfile::tempdir().unwrap();
        let stub_path = dir.path().join("claude");
        let argv_sentinel = dir.path().join("argv.txt");
        write_stub_binary(&stub_path, &argv_sentinel);

        // 2. Point backend resolution at the stub and skip the live ping.
        std::env::set_var("CAREERAI_CLAUDE_BIN", &stub_path);
        std::env::set_var("CAREERAI_SKIP_CLI_PROBE", "1");

        // 3. Resolve the backend explicitly to ClaudeCli. This is where
        //    `pick_default_model` strips the `anthropic/` prefix.
        let cache_dir = dir.path().join("cache");
        let cfg = fixture_cfg(cache_dir.to_string_lossy().into_owned());
        let cache = Arc::new(Cache::new(&cache_dir));
        let backend = Backend::resolve(BackendChoice::ClaudeCli, &cfg, cache)
            .await
            .expect("resolve claude-cli backend");

        // 4. Seed an in-memory SQLite pool with a Shortlisted listing.
        let pool = pool_in_memory().await.unwrap();
        let new = NewListing {
            source: "fixture".into(),
            external_id: "cc-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Beta Corp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/cc-1".into(),
            description: "Build and ship a real-time LLM system.".into(),
            raw_json: None,
        };
        let (listing_id, _) = queries::insert_or_ignore(&pool, &new).await.unwrap();
        queries::transition(
            &pool,
            &listing_id,
            careerai_core::state::ListingState::Shortlisted,
            None,
        )
        .await
        .unwrap();

        // 5. Drive `tailor_for_listing` against the resolved Backend.
        //    `Backend` implements `Llm`, so it satisfies the trait
        //    bound on the function.
        let profile = fixture_profile();
        let outcome = tailor_for_listing(&pool, &backend, &listing_id, &profile, &cfg)
            .await
            .expect("tailor against stub claude-cli");

        // --- Assertions on the parsed application ---
        assert!(
            !outcome.application_id.is_empty(),
            "application_id should be populated"
        );
        // The diff cannot invent experience entries; the count must
        // match the input profile.
        assert_eq!(
            outcome.resume_view.experience.len(),
            profile.experience.len(),
            "constrained diff must not invent or drop experience entries"
        );
        // The cover letter draft prompt is a separate hop; the stub
        // returns the cover-letter body for prompts containing the
        // marker.
        assert!(
            !outcome.cover_letter.body.is_empty(),
            "cover letter body should be populated"
        );

        // --- argv assertion: the stripped model name reached the binary ---
        let argv = std::fs::read_to_string(&argv_sentinel)
            .expect("stub binary must have written its argv");
        let argv_lines: Vec<&str> = argv.lines().collect();
        assert!(
            argv_lines.iter().any(|a| *a == "--model"),
            "argv missing --model flag: {argv}"
        );
        assert!(
            argv_lines.iter().any(|a| *a == "claude-sonnet-4-6"),
            "argv must carry the stripped model name `claude-sonnet-4-6`, got: {argv}"
        );
        // Defense in depth: the namespaced form must NEVER reach the
        // subprocess argv.
        assert!(
            !argv_lines.iter().any(|a| a.contains("anthropic/")),
            "argv must NOT contain `anthropic/` prefix; got: {argv}"
        );
    }
}
