use async_trait::async_trait;

/// Default prompt version baked into the cache key.
pub const DEFAULT_PROMPT_VERSION: &str = "profile_extract.v1";

/// Default model. Overridable via [`ExtractOptions::model`].
pub const DEFAULT_MODEL: &str = "claude-haiku-4-5";

/// Tunable knobs for [`extract_profile_from_text`].
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub model: String,
    pub prompt_version: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub cache_schema: bool,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            prompt_version: DEFAULT_PROMPT_VERSION.to_string(),
            temperature: 0.0,
            max_tokens: 4096,
            cache_schema: true,
        }
    }
}

/// Small request DTO mirroring `careerai-llm::LlmRequest` minus the
/// fields we don't need at this boundary.
#[derive(Debug, Clone)]
pub struct ExtractRequest {
    pub system: String,
    pub profile_block: String,
    pub user: String,
    pub prompt_version: String,
    pub model: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub cache_schema: bool,
}

/// Provider-agnostic shim consumed by [`extract_profile_from_text`].
#[async_trait]
pub trait LlmCaller: Send + Sync {
    async fn call(&self, req: &ExtractRequest) -> Result<String, String>;
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("llm call failed: {0}")]
    LlmCall(String),

    #[error("response is not valid JSON: {0}")]
    ParseJson(String),

    #[error("response shape does not match Profile schema: {0}")]
    SchemaDeserialize(String),

    #[error("response failed schema validation: {0}")]
    SchemaValidate(String),

    #[error("schema validation failed twice; giving up")]
    MaxRetries,
}
