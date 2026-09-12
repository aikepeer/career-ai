//! Worker adapters reusing existing careerai crate logic (PR 6+7).
//!
//! Workers receive a signed tenant/job context, re-resolve entitlement
//! in-transaction, and dispatch to the appropriate adapter. Each adapter
//! wraps calls into existing pure-logic crates (careerai-profile,
//! careerai-match, careerai-tailor, careerai-render) without modifying
//! their behavior.

pub mod artifacts;
pub mod outcomes;
pub mod preparation;

use serde::{Deserialize, Serialize};

/// Kinds of durable jobs the hosted workers handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    ProfileImport,
    Discovery,
    Match,
    PreparationProgram,
    TailorArtifact,
    RenderArtifact,
}

/// A queued job awaiting execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedJob {
    pub kind: JobKind,
    pub payload: serde_json::Value,
    pub tenant_id: uuid::Uuid,
    pub actor_id: uuid::Uuid,
    pub attempt: u32,
    pub fencing_token: u64,
}

/// Result of a worker execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerResult {
    pub success: bool,
    pub output: serde_json::Value,
    pub error: Option<String>,
}

impl WorkerResult {
    pub fn ok(output: serde_json::Value) -> Self {
        Self {
            success: true,
            output,
            error: None,
        }
    }

    pub fn fail(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            output: serde_json::Value::Null,
            error: Some(msg.into()),
        }
    }
}

/// Dispatch a queued job to the appropriate handler.
///
/// This is the entry point called by the durable job runner after
/// acquiring a lease. The actual adapter logic lives in the submodule
/// types — this function routes based on `JobKind`.
pub fn dispatch(job: &QueuedJob) -> WorkerResult {
    match job.kind {
        JobKind::PreparationProgram => {
            let req: preparation::PreparationRequest =
                match serde_json::from_value(job.payload.clone()) {
                    Ok(r) => r,
                    Err(e) => return WorkerResult::fail(format!("invalid payload: {e}")),
                };
            let program = preparation::generate_program(&req);
            WorkerResult::ok(serde_json::to_value(&program).unwrap_or_default())
        }
        JobKind::ProfileImport
        | JobKind::Discovery
        | JobKind::Match
        | JobKind::TailorArtifact
        | JobKind::RenderArtifact => WorkerResult::fail(format!(
            "{} jobs require I/O adapters not yet wired in this beta build",
            job_payload_name(&job.kind)
        )),
    }
}

fn job_payload_name(kind: &JobKind) -> &'static str {
    match kind {
        JobKind::ProfileImport => "profile_import",
        JobKind::Discovery => "discovery",
        JobKind::Match => "match",
        JobKind::TailorArtifact => "tailor_artifact",
        JobKind::RenderArtifact => "render_artifact",
        JobKind::PreparationProgram => "preparation_program",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_preparation_program_succeeds() {
        let job = QueuedJob {
            kind: JobKind::PreparationProgram,
            payload: serde_json::json!({
                "company_record": {
                    "name": "Acme",
                    "values": [],
                    "interview_process": [],
                    "benefits": [],
                    "known_questions": []
                },
                "listing_evidence": [],
                "profile_skills": []
            }),
            tenant_id: uuid::Uuid::new_v4(),
            actor_id: uuid::Uuid::new_v4(),
            attempt: 1,
            fencing_token: 1,
        };
        let result = dispatch(&job);
        assert!(result.success, "{}", result.error.unwrap_or_default());
    }

    #[test]
    fn dispatch_io_job_fails_gracefully() {
        let job = QueuedJob {
            kind: JobKind::ProfileImport,
            payload: serde_json::Value::Null,
            tenant_id: uuid::Uuid::new_v4(),
            actor_id: uuid::Uuid::new_v4(),
            attempt: 1,
            fencing_token: 1,
        };
        let result = dispatch(&job);
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("profile_import"));
    }

    #[test]
    fn dispatch_invalid_payload_fails() {
        let job = QueuedJob {
            kind: JobKind::PreparationProgram,
            payload: serde_json::json!("not an object"),
            tenant_id: uuid::Uuid::new_v4(),
            actor_id: uuid::Uuid::new_v4(),
            attempt: 1,
            fencing_token: 1,
        };
        let result = dispatch(&job);
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("invalid payload"));
    }

    #[test]
    fn worker_result_ok_and_fail() {
        let ok = WorkerResult::ok(serde_json::json!({"id": 1}));
        assert!(ok.success);
        assert!(ok.error.is_none());

        let fail = WorkerResult::fail("boom");
        assert!(!fail.success);
        assert_eq!(fail.error.as_deref(), Some("boom"));
    }
}
