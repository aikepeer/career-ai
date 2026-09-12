//! Review-only tailored artifacts (PR 7).
//!
//! Defines the types for review-only artifact generation. The actual
//! tailoring reuses careerai-tailor's constrained-diff logic; rendering
//! reuses careerai-render's sandboxed pandoc path. No live submission
//! occurs — artifacts are preview/download only.

use serde::{Deserialize, Serialize};

/// Kind of tailored artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Resume,
    CoverLetter,
}

/// Status of an artifact in the review workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    /// Generated, awaiting user review.
    Preview,
    /// User approved the artifact.
    Approved,
    /// User rejected the artifact.
    Rejected,
}

/// A review-only tailored artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewArtifact {
    pub id: uuid::Uuid,
    pub tenant_id: uuid::Uuid,
    pub application_id: uuid::Uuid,
    pub kind: ArtifactKind,
    pub status: ArtifactStatus,
    /// The constrained diff applied to the profile (resume only).
    pub diff_json: Option<String>,
    /// Path to the rendered preview (set after rendering).
    pub preview_path: Option<String>,
    /// SHA-256 digest of the rendered content for integrity checking.
    pub content_digest: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub reviewed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Validate that a constrained diff does not invent new content.
///
/// This delegates to careerai-tailor's diff validator. The diff must
/// only reorder or rewrite existing bullets — never add new experience,
/// titles, dates, or employers.
pub fn validate_diff(diff_json: &str) -> Result<(), DiffValidationError> {
    let parsed: serde_json::Value = serde_json::from_str(diff_json)
        .map_err(|e| DiffValidationError::MalformedJson(e.to_string()))?;

    let obj = parsed.as_object().ok_or(DiffValidationError::NotAnObject)?;

    // Check for forbidden operations that invent new content
    if let Some(ops) = obj.get("ops").and_then(|o| o.as_array()) {
        for op in ops {
            let op_obj = op.as_object().ok_or(DiffValidationError::OpNotAnObject)?;
            let op_type = op_obj.get("op").and_then(|t| t.as_str()).unwrap_or("");

            match op_type {
                "reorder" | "rewrite" | "omit" => { /* allowed */ }
                "add" | "create" | "insert" => {
                    return Err(DiffValidationError::InventedContent {
                        op: op_type.to_string(),
                    });
                }
                _ => {
                    return Err(DiffValidationError::UnknownOp {
                        op: op_type.to_string(),
                    });
                }
            }
        }
    }

    Ok(())
}

/// Error from diff validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffValidationError {
    MalformedJson(String),
    NotAnObject,
    OpNotAnObject,
    UnknownOp { op: String },
    InventedContent { op: String },
}

impl std::fmt::Display for DiffValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedJson(e) => write!(f, "malformed JSON: {e}"),
            Self::NotAnObject => write!(f, "diff root must be a JSON object"),
            Self::OpNotAnObject => write!(f, "each op must be a JSON object"),
            Self::UnknownOp { op } => write!(f, "unknown op type: {op}"),
            Self::InventedContent { op } => write!(
                f,
                "op '{op}' invents new content — only reorder, rewrite, and omit are allowed"
            ),
        }
    }
}

impl std::error::Error for DiffValidationError {}

/// Transition an artifact's review status.
///
/// Artifacts can go Preview → Approved or Preview → Rejected.
/// Once Approved or Rejected, the status is terminal.
pub fn transition_status(
    current: ArtifactStatus,
    target: ArtifactStatus,
) -> Result<ArtifactStatus, StatusTransitionError> {
    match (&current, &target) {
        (ArtifactStatus::Preview, ArtifactStatus::Approved) => Ok(ArtifactStatus::Approved),
        (ArtifactStatus::Preview, ArtifactStatus::Rejected) => Ok(ArtifactStatus::Rejected),
        (ArtifactStatus::Approved | ArtifactStatus::Rejected, _) => {
            Err(StatusTransitionError::TerminalState { current, target })
        }
        (_, ArtifactStatus::Preview) => Err(StatusTransitionError::CannotRevert { current }),
    }
}

