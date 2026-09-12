//! Resource route handlers: import_profile, create_program,
//! list_listings, preview_application.
//!
//! These handlers dispatch to the worker adapters which call into the
//! existing careerai-profile, careerai-match, careerai-tailor crates.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::api::middleware::SessionAuth;
use crate::api::state::AppState;
use crate::entitlement::admission::{admit, AdmissionResult};
use crate::workers::adapters::{
    build_preparation_from_matches, MatchListing, MatchResult, ProfileImportRequest,
    ProfileImportResult, TailorRequest,
};
use crate::workers::preparation::PreparationProgram;
use crate::workers::{dispatch, JobKind, QueuedJob, WorkerResult};

// ── Profile import ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ImportProfileRequest {
    pub paths: Vec<String>,
}

pub async fn import_profile(
    State(state): State<Arc<AppState>>,
    session: SessionAuth,
    Json(req): Json<ImportProfileRequest>,
) -> Result<Json<ProfileImportResult>, ApiError> {
    // Entitlement admission
    let mut entitlements = state.entitlements.write().await;
    let entitlement = entitlements
        .get_mut(session.tenant_id())
        .ok_or_else(|| ApiError::forbidden("no entitlement"))?;
    match admit(entitlement, "import", 1, chrono::Utc::now()) {
        AdmissionResult::Free | AdmissionResult::Admitted(_) => {}
        AdmissionResult::Denied(_) => return Err(ApiError::forbidden("usage limit exceeded")),
    }
    drop(entitlements);

    // Dispatch profile import job
    let job = QueuedJob {
        kind: JobKind::ProfileImport,
        payload: serde_json::to_value(ProfileImportRequest { paths: req.paths })
            .map_err(|e| ApiError::bad_request(&format!("serialization: {e}")))?,
        tenant_id: uuid::Uuid::parse_str(session.tenant_id())
            .unwrap_or_else(|_| uuid::Uuid::nil()),
        actor_id: uuid::Uuid::parse_str(session.user_id())
            .unwrap_or_else(|_| uuid::Uuid::nil()),
        attempt: 1,
        fencing_token: 1,
    };
    let result = dispatch(&job);
    if !result.success {
        return Err(ApiError::bad_request(
            result.error.as_deref().unwrap_or("import failed"),
        ));
    }
    let import_result: ProfileImportResult = serde_json::from_value(result.output)
        .map_err(|e| ApiError::bad_request(&format!("result parse: {e}")))?;
    Ok(Json(import_result))
}

// ── Preparation program ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateProgramRequest {
    pub company_name: String,
    pub profile_yaml: String,
    pub listings: Vec<MatchListing>,
    pub matches: Vec<MatchResult>,
}

pub async fn create_program(
    State(state): State<Arc<AppState>>,
    session: SessionAuth,
    Json(req): Json<CreateProgramRequest>,
) -> Result<Json<PreparationProgram>, ApiError> {
    // Entitlement admission
    let mut entitlements = state.entitlements.write().await;
    let entitlement = entitlements
        .get_mut(session.tenant_id())
        .ok_or_else(|| ApiError::forbidden("no entitlement"))?;
    match admit(entitlement, "preparation", 1, chrono::Utc::now()) {
        AdmissionResult::Free | AdmissionResult::Admitted(_) => {}
        AdmissionResult::Denied(_) => return Err(ApiError::forbidden("usage limit exceeded")),
    }
    drop(entitlements);

    // Build preparation request from profile + matches
    let profile: careerai_profile::Profile = serde_yaml::from_str(&req.profile_yaml)
        .map_err(|e| ApiError::bad_request(&format!("profile YAML: {e}")))?;
    let prep_req = build_preparation_from_matches(
        &profile,
        &req.matches,
        &req.listings,
        &req.company_name,
    );

    // Dispatch preparation program job
    let job = QueuedJob {
        kind: JobKind::PreparationProgram,
        payload: serde_json::to_value(&prep_req)
            .map_err(|e| ApiError::bad_request(&format!("serialization: {e}")))?,
        tenant_id: uuid::Uuid::parse_str(session.tenant_id())
            .unwrap_or_else(|_| uuid::Uuid::nil()),
        actor_id: uuid::Uuid::parse_str(session.user_id())
            .unwrap_or_else(|_| uuid::Uuid::nil()),
        attempt: 1,
        fencing_token: 1,
    };
    let result = dispatch(&job);
    if !result.success {
        return Err(ApiError::bad_request(
            result.error.as_deref().unwrap_or("preparation failed"),
        ));
    }
    let program: PreparationProgram = serde_json::from_value(result.output)
        .map_err(|e| ApiError::bad_request(&format!("result parse: {e}")))?;
    Ok(Json(program))
}

// ── Listings ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ListingSummary {
    pub id: String,
    pub title: String,
    pub company: String,
    pub score: Option<f64>,
}

pub async fn list_listings(
    State(_state): State<Arc<AppState>>,
    _session: SessionAuth,
) -> Result<Json<Vec<ListingSummary>>, ApiError> {
    // Beta: return empty list (discovery jobs populate this in production)
    Ok(Json(Vec::new()))
}

// ── Application preview (tailor) ────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PreviewRequest {
    pub profile_yaml: String,
    pub listing: MatchListing,
    pub drop_threshold: Option<f32>,
}

#[derive(Debug, Serialize)]
pub struct PreviewResponse {
    pub application_id: String,
    pub resume_view_json: String,
    pub cover_letter_body: String,
    pub diff_json: String,
}

pub async fn preview_application(
    State(state): State<Arc<AppState>>,
    session: SessionAuth,
    Path(_id): Path<String>,
    Json(req): Json<PreviewRequest>,
) -> Result<Json<PreviewResponse>, ApiError> {
    // Entitlement admission
    let mut entitlements = state.entitlements.write().await;
    let entitlement = entitlements
        .get_mut(session.tenant_id())
        .ok_or_else(|| ApiError::forbidden("no entitlement"))?;
    match admit(entitlement, "tailor", 1, chrono::Utc::now()) {
        AdmissionResult::Free | AdmissionResult::Admitted(_) => {}
        AdmissionResult::Denied(_) => return Err(ApiError::forbidden("usage limit exceeded")),
    }
    drop(entitlements);

    let tailor_req = TailorRequest {
        profile_yaml: req.profile_yaml,
        listing: req.listing,
        drop_threshold: req.drop_threshold.unwrap_or(0.3),
    };

    let job = QueuedJob {
        kind: JobKind::TailorArtifact,
        payload: serde_json::to_value(&tailor_req)
            .map_err(|e| ApiError::bad_request(&format!("serialization: {e}")))?,
        tenant_id: uuid::Uuid::parse_str(session.tenant_id())
            .unwrap_or_else(|_| uuid::Uuid::nil()),
        actor_id: uuid::Uuid::parse_str(session.user_id())
            .unwrap_or_else(|_| uuid::Uuid::nil()),
        attempt: 1,
        fencing_token: 1,
    };
    let result: WorkerResult = dispatch(&job);
    if !result.success {
        return Err(ApiError::bad_request(
            result.error.as_deref().unwrap_or("tailoring failed"),
        ));
    }
    let tailor_result: crate::workers::adapters::TailorResult =
        serde_json::from_value(result.output)
            .map_err(|e| ApiError::bad_request(&format!("result parse: {e}")))?;
    Ok(Json(PreviewResponse {
        application_id: tailor_result.application_id,
        resume_view_json: tailor_result.resume_view_json,
        cover_letter_body: tailor_result.cover_letter_body,
        diff_json: tailor_result.diff_json,
    }))
}
