//! API error handling with problem+json responses (RFC 9457).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// API error that produces a problem+json response.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub title: String,
    pub detail: String,
    pub instance: Option<String>,
}

impl ApiError {
    pub fn not_found(resource: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            title: "Resource not found".into(),
            detail: format!("{resource} not found"),
            instance: None,
        }
    }

    pub fn forbidden(operation: &str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            title: "Forbidden".into(),
            detail: format!("Operation not permitted: {operation}"),
            instance: None,
        }
    }

    pub fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            title: "Unauthorized".into(),
            detail: "Authentication required".into(),
            instance: None,
        }
    }

    pub fn bad_request(detail: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            title: "Bad request".into(),
            detail: detail.to_string(),
            instance: None,
        }
    }

    pub fn reauth_required() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            title: "Reauthentication required".into(),
            detail: "This operation requires recent reauthentication".into(),
            instance: None,
        }
    }

    pub fn rate_limited() -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            title: "Rate limited".into(),
            detail: "Too many requests. Please try again later.".into(),
            instance: None,
        }
    }

    pub fn conflict(detail: &str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            title: "Conflict".into(),
            detail: detail.to_string(),
            instance: None,
        }
    }
}

/// Problem+json body (RFC 9457).
#[derive(Debug, Serialize)]
struct Problem {
    #[serde(rename = "type")]
    kind: String,
    title: String,
    status: u16,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance: Option<String>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let problem = Problem {
            kind: "about:blank".into(),
            title: self.title,
            status: self.status.as_u16(),
            detail: self.detail,
            instance: self.instance,
        };
        (self.status, Json(problem)).into_response()
    }
}

/// Create a problem+json response directly.
pub fn problem_json(
    status: StatusCode,
    title: &str,
    detail: &str,
) -> Response {
    let problem = Problem {
        kind: "about:blank".into(),
        title: title.to_string(),
        status: status.as_u16(),
        detail: detail.to_string(),
        instance: None,
    };
    (status, Json(problem)).into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn not_found_error() {
        let e = ApiError::not_found("listing");
        assert_eq!(e.status, StatusCode::NOT_FOUND);
        assert!(e.detail.contains("listing"));
    }

    #[test]
    fn forbidden_error() {
        let e = ApiError::forbidden("approve");
        assert_eq!(e.status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn rate_limited_error() {
        let e = ApiError::rate_limited();
        assert_eq!(e.status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn reauth_error() {
        let e = ApiError::reauth_required();
        assert_eq!(e.status, StatusCode::UNAUTHORIZED);
        assert!(e.detail.contains("reauthentication"));
    }
}
