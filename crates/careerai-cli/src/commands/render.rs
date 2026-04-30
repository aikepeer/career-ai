//! `careerai render` exit-code mapping. The dispatch itself is a
//! small block in `main()` because the success path prints the
//! emitted artifact paths and stays near the command-table.

/// Map a render-path error into a stable process exit code.
///
/// - 2: application / payload / listing not found
/// - 3: application in unexpected state
/// - 6: pandoc missing on PATH
/// - 1: anything else
pub(crate) fn map_render_error_to_exit_code(err: &anyhow::Error) -> i32 {
    for cause in err.chain() {
        if let Some(re) = cause.downcast_ref::<careerai_render::RenderError>() {
            if matches!(re, careerai_render::RenderError::PandocMissing) {
                return 6;
            }
        }
    }

    let msg = err.to_string();
    if msg.starts_with("application not found:")
        || msg.starts_with("application payload not found:")
        || msg.starts_with("listing not found:")
    {
        return 2;
    }
    if msg.contains("expected 'tailored'") {
        return 3;
    }
    1
}
