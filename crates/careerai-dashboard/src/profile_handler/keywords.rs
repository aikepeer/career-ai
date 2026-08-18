//! Keyword add/remove/toggle against `config/local.yaml`.

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use std::path::Path;

use super::util::atomic_write;

#[derive(Debug, Deserialize)]
pub struct KeywordToggleRequest {
    pub keyword: String,
    pub action: String, // "add", "remove", "toggle"
}

pub async fn api_config_keywords(Json(payload): Json<KeywordToggleRequest>) -> impl IntoResponse {
    let kw = payload.keyword.trim();
    if kw.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Keyword cannot be empty" })),
        )
            .into_response();
    }

    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("cwd error: {e}") })),
            )
                .into_response();
        }
    };

    let config_path = cwd.join("config").join("local.yaml");
    match update_keywords_in_config(&config_path, &payload.action, kw) {
        Ok(modified) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "keyword": kw,
                "modified": modified,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// Merge a keyword add/remove/toggle into `config/local.yaml`'s
/// `sources.keywords` sequence, preserving every other key and any hand
/// edits. Returns whether the file changed. Unlike the previous
/// string-surgery implementation, this parses and re-serializes YAML so a
/// keyword containing quotes/colons/newlines cannot inject additional keys.
fn update_keywords_in_config(cfg_path: &Path, action: &str, keyword: &str) -> Result<bool, String> {
    let mut doc: serde_yaml::Value = if cfg_path.exists() {
        let raw = std::fs::read_to_string(cfg_path)
            .map_err(|e| format!("read {}: {e}", cfg_path.display()))?;
        serde_yaml::from_str(&raw).map_err(|e| format!("parse {}: {e}", cfg_path.display()))?
    } else {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    };

    let root = doc
        .as_mapping_mut()
        .ok_or_else(|| "config root must be a YAML mapping".to_string())?;
    let sources = root
        .entry(serde_yaml::Value::String("sources".to_string()))
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    let sources_map = sources
        .as_mapping_mut()
        .ok_or_else(|| "`sources` must be a YAML mapping".to_string())?;
    let keywords = sources_map
        .entry(serde_yaml::Value::String("keywords".to_string()))
        .or_insert_with(|| serde_yaml::Value::Sequence(Vec::new()));
    let seq = keywords
        .as_sequence_mut()
        .ok_or_else(|| "`sources.keywords` must be a YAML sequence".to_string())?;

    let present = seq.iter().any(|v| v.as_str() == Some(keyword));
    let mut modified = false;
    match action {
        "add" => {
            if !present {
                seq.push(serde_yaml::Value::String(keyword.to_string()));
                modified = true;
            }
        }
        "remove" => {
            if present {
                seq.retain(|v| v.as_str() != Some(keyword));
                modified = true;
            }
        }
        "toggle" => {
            if present {
                seq.retain(|v| v.as_str() != Some(keyword));
            } else {
                seq.push(serde_yaml::Value::String(keyword.to_string()));
            }
            modified = true;
        }
        _ => {}
    }

    if modified {
        let yaml = serde_yaml::to_string(&doc).map_err(|e| format!("serialize config: {e}"))?;
        atomic_write(cfg_path, &yaml)?;
    }
    Ok(modified)
}

pub fn load_keywords_from_config(
    cfg: Option<&careerai_core::config::CoreConfig>,
) -> Vec<crate::view::KeywordStatus> {
    let mut keywords = cfg.map_or_else(
        || {
            vec![
                "AI/ML".into(),
                "Embedded Systems".into(),
                "Robotics".into(),
                "Rust".into(),
            ]
        },
        |c| c.sources.keywords.clone(),
    );
    keywords.sort_by_key(|a| a.to_lowercase());
    keywords
        .into_iter()
        .map(|k| crate::view::KeywordStatus {
            name: k,
            enabled: true,
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn update_keywords_add_preserves_unrelated_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config").join("local.yaml");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, "llm:\n  model: claude-3-5-sonnet\n").unwrap();

        let modified = update_keywords_in_config(&cfg, "add", "machine learning").unwrap();
        assert!(modified);

        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            text.contains("model: claude-3-5-sonnet"),
            "llm key lost: {text}"
        );
        assert!(text.contains("machine learning"), "keyword missing: {text}");
    }

    #[test]
    fn update_keywords_add_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config").join("local.yaml");

        assert!(update_keywords_in_config(&cfg, "add", "rust").unwrap());
        assert!(!update_keywords_in_config(&cfg, "add", "rust").unwrap());

        let doc: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        let seq = doc["sources"]["keywords"].as_sequence().unwrap();
        assert_eq!(seq.len(), 1);
        assert_eq!(seq[0].as_str(), Some("rust"));
    }

    #[test]
    fn update_keywords_toggle_removes_then_readds() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config").join("local.yaml");

        assert!(update_keywords_in_config(&cfg, "toggle", "robotics").unwrap());
        assert!(update_keywords_in_config(&cfg, "toggle", "robotics").unwrap());

        let doc: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert!(doc["sources"]["keywords"].as_sequence().unwrap().is_empty());
    }

    #[test]
    fn update_keywords_remove_does_not_touch_other_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config").join("local.yaml");
        update_keywords_in_config(&cfg, "add", "rust").unwrap();
        update_keywords_in_config(&cfg, "add", "go").unwrap();

        let modified = update_keywords_in_config(&cfg, "remove", "rust").unwrap();
        assert!(modified);

        let doc: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        let seq = doc["sources"]["keywords"].as_sequence().unwrap();
        assert_eq!(seq.len(), 1);
        assert_eq!(seq[0].as_str(), Some("go"));
    }

    #[test]
    fn update_keywords_treats_keyword_as_scalar() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config").join("local.yaml");

        // A hostile keyword must stay a scalar value, not inject new keys.
        let evil = "x\nllm:\n  auto_submit: true";
        update_keywords_in_config(&cfg, "add", evil).unwrap();

        let doc: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        let seq = doc["sources"]["keywords"].as_sequence().unwrap();
        assert_eq!(seq.len(), 1);
        assert_eq!(seq[0].as_str(), Some(evil));
        assert!(
            doc.get("llm").is_none() || doc["llm"].get("auto_submit").is_none(),
            "keyword must not inject config keys"
        );
    }
}
