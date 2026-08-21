//! Chat Completions Transport configuration

use crate::config::provider::ProviderQuirks;

/// Chat Completions Transport configuration
#[derive(Debug, Clone)]
pub struct ChatCompletionsConfig {
    /// Provider name (e.g. "deepseek", "openai")
    pub name: String,
    /// API endpoint (without /chat/completions suffix)
    pub base_url: String,
    /// API key
    pub api_key: String,
    /// Model name
    pub model: String,
    /// Request timeout (seconds)
    pub timeout_secs: u64,
    /// Auth header name
    pub auth_header: String,
    /// Auth header value prefix
    pub auth_prefix: String,
    /// Explicit Provider type (when set, from_config uses this, skipping heuristic inference)
    pub provider_kind: Option<crate::api::ProviderKind>,
    /// Per-Provider protocol quirks embedded at construction time.
    /// The transport reads these directly instead of looking up ProviderRegistry
    /// or sniffing strings from base_url/model name.
    pub quirks: ProviderQuirks,
    /// Reasoning depth from user config (`config.toml [[providers]] reasoning_effort`).
    /// The transport emits `reasoning_effort` only when the Provider's quirks
    /// declare support (`supports_reasoning_effort`) — other providers ignore it.
    pub reasoning_effort: Option<String>,
}

impl ChatCompletionsConfig {
    // Constructors unified into ClientFactory / ModelRegistry.

    /// Full URL
    pub(crate) fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    /// Return a copy with the specified model
    pub fn with_model(&self, model: &str) -> Self {
        Self {
            model: model.to_string(),
            ..self.clone()
        }
    }

    /// Set explicit Provider type
    pub fn with_provider_kind(mut self, kind: crate::api::ProviderKind) -> Self {
        self.provider_kind = Some(kind);
        self
    }
}
