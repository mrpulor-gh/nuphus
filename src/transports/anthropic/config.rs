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
    ///
    /// F6 归一（refactor 设计 §4.5）：base 若已以 `/v1` 结尾，先剥离再拼
    /// `/v1/messages`，避免 `/v1/v1` 双拼。官方 anthropic base 无 `/v1`
    /// （`https://api.anthropic.com`）→ 归一零影响；opencode zen/go/v1 →
    /// 归一为正确的 `.../v1/messages`。
    pub(crate) fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        format!("{base}/v1/messages")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(base_url: &str) -> AnthropicConfig {
        AnthropicConfig {
            api_key: "sk-test".into(),
            base_url: base_url.into(),
            model: "claude-test".into(),
            timeout_secs: 300,
            provider_kind: Some(crate::api::ProviderKind::Anthropic),
            reasoning_effort: None,
        }
    }

    /// 零回归断言：官方 base 无 /v1 → 拼 /v1/messages 行为不变。
    #[test]
    fn endpoint_official_base_unchanged() {
        assert_eq!(
            cfg("https://api.anthropic.com").endpoint(),
            "https://api.anthropic.com/v1/messages"
        );
    }

    /// base 已含 /v1（opencode zen/go/v1 场景）→ 剥离后拼，防 /v1/v1 双拼。
    #[test]
    fn endpoint_strips_embedded_v1() {
        assert_eq!(
            cfg("https://opencode.ai/zen/go/v1").endpoint(),
            "https://opencode.ai/zen/go/v1/messages"
        );
    }
}
