//! Session authentication middleware.
//!
//! `SessionAuth` is an Axum extractor that reads the `Authorization: Bearer
//! <token>` header, hashes the token, looks up the session, and verifies it.
//! Handlers that require authentication accept `SessionAuth` as a parameter.

use std::sync::Arc;

use axum::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::auth::session::{SessionManager, SessionRecord};

/// Extracted and verified session, injected into authenticated handlers.
#[derive(Debug, Clone)]
pub struct SessionAuth {
    pub record: SessionRecord,
    /// The raw token (needed for session rotation responses).
    pub raw_token: String,
}

impl SessionAuth {
    /// Role of the authenticated user.
    pub fn role_str(&self) -> &str {
        &self.record.role
    }

    /// Tenant ID from the session.
    pub fn tenant_id(&self) -> &str {
        &self.record.tenant_id
    }

    /// User ID from the session.
    pub fn user_id(&self) -> &str {
        &self.record.user_id
    }

    /// Workspace ID from the session.
    pub fn workspace_id(&self) -> &str {
        &self.record.workspace_id
    }
}

#[async_trait]
impl FromRequestParts<Arc<AppState>> for SessionAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(ApiError::unauthorized)?;

        let raw_token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(ApiError::unauthorized)?;

        let token_hash = SessionManager::hash_token(raw_token);
        let sessions = state.sessions.read().await;
        let record = sessions
            .get(&token_hash)
            .cloned()
            .ok_or_else(ApiError::unauthorized)?;
        drop(sessions);

        SessionManager::verify(&record, raw_token, chrono::Utc::now())
            .map_err(|_| ApiError::unauthorized())?;

        Ok(SessionAuth {
            record,
            raw_token: raw_token.to_string(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::auth::session::SessionManager;

    use axum::http::Request;

    fn make_request(token: Option<&str>) -> Request<()> {
        let mut builder = Request::builder();
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        builder.body(()).unwrap()
    }

    #[tokio::test]
    async fn extracts_valid_session() {
        let state = AppState::arc([0u8; 32]);
        let (raw, record) = SessionManager::create("t1", "u1", "w1", "owner");
        state
            .sessions
            .write()
            .await
            .insert(record.token_hash.clone(), record);

        let req = make_request(Some(&raw));
        let (mut parts, ()) = req.into_parts();
        let auth = SessionAuth::from_request_parts(&mut parts, &state)
            .await
            .unwrap();
        assert_eq!(auth.tenant_id(), "t1");
        assert_eq!(auth.user_id(), "u1");
    }

    #[tokio::test]
    async fn rejects_missing_header() {
        let state = AppState::arc([0u8; 32]);
        let req = make_request(None);
        let (mut parts, ()) = req.into_parts();
        let result = SessionAuth::from_request_parts(&mut parts, &state).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rejects_bad_token() {
        let state = AppState::arc([0u8; 32]);
        let req = make_request(Some("not-a-real-token"));
        let (mut parts, ()) = req.into_parts();
        let result = SessionAuth::from_request_parts(&mut parts, &state).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rejects_revoked_session() {
        let state = AppState::arc([0u8; 32]);
        let (raw, mut record) = SessionManager::create("t1", "u1", "w1", "owner");
        record.revoked = true;
        state
            .sessions
            .write()
            .await
            .insert(record.token_hash.clone(), record);

        let req = make_request(Some(&raw));
        let (mut parts, ()) = req.into_parts();
        let result = SessionAuth::from_request_parts(&mut parts, &state).await;
        assert!(result.is_err());
    }
}
