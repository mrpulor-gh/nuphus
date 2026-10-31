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
    /// 段级自定义请求头（anthropic 兼容中转实例的网关标头等）。官方段为空 →
    /// 注入循环零生效，请求头与改造前逐字节一致。`x-api-key` 被跳过（鉴权头
    /// 不可被覆盖成双值）。
    pub extra_headers: Vec<(String, String)>,
    /// providers.toml **段名**（如 "minimax" / "qwen"），用于把模型级
    /// `max_tokens` 精确解析到所属段。
    ///
    /// ⚠️ 必须与 Chat Completions 通道的 `provider_segment` 语义一致
    /// （见 `chat_completions::ChatCompletionsConfig::provider_segment`）：
    /// `ModelRegistry::get_max_output_tokens` 只认「段限定」，无绑定时不扫描
    /// 兄弟段（`config/model.rs` 的 `resolve_capability` 语义）。若这里传
    /// `None`，用户在 providers.toml 里显式配的模型级 `max_tokens` 会被**静默
    /// 丢弃**，Anthropic Messages 强制要求的 `max_tokens` 只能落到 8192 兜底
    /// ——长 thinking 流会吃满预算导致 text/tool_call 缺失（空交付）。
    ///
    /// 注意：这里存的是**段名**（`ProviderConfig.name`），不是 provider *type*。
    /// Chat Completions 侧注释提到 type id 会让 `custom-xxx` 实例全部塌缩到
    /// "custom" 而错过自己的段；段名同理，取 `cfg.name` 才是段限定键。
    pub provider_segment: String,
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
            extra_headers: Vec::new(),
            provider_segment: String::new(),
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
