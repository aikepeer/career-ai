//! `careerai tailor` exit-code mapping. The dispatch itself is a
//! small block in `main()` because the success path prints two
//! lines of "next-step" guidance that benefit from staying near the
//! command-table for discoverability.

/// Map a tailor-path error into a stable process exit code.
///
/// - 2: listing / application not found
/// - 3: listing in unexpected state
/// - 4: constrained-diff validator rejected the LLM output
/// - 5: LLM provider/upstream failure
/// - 1: anything else
pub(crate) fn map_tailor_error_to_exit_code(err: &anyhow::Error) -> i32 {
    // Walk the chain looking for typed causes.
    for cause in err.chain() {
        if let Some(te) = cause.downcast_ref::<careerai_tailor::TailorError>() {
            return match te {
                careerai_tailor::TailorError::InventedContent { .. }
                | careerai_tailor::TailorError::Schema(_)
                | careerai_tailor::TailorError::BadPath(_)
                | careerai_tailor::TailorError::CoverLetterTooLong { .. } => 4,
                careerai_tailor::TailorError::Llm(_) => 5,
                _ => 1,
            };
        }
        if cause.downcast_ref::<careerai_llm::LlmError>().is_some() {
            return 5;
        }
    }

    // String-level fallbacks for the `anyhow::bail!` paths that never carry a
    // typed cause — keep these in sync with the messages in `pipeline.rs`.
    let msg = err.to_string();
    if msg.starts_with("listing not found:") || msg.starts_with("application not found:") {
        return 2;
    }
    if msg.contains("expected 'shortlisted'") {
        return 3;
    }
    1
}
