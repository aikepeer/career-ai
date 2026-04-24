//! Fixture-driven `Llm` impl.
//!
//! Zero network. Keys fixtures on `LlmRequest::prompt_version` — an exact
//! match returns the canned `text`; a miss returns `LlmError::Upstream`
//! with the offending prompt_version so tests fail loudly.

use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use tracing::debug;

use crate::error::{LlmError, Result};
use crate::trait_def::Llm;
use crate::types::{LlmRequest, LlmResponse};

#[derive(Debug, Default, Clone)]
pub struct MockLlm {
    fixtures: HashMap<String, String>,
}

impl MockLlm {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a `MockLlm` with a single fixture.
    #[must_use]
    pub fn with_fixture(
        prompt_version: impl Into<String>,
        response_text: impl Into<String>,
    ) -> Self {
        let mut m = Self::new();
        m.insert_fixture(prompt_version, response_text);
        m
    }

    pub fn insert_fixture(
        &mut self,
        prompt_version: impl Into<String>,
        response_text: impl Into<String>,
    ) -> &mut Self {
        self.fixtures
            .insert(prompt_version.into(), response_text.into());
        self
    }

    /// Load every `*.json` in `dir`; filename stem is the prompt_version,
    /// the raw file contents is the canned response text (not parsed —
    /// callers decide whether the fixture is JSON or prose).
    pub fn from_dir(dir: impl AsRef<Path>) -> std::io::Result<Self> {
        let dir = dir.as_ref();
        let mut fixtures = HashMap::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let contents = std::fs::read_to_string(&path)?;
            fixtures.insert(stem.to_owned(), contents);
        }
        Ok(Self { fixtures })
    }
}

#[async_trait]
impl Llm for MockLlm {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn complete(&self, req: &LlmRequest) -> Result<LlmResponse> {
        match self.fixtures.get(&req.prompt_version) {
            Some(text) => {
                debug!(prompt_version = %req.prompt_version, "mock fixture hit");
                Ok(LlmResponse {
                    text: text.clone(),
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    cache_hit: false,
                    cached_prompt_tokens: 0,
                })
            }
            None => Err(LlmError::Upstream(format!(
                "no fixture for prompt_version={}",
                req.prompt_version
            ))),
        }
    }
}
