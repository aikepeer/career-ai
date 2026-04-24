//! Small helpers shared across adapters.

use std::sync::OnceLock;

use regex::Regex;

fn tag_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    #[allow(clippy::unwrap_used)]
    R.get_or_init(|| Regex::new(r"<[^>]+>").unwrap())
}

/// Minimal HTML → plain text. Not a full sanitizer — adapters hand us
/// well-formed marketing HTML, not hostile input. Collapses whitespace so
/// downstream embeddings see clean tokens.
#[must_use]
pub fn html_to_text(html: &str) -> String {
    let stripped = tag_re().replace_all(html, " ");
    let decoded = stripped
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags_and_decodes_common_entities() {
        let html = "<p>Build <strong>LLM</strong> apps &amp; ship them.</p><ul><li>Rust</li></ul>";
        let text = html_to_text(html);
        assert_eq!(text, "Build LLM apps & ship them. Rust");
    }

    #[test]
    fn collapses_whitespace() {
        assert_eq!(html_to_text("  hello\n\n\tworld  "), "hello world");
    }
}
