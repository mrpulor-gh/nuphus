//! Anthropic Messages API Transport configuration

/// Anthropic Transport configuration
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    /// API key (sent as x-api-key header)
    pub api_key: String,
    /// Base URL (default: https://api.anthropic.com)
    pub base_url: String,
    /// Model name
    pub model: String,
    /// Request timeout (seconds)
    pub timeout_secs: u64,
    /// Explicit Provider type
    pub provider_kind: Option<crate::api::ProviderKind>,
    /// Reasoning depth (`reasoning.effort`) — Anthropic extended thinking budget
    /// (`"none" | "low" | "high" | "max"`). None = keep current behavior (no
    /// reasoning block sent).
    pub reasoning_effort: Option<String>,
}

impl AnthropicConfig {
    /// Full URL for the Messages API endpoint
    pub(crate) fn endpoint(&self) -> String {
        format!("{}/v1/messages", self.base_url.trim_end_matches('/'))
    }
}
