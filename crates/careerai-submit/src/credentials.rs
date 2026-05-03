//! OS-keychain-backed credential store for browser submitters.
//!
//! Primary path: `keyring` crate. Fallback: env vars prefixed
//! `CAREERAI_<SOURCE>_<NAME>`. Secrets are never logged.
//!
//! Split into per-concern submodules to stay under the 300-LOC cap.

mod cookie;
mod credential;
#[cfg(test)]
mod tests;

pub use cookie::{cookie_expiry, cookie_health, cookie_remaining, parse_jwt_exp, CookieHealth};
pub use credential::{delete, load, store, Credential, SERVICE};
