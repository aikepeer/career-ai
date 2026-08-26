#![cfg(unix)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::unwrap_in_result)]

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use crate::backend::Backend;
use crate::cache::Cache;
use crate::trait_def::Llm;
use crate::types::LlmRequest;
use careerai_core::config::{BackendChoice, LlmConfig};

#[tokio::test]
async fn backend_resolution_forwards_goose_provider_and_default_model() {
    let _guard = crate::ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let stub = dir.path().join("goose");
    let script = r#"#!/bin/sh
args="$*"
case "$args" in
  *"--provider antigravity"*) ;;
  *) echo "provider missing: $args" >&2; exit 2 ;;
esac
case "$args" in
  *"--model model-default"*) ;;
  *) echo "model missing: $args" >&2; exit 2 ;;
esac
case "$args" in
  *"--max-turns 1"*) ;;
  *) echo "turn limit missing: $args" >&2; exit 2 ;;
esac
cat >/dev/null
printf '%s' '{"messages":[{"role":"assistant","content":[{"text":"BACKEND_OK"}]}],"metadata":{"input_tokens":4,"output_tokens":2,"status":"completed"}}'
"#;
    std::fs::write(&stub, script).unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let previous_path = std::env::var("PATH").ok();
    let model_vars = ["LLM_MODEL", "CAREERAI_LLM_MODEL", "MODEL"];
    let previous_models: Vec<_> = model_vars
        .iter()
        .map(|key| (*key, std::env::var(key).ok()))
        .collect();
    for key in model_vars {
        std::env::remove_var(key);
    }
    let path_value = dir.path().to_string_lossy().into_owned();
    std::env::set_var("PATH", path_value);
    let mut cfg = LlmConfig::default();
    cfg.backend = BackendChoice::Goose;
    cfg.provider = "antigravity".into();
    cfg.model = "provider/model-default".into();
    cfg.max_retries = 0;

    let cache = Arc::new(Cache::new(dir.path().join("cache")));
    let backend = Backend::resolve(BackendChoice::Goose, &cfg, cache)
        .await
        .expect("Goose backend should resolve from PATH");
    let response = backend
        .complete(&LlmRequest {
            system: "system".into(),
            profile_block: "profile".into(),
            user: "request".into(),
            prompt_version: "test".into(),
            model: String::new(),
            temperature: 0.0,
            max_tokens: 8,
            cache_profile: false,
        })
        .await
        .expect("Goose stub should complete");

    if let Some(path) = previous_path {
        std::env::set_var("PATH", path);
    } else {
        std::env::remove_var("PATH");
    }
    for (key, value) in previous_models {
        if let Some(value) = value {
            std::env::set_var(key, value);
        } else {
            std::env::remove_var(key);
        }
    }
    assert_eq!(response.text, "BACKEND_OK");
    assert_eq!(response.prompt_tokens, 4);
    assert_eq!(response.completion_tokens, 2);
}
