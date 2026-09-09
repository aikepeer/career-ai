//! Email validation: format check → role-address blocklist → DNS MX
//! lookup, with an in-memory per-domain cache to avoid repeated DNS
//! round-trips.
//!
//! DNS is abstracted behind [`DnsResolver`] so the validator can be
//! exercised offline with a mock; the production impl is
//! [`HickoryDnsResolver`] (backed by `hickory-resolver`).

use std::collections::HashMap;

use async_trait::async_trait;
use regex::Regex;
use tokio::sync::Mutex;

use hickory_resolver::TokioResolver;

/// Local parts that are generic role/inbox addresses, never a real
/// recruiter contact. An email whose local part matches one of these
/// (case-insensitive) is rejected before any DNS work.
const BLOCKED_LOCAL_PARTS: &[&str] = &[
    "noreply",
    "no-reply",
    "donotreply",
    "support",
    "sales",
    "info",
    "press",
    "privacy",
];

/// Abstraction over DNS so the validator can be tested without touching
/// the network. The production implementation is [`HickoryDnsResolver`].
#[async_trait]
pub trait DnsResolver: Send + Sync {
    /// MX exchange hostnames for `domain`. An empty `Vec` means the domain
    /// resolves but publishes no MX record.
    async fn lookup_mx(&self, domain: &str) -> Result<Vec<String>, String>;
    /// A/AAAA addresses for `host`. An empty `Vec` means the host does not
    /// resolve.
    async fn lookup_ip(&self, host: &str) -> Result<Vec<std::net::IpAddr>, String>;
}

/// Production DNS resolver backed by `hickory-resolver`, configured from
/// the OS system configuration (`/etc/resolv.conf` on Unix).
#[derive(Debug)]
pub struct HickoryDnsResolver {
    resolver: TokioResolver,
}

impl HickoryDnsResolver {
    /// Build a resolver from the system configuration.
    ///
    /// Returns an error if the system resolver configuration cannot be
    /// read (e.g. no `resolv.conf` in a minimal container).
    pub fn new() -> std::result::Result<Self, String> {
        let resolver = TokioResolver::builder_tokio()
            .map_err(|e| e.to_string())?
            .build();
        Ok(Self { resolver })
    }
}

