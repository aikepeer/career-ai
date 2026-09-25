//! CSRF defense-in-depth for the unauthenticated local dashboard.
//!
//! The dashboard has no login and defaults to loopback-only, but browsers
//! treat `http://127.0.0.1:PORT` as a normal origin: a malicious website
//! can submit a cross-origin `<form>` to it. The JSON endpoints are
//! protected by the CORS preflight (we send no CORS headers), but multipart
//! and form posts are "simple" requests that browsers send without a
//! preflight. This middleware rejects cross-origin mutating requests by
//! comparing the `Origin`/`Referer` header against the request `Host`.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::AppState;

pub async fn enforce_same_origin(req: Request, next: Next) -> Response {
    if *req.method() != Method::POST {
        return next.run(req).await;
    }

    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok());
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    let referer = req
        .headers()
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok());

    let rejected = match origin.or(referer) {
        Some(value) => !origin_matches_host(value, host),
        // `curl` and same-origin fetches that omit both headers stay
        // allowed; requiring a token would break the CLI/test callers.
        None => false,
    };

    if rejected {
        return (StatusCode::FORBIDDEN, "cross-origin request rejected\n").into_response();
    }
    next.run(req).await
}

fn origin_matches_host(origin_or_referer: &str, host: Option<&str>) -> bool {
    let Some(host) = host else { return false };
    let Some(authority) = authority_of(origin_or_referer) else {
        return false;
    };
    authority.eq_ignore_ascii_case(host)
}

/// Require a bearer token on every request when one is configured. When
/// `CAREERAI_DASHBOARD_TOKEN` is unset this middleware is a no-op, so the
/// loopback-only + same-origin posture stays unchanged for local use.
pub async fn enforce_auth_token(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(expected) = state.auth_token.as_deref() else {
        return next.run(req).await;
    };

    let provided = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(strip_bearer)
        .or_else(|| {
            req.headers()
                .get("x-careerai-token")
                .and_then(|v| v.to_str().ok())
        });

    if provided != Some(expected) {
        return (StatusCode::UNAUTHORIZED, "invalid or missing token\n").into_response();
    }
    next.run(req).await
}

/// Strip a case-insensitive `Bearer ` prefix from an `Authorization` header
/// value. Returns the remainder unchanged when the prefix is absent so a
/// bare token in the Authorization header is also accepted.
fn strip_bearer(value: &str) -> Option<&str> {
    let rest = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    Some(rest)
}

/// Extract the `host[:port]` authority from an absolute URL string.
/// Returns `None` for relative or malformed values.
fn authority_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    rest.split('/').next()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn authority_extraction() {
        assert_eq!(
            authority_of("http://127.0.0.1:8787/path"),
            Some("127.0.0.1:8787")
        );
        assert_eq!(
            authority_of("https://localhost:8787"),
            Some("localhost:8787")
        );
        assert_eq!(authority_of("relative/path"), None);
        assert_eq!(authority_of("ftp://example.com"), None);
    }

    #[test]
    fn strip_bearer_handles_case_and_missing_prefix() {
        assert_eq!(strip_bearer("Bearer abc"), Some("abc"));
        assert_eq!(strip_bearer("bearer abc"), Some("abc"));
        assert_eq!(strip_bearer("abc"), None);
        assert_eq!(strip_bearer("Basic abc"), None);
    }

    #[test]
    fn origin_host_comparison() {
        assert!(origin_matches_host(
            "http://127.0.0.1:8787",
            Some("127.0.0.1:8787")
        ));
        assert!(origin_matches_host(
            "http://localhost:8787/x",
            Some("localhost:8787")
        ));
        assert!(origin_matches_host(
            "http://LOCALHOST:8787",
            Some("localhost:8787")
        ));
        assert!(!origin_matches_host(
            "http://evil.example",
            Some("127.0.0.1:8787")
        ));
        assert!(!origin_matches_host("http://127.0.0.1:8787", None));
    }
}
