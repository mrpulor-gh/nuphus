use crate::config::provider::*;
use crate::transports::anthropic::{AnthropicConfig, AnthropicTransport};
use crate::transports::Transport;
use std::sync::Arc;

/// Anthropic 兼容端点的 base_url 归一。
///
/// DashScope 官方提供两条协议面：
/// - Chat Completions / OpenAI 兼容：`/compatible-mode/v1`
///   （`default_base_url()` 保持指向它，供模型级覆盖切回 Chat Completions
///   时 factory 通用分支使用）。
/// - Anthropic Messages：`/apps/anthropic`（本 provider 默认路由；官方
///   文档明确 base 不得以 `/v1/` 结尾，由客户端追加 `/v1/messages`）。
///
/// 与 minimax 同型：存量落盘的旧 chat 端点按当前 dashscope 语义精确改写
/// （含业务空间专属域 `{WorkspaceId}.cn-*.maas.aliyuncs.com` 上的同形路径）；
/// 其它值视为用户显式的 Anthropic 面端点，原样透传。
fn anthropic_base_url(cfg_base_url: &str) -> String {
    // 空 → 公网默认域；
    let trimmed = cfg_base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return "https://dashscope.aliyuncs.com/apps/anthropic".to_string();
    }
    // 旧 OpenAI 兼容面 → 官方 Anthropic 兼容面（同 host 同构换后缀）
    if let Some(host) = trimmed.strip_suffix("/compatible-mode/v1") {
        return format!("{host}/apps/anthropic");
    }
    trimmed.to_string()
}

pub struct QwenProvider;

impl Provider for QwenProvider {
    fn id(&self) -> &'static str {
        "qwen"
    }
    fn display_name(&self) -> &'static str {
        "Qwen"
    }
    fn default_base_url(&self) -> &'static str {
        "https://dashscope.aliyuncs.com/compatible-mode/v1"
    }
    fn auth_header(&self) -> &'static str {
        "authorization"
    }
    fn auth_prefix(&self) -> &'static str {
        "Bearer "
    }
    fn default_model(&self) -> &'static str {
        "qwen-plus"
    }

    fn models(&self) -> &'static [ModelDef] {
        &[
            ModelDef {
                id: "qwen-plus",
                aliases: &["qwen", "default"],
                context_window: 131_072,
                max_output_tokens: 8_192,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: false,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.0,
                cost_per_million_out: 0.0,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "qwen-turbo",
                aliases: &["qwen-fast"],
                context_window: 32_768,
                max_output_tokens: 8_192,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: false,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.0,
                cost_per_million_out: 0.0,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
        ]
    }

    fn quirks(&self) -> ProviderQuirks {
        ProviderQuirks {
            requires_reasoning_echo: false,
            supports_reasoning_effort: false,
            effort_excludes_tools: false,
            sanitize_tools: None,
            extra_headers: vec![],
            forbidden_request_fields: &[],
            max_tokens_field: MaxTokensField::MaxTokens,
            user_agent: None,
            content_tool_tags: &[],
            cache_hit_field: "",
        }
    }

    fn default_transport(&self) -> TransportKind {
        // DashScope 官方 Anthropic 兼容端点（/apps/anthropic，Anthropic SDK
        // 直进指引同款）：qwen3.7 系走 Messages API 原生 thinking/tool_use
        // 流；AnthropicTransport 以 x-api-key + anthropic-version 直连，
        // 与官方 SDK 语义一致。Chat Completions 仍可经模型级覆盖切回。
        TransportKind::Anthropic
    }

    fn transport(&self, cfg: &ProviderConfig, model_id: &str) -> Arc<dyn Transport> {
        Arc::new(AnthropicTransport::new(AnthropicConfig {
            api_key: cfg.api_key.clone(),
            base_url: anthropic_base_url(&cfg.base_url),
            model: model_id.to_string(),
            timeout_secs: cfg.timeout_secs,
            provider_kind: Some(crate::api::ProviderKind::Qwen),
            reasoning_effort: cfg.reasoning_effort.clone(),
            extra_headers: vec![],
            // 段名（providers.toml 段限定键）→ 模型级 max_tokens 精确解析
            provider_segment: cfg.name.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_base_url_rewrites_legacy_chat_endpoints() {
        // 空值 → 官方 Anthropic 兼容端点（公网默认域）
        assert_eq!(
            anthropic_base_url(""),
            "https://dashscope.aliyuncs.com/apps/anthropic"
        );
        // 存量 OpenAI 兼容面 → 同 host 的 Anthropic 兼容面
        assert_eq!(
            anthropic_base_url("https://dashscope.aliyuncs.com/compatible-mode/v1"),
            "https://dashscope.aliyuncs.com/apps/anthropic"
        );
        // 业务空间专属域上的同形路径同样改写
        assert_eq!(
            anthropic_base_url("https://w123.cn-hongkong.maas.aliyuncs.com/compatible-mode/v1"),
            "https://w123.cn-hongkong.maas.aliyuncs.com/apps/anthropic"
        );
        // 用户显式端点原样透传（含自定义 Anthropic 协议网关）
        assert_eq!(
            anthropic_base_url("https://gw.example.com/anthropic"),
            "https://gw.example.com/anthropic"
        );
    }
}