/// Error from an invalid status transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusTransitionError {
    TerminalState {
        current: ArtifactStatus,
        target: ArtifactStatus,
    },
    CannotRevert {
        current: ArtifactStatus,
    },
}

impl std::fmt::Display for StatusTransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TerminalState { current, target } => {
                write!(
                    f,
                    "cannot transition from terminal state {current:?} to {target:?}"
                )
            }
            Self::CannotRevert { current } => {
                write!(f, "cannot revert from {current:?} back to Preview")
            }
        }
    }
}

impl std::error::Error for StatusTransitionError {}

/// Compute a SHA-256 digest of rendered content for integrity checking.
pub fn content_digest(content: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_reorder_and_rewrite() {
        let diff = r#"{"ops":[{"op":"reorder"},{"op":"rewrite"},{"op":"omit"}]}"#;
        assert!(validate_diff(diff).is_ok());
    }

    #[test]
    fn validate_rejects_add_op() {
        let diff = r#"{"ops":[{"op":"add"}]}"#;
        let err = validate_diff(diff).unwrap_err();
        assert_eq!(
            err,
            DiffValidationError::InventedContent { op: "add".into() }
        );
    }

    #[test]
    fn validate_rejects_create_op() {
        let diff = r#"{"ops":[{"op":"create"}]}"#;
        let err = validate_diff(diff).unwrap_err();
        assert_eq!(
            err,
            DiffValidationError::InventedContent {
                op: "create".into()
            }
        );
    }

    #[test]
    fn validate_rejects_insert_op() {
        let diff = r#"{"ops":[{"op":"insert"}]}"#;
        let err = validate_diff(diff).unwrap_err();
        assert_eq!(
            err,
            DiffValidationError::InventedContent {
                op: "insert".into()
            }
        );
    }

    #[test]
    fn validate_rejects_unknown_op() {
        let diff = r#"{"ops":[{"op":"teleport"}]}"#;
        let err = validate_diff(diff).unwrap_err();
        assert_eq!(
            err,
            DiffValidationError::UnknownOp {
                op: "teleport".into()
            }
        );
    }

    #[test]
    fn validate_rejects_non_object_root() {
        let diff = r"[]";
        assert!(matches!(
            validate_diff(diff),
            Err(DiffValidationError::NotAnObject)
        ));
    }

    #[test]
    fn validate_rejects_malformed_json() {
        let diff = r"not json";
        assert!(matches!(
            validate_diff(diff),
            Err(DiffValidationError::MalformedJson(_))
        ));
    }

    #[test]
    fn transition_preview_to_approved() {
        let result = transition_status(ArtifactStatus::Preview, ArtifactStatus::Approved);
        assert_eq!(result.unwrap(), ArtifactStatus::Approved);
    }

    #[test]
    fn transition_preview_to_rejected() {
        let result = transition_status(ArtifactStatus::Preview, ArtifactStatus::Rejected);
        assert_eq!(result.unwrap(), ArtifactStatus::Rejected);
    }

    #[test]
    fn transition_approved_is_terminal() {
        let result = transition_status(ArtifactStatus::Approved, ArtifactStatus::Rejected);
        assert!(matches!(
            result,
            Err(StatusTransitionError::TerminalState { .. })
        ));
    }

    #[test]
    fn transition_rejected_is_terminal() {
        let result = transition_status(ArtifactStatus::Rejected, ArtifactStatus::Approved);
        assert!(matches!(
            result,
            Err(StatusTransitionError::TerminalState { .. })
        ));
    }

    #[test]
    fn content_digest_is_stable() {
        let content = b"hello world";
        let d1 = content_digest(content);
        let d2 = content_digest(content);
        assert_eq!(d1, d2);
        assert_eq!(d1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[test]
    fn content_digest_differs_for_different_input() {
        let d1 = content_digest(b"hello");
        let d2 = content_digest(b"world");
        assert_ne!(d1, d2);
    }
}
