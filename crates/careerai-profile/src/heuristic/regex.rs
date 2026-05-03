use std::sync::OnceLock;

use regex::Regex;

const EMAIL_RE: &str = r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}";
const PHONE_RE: &str = r"\+?[0-9][0-9\s\-()]{7,}[0-9]";
const GITHUB_RE: &str = r"https?://(?:www\.)?github\.com/[A-Za-z0-9_.-]+";
const LINKEDIN_RE: &str = r"https?://(?:www\.)?linkedin\.com/in/[A-Za-z0-9_.-]+/?";

fn compiled(re: &str) -> &'static Regex {
    #[allow(clippy::unwrap_used)]
    let boxed: Box<Regex> = Box::new(Regex::new(re).unwrap());
    Box::leak(boxed)
}

pub(super) fn email_re() -> &'static Regex {
    static R: OnceLock<&'static Regex> = OnceLock::new();
    R.get_or_init(|| compiled(EMAIL_RE))
}
pub(super) fn phone_re() -> &'static Regex {
    static R: OnceLock<&'static Regex> = OnceLock::new();
    R.get_or_init(|| compiled(PHONE_RE))
}
pub(super) fn github_re() -> &'static Regex {
    static R: OnceLock<&'static Regex> = OnceLock::new();
    R.get_or_init(|| compiled(GITHUB_RE))
}
pub(super) fn linkedin_re() -> &'static Regex {
    static R: OnceLock<&'static Regex> = OnceLock::new();
    R.get_or_init(|| compiled(LINKEDIN_RE))
}