#[async_trait]
impl DnsResolver for HickoryDnsResolver {
    async fn lookup_mx(&self, domain: &str) -> Result<Vec<String>, String> {
        match self.resolver.mx_lookup(domain).await {
            // `MxLookup::iter` yields `&MX` directly, so each item already
            // exposes its exchange host.
            Ok(lookup) => Ok(lookup.iter().map(|mx| mx.exchange().to_string()).collect()),
            Err(e) if e.is_no_records_found() => Ok(Vec::new()),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn lookup_ip(&self, host: &str) -> Result<Vec<std::net::IpAddr>, String> {
        match self.resolver.lookup_ip(host).await {
            Ok(lookup) => Ok(lookup.iter().collect()),
            Err(e) if e.is_no_records_found() => Ok(Vec::new()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Why an email failed validation. Format and blocklist failures are
/// permanent; `NoMxRecord` is permanent for that domain; `DnsError` is
/// transient (network) and is deliberately not cached.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EmailValidationError {
    #[error("invalid email format: {0}")]
    InvalidFormat(String),
    #[error("email uses a blocked role address: {0}")]
    BlockedDomain(String),
    #[error("no MX record found for domain: {0}")]
    NoMxRecord(String),
    #[error("dns lookup failed for {domain}: {detail}")]
    DnsError { domain: String, detail: String },
}

/// Validates an email address in three stages — regex format, a
/// role-address blocklist, then a DNS MX lookup — caching the MX result
/// per domain so repeated addresses at the same domain share one lookup.
#[derive(Debug)]
pub struct EmailValidator<R: DnsResolver> {
    format_re: Regex,
    resolver: R,
    /// domain → has a valid MX record. `false` entries are cached too, so a
    /// known-no-MX domain short-circuits to [`EmailValidationError::NoMxRecord`]
    /// without another DNS round-trip.
    cache: Mutex<HashMap<String, bool>>,
}

impl<R: DnsResolver> EmailValidator<R> {
    /// Construct a validator over the given DNS resolver. Returns an error
    /// only if the format regex fails to compile (effectively never for
    /// the hard-coded pattern).
    pub fn new(resolver: R) -> std::result::Result<Self, String> {
        let format_re = Regex::new(r"^[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}$")
            .map_err(|e| e.to_string())?;
        Ok(Self {
            format_re,
            resolver,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Validate `email`. Returns `Ok(true)` when the address is well-formed,
    /// not a blocked role address, and its domain has at least one MX record.
    pub async fn validate(&self, email: &str) -> Result<bool, EmailValidationError> {
        // Stage 1: format check.
        if !self.format_re.is_match(email) {
            return Err(EmailValidationError::InvalidFormat(email.to_owned()));
        }

        // Split into local@domain for the remaining stages.
        let Some(at) = email.rfind('@') else {
            return Err(EmailValidationError::InvalidFormat(email.to_owned()));
        };
        let local = &email[..at];
        let domain = &email[at + 1..];

        // Stage 2: role-address blocklist (case-insensitive).
        let lower = local.to_ascii_lowercase();
        if BLOCKED_LOCAL_PARTS.contains(&lower.as_str()) {
            return Err(EmailValidationError::BlockedDomain(email.to_owned()));
        }

        // Stage 3: DNS MX lookup with per-domain cache.
        {
            let cache = self.cache.lock().await;
            if let Some(&has_mx) = cache.get(domain) {
                return if has_mx {
                    Ok(true)
                } else {
                    Err(EmailValidationError::NoMxRecord(domain.to_owned()))
                };
            }
        }

        // Not cached — perform the DNS lookup.
        match self.resolver.lookup_mx(domain).await {
            Ok(mx) if !mx.is_empty() => {
                self.cache.lock().await.insert(domain.to_owned(), true);
                Ok(true)
            }
            Ok(_) => {
                self.cache.lock().await.insert(domain.to_owned(), false);
                Err(EmailValidationError::NoMxRecord(domain.to_owned()))
            }
            Err(detail) => Err(EmailValidationError::DnsError {
                domain: domain.to_owned(),
                detail,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::collections::HashMap;
    use std::net::IpAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Mock DNS resolver: returns canned MX/IP results keyed by domain and
    /// counts MX lookups so cache behaviour can be asserted.
    #[derive(Debug, Default, Clone)]
    struct MockResolver {
        mx: HashMap<String, Vec<String>>,
        ips: HashMap<String, Vec<IpAddr>>,
        mx_err: Option<String>,
        mx_lookups: Arc<AtomicUsize>,
    }

    impl MockResolver {
        fn new() -> Self {
            Self::default()
        }
        fn with_mx(mut self, domain: &str, mx: &[&str]) -> Self {
            self.mx.insert(
                domain.to_owned(),
                mx.iter().map(|s| (*s).to_owned()).collect(),
            );
            self
        }
        fn with_mx_err(mut self, err: &str) -> Self {
            self.mx_err = Some(err.to_owned());
            self
        }
    }

    #[async_trait]
    impl DnsResolver for MockResolver {
        async fn lookup_mx(&self, domain: &str) -> Result<Vec<String>, String> {
            self.mx_lookups.fetch_add(1, Ordering::Relaxed);
            if let Some(err) = &self.mx_err {
                return Err(err.clone());
            }
            Ok(self.mx.get(domain).cloned().unwrap_or_default())
        }
        async fn lookup_ip(&self, host: &str) -> Result<Vec<IpAddr>, String> {
            Ok(self.ips.get(host).cloned().unwrap_or_default())
        }
    }

    #[tokio::test]
    async fn valid_email_with_mx_returns_ok_true() {
        let mock = MockResolver::new().with_mx("gmail.com", &["gmail-smtp-in.l.google.com."]);
        let v = EmailValidator::new(mock).unwrap();
        assert_eq!(v.validate("user@gmail.com").await, Ok(true));
    }

    #[tokio::test]
    async fn invalid_format_returns_err() {
        let mock = MockResolver::new();
        let v = EmailValidator::new(mock).unwrap();
        assert_eq!(
            v.validate("not-an-email").await,
            Err(EmailValidationError::InvalidFormat(
                "not-an-email".to_owned()
            ))
        );
    }

    #[tokio::test]
    async fn missing_at_sign_is_invalid_format() {
        let mock = MockResolver::new();
        let v = EmailValidator::new(mock).unwrap();
        assert_eq!(
            v.validate("usergmail.com").await,
            Err(EmailValidationError::InvalidFormat(
                "usergmail.com".to_owned()
            ))
        );
    }

    #[tokio::test]
    async fn blocked_role_addresses_are_rejected() {
        let mock = MockResolver::new().with_mx("acme.com", &["mail.acme.com."]);
        let v = EmailValidator::new(mock).unwrap();
        for local in [
            "noreply",
            "no-reply",
            "donotreply",
            "support",
            "sales",
            "info",
            "press",
            "privacy",
        ] {
            let email = format!("{local}@acme.com");
            assert_eq!(
                v.validate(&email).await,
                Err(EmailValidationError::BlockedDomain(email.clone())),
                "{email} should be blocked"
            );
        }
    }

    #[tokio::test]
    async fn blocked_local_part_is_case_insensitive() {
        let mock = MockResolver::new().with_mx("acme.com", &["mail.acme.com."]);
        let v = EmailValidator::new(mock).unwrap();
        assert_eq!(
            v.validate("Support@acme.com").await,
            Err(EmailValidationError::BlockedDomain(
                "Support@acme.com".to_owned()
            ))
        );
    }

    #[tokio::test]
    async fn domain_with_no_mx_returns_no_mx_record() {
        let mock = MockResolver::new(); // no MX registered for "nope.com"
        let v = EmailValidator::new(mock).unwrap();
        assert_eq!(
            v.validate("user@nope.com").await,
            Err(EmailValidationError::NoMxRecord("nope.com".to_owned()))
        );
    }

    #[tokio::test]
    async fn dns_error_is_returned_not_cached() {
        let mock = MockResolver::new().with_mx_err("timeout");
        let lookups = mock.mx_lookups.clone();
        let v = EmailValidator::new(mock).unwrap();
        assert_eq!(
            v.validate("user@acme.com").await,
            Err(EmailValidationError::DnsError {
                domain: "acme.com".to_owned(),
                detail: "timeout".to_owned(),
            })
        );
        // A transient DNS error must not be cached: a second call still
        // performs a lookup (counter advances to 2).
        assert_eq!(
            v.validate("other@acme.com").await,
            Err(EmailValidationError::DnsError {
                domain: "acme.com".to_owned(),
                detail: "timeout".to_owned(),
            })
        );
        assert_eq!(
            lookups.load(Ordering::Relaxed),
            2,
            "dns errors must not be cached"
        );
    }

    #[tokio::test]
    async fn cache_avoids_repeated_dns_for_same_domain() {
        let mock = MockResolver::new().with_mx("gmail.com", &["gmail-smtp-in.l.google.com."]);
        let lookups = mock.mx_lookups.clone();
        let v = EmailValidator::new(mock).unwrap();
        assert_eq!(v.validate("a@gmail.com").await, Ok(true));
        assert_eq!(v.validate("b@gmail.com").await, Ok(true));
        assert_eq!(
            lookups.load(Ordering::Relaxed),
            1,
            "second email at same domain should hit the cache"
        );
    }

    #[tokio::test]
    async fn cache_stores_no_mx_result() {
        let mock = MockResolver::new(); // no MX for "nope.com"
        let lookups = mock.mx_lookups.clone();
        let v = EmailValidator::new(mock).unwrap();
        assert_eq!(
            v.validate("a@nope.com").await,
            Err(EmailValidationError::NoMxRecord("nope.com".to_owned()))
        );
        assert_eq!(
            v.validate("b@nope.com").await,
            Err(EmailValidationError::NoMxRecord("nope.com".to_owned()))
        );
        assert_eq!(
            lookups.load(Ordering::Relaxed),
            1,
            "second no-MX email at same domain should hit the cache"
        );
    }

    #[tokio::test]
    async fn plus_addressing_is_valid_format() {
        let mock = MockResolver::new().with_mx("acme.com", &["mail.acme.com."]);
        let v = EmailValidator::new(mock).unwrap();
        assert_eq!(v.validate("user+job@acme.com").await, Ok(true));
    }
}
