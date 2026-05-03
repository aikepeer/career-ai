//! OS-keychain-backed credential store for browser submitters.
//!
//! Primary path: `keyring` crate. Fallback: env vars prefixed
//! `CAREERAI_<SOURCE>_<NAME>`. Secrets are never logged.
//!
//! Split into per-concern submodules to stay under the 300-LOC cap.

mod credential;
mod cookie;
#[cfg(test)]
mod tests;

pub use credential::{delete, load, store, Credential, SERVICE};
pub use cookie::{
    cookie_expiry, cookie_health, cookie_remaining, parse_jwt_exp, CookieHealth,
};
