//! CSS selectors for Naukri.com Apply flow. Pinned and commented so a
//! selector change at the LIVE site can't pass the fixture test while
//! breaking the live browser flow. Mirrors the discipline established
//! in `linkedin_selectors.rs`.

/// Primary "Apply" CTA on a Naukri job page. Naukri ships several
/// id/class variants depending on logged-in state and A/B bucket, so
/// we accept the union. The aria-label fallback is the most stable
/// anchor and survives most class renames.
pub const APPLY_BUTTON_SELECTOR: &str =
    "button#apply-button, button.apply-button, button[id^='apply-button-'], button[aria-label*='Apply']";

/// Confirmation control inside the apply modal. Naukri sometimes
/// asks the user to confirm before firing the actual POST.
pub const CONFIRM_APPLY_SELECTOR: &str =
    "button.apply-button-text, button[type='submit'][class*='apply'], button.btn-apply";

/// "Already applied" / success indicator. We treat the presence of
/// this element after Apply-click as the success signal.
pub const APPLIED_SUCCESS_SELECTOR: &str =
    "button.applied, button[disabled].apply-button, span.already-applied, div.success-message";

/// Login-required indicator. If we hit this, the session cookie is
/// dead and the operator needs to refresh.
pub const LOGIN_REQUIRED_INDICATOR: &str =
    "div.login-layer, form[name='loginForm'], div.lgnLayer, a[href*='/nlogin/']";
