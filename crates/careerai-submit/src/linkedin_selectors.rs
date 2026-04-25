//! LinkedIn DOM selectors as `const &str` constants. Kept in a
//! feature-flag-FREE module so the fixture test in
//! `tests/linkedin_fixture_it.rs` can import the SAME expression the
//! runtime click code uses, even when `--features browser` is off
//! (CI default).
//!
//! Without this central source of truth, a class rename in
//! `linkedin.rs::click_easy_apply` could pass the fixture test while
//! breaking the live browser flow.

/// CSS selector for the LinkedIn Easy Apply call-to-action. Matches
/// the class-named button AND any `<button>` whose `aria-label`
/// contains "Easy Apply" — LinkedIn ships a few CTA variants depending
/// on which A/B bucket the account is in. A class rename should not
/// break us silently.
pub const EASY_APPLY_SELECTOR: &str = "button.jobs-apply-button, button[aria-label*='Easy Apply']";
