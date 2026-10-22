use crate::config::provider::*;
use crate::transports::anthropic::{AnthropicConfig, AnthropicTransport};
use crate::transports::Transport;
use std::sync::Arc;

/// Anthropic 兼容端点的 base_url 归一。
///
/// MiniMax 官方提供两条协议面：
/// - Chat Completions：`/v1`（`default_base_url()` 保持指向它，供模型级
///   覆盖切回 Chat Completions 时 factory 通用分支使用）。
/// - Anthropic Messages：`/anthropic`（本 provider 默认路由）。
///
/// 存量配置可能把旧 chat 端点写进了 `base_url`（配置向导曾按
/// `default_base_url()` 显式落盘）。把它精确改写成同源 Anthropic 端点，
/// 避免模型级升级后 Anthropic POST 打进 `/v1` chat 路径；其它值视为
/// 用户显式选择，原样透传。
fn anthropic_base_url(cfg_base_url: &str) -> String {
    const KNOWN_CHAT_BASES: [(&str, &str); 2] = [
        (
            "https://api.minimaxi.com/v1",
            "https://api.minimaxi.com/anthropic",
        ),
        ("https://api.minimax.io/v1", "https://api.minimax.io/anthropic"),
    ];
    let trimmed = cfg_base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return "https://api.minimaxi.com/anthropic".to_string();
    }
    for (chat_base, anthropic_base) in KNOWN_CHAT_BASES {
        if trimmed.eq_ignore_ascii_case(chat_base) {
            return anthropic_base.to_string();
        }
    }
    trimmed.to_string()
}

pub struct MiniMaxProvider;

impl Provider for MiniMaxProvider {
    fn id(&self) -> &'static str {
        "minimax"
    }
    fn display_name(&self) -> &'static str {
        "MiniMax"
    }
    fn default_base_url(&self) -> &'static str {
        "https://api.minimaxi.com/v1"
    }
    fn auth_header(&self) -> &'static str {
        "authorization"
    }
    fn auth_prefix(&self) -> &'static str {
        "Bearer "
    }
    fn default_model(&self) -> &'static str {
        "MiniMax-M2.7"
    }

    fn models(&self) -> &'static [ModelDef] {
        &[
            ModelDef {
                id: "MiniMax-M3",
                aliases: &["minimaxM3", "m3"],
                context_window: 250_000,
                max_output_tokens: 8_192,
                supports_streaming: true,
                supports_vision: true,
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
                id: "MiniMax-M2.7",
                aliases: &["minimax", "default"],
                context_window: 250_000,
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
            content_tool_tags: &["function_call", "tool", "action"],
            cache_hit_field: "",
        }
    }

    fn default_transport(&self) -> TransportKind {
        // MiniMax 官方 Anthropic 兼容端点（README/Claude Code 接入指引同款，
        // 与 pi / MiniMax 官方 anthropic-messages 链路一致）：M3/M2.5 走
        // Messages API 原生 thinking/tool_use 流。Chat Completions 仍可经
        // 模型级覆盖切回（factory 通用分支，默认端点见 default_base_url）。
        TransportKind::Anthropic
    }

    fn transport(&self, cfg: &ProviderConfig, model_id: &str) -> Arc<dyn Transport> {
        Arc::new(AnthropicTransport::new(AnthropicConfig {
            api_key: cfg.api_key.clone(),
            base_url: anthropic_base_url(&cfg.base_url),
            model: model_id.to_string(),
            timeout_secs: cfg.timeout_secs,
            provider_kind: Some(crate::api::ProviderKind::MiniMax),
            reasoning_effort: cfg.reasoning_effort.clone(),
            extra_headers: vec![],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_base_url_rewrites_legacy_chat_endpoints() {
        // 空值 → 官方 Anthropic 兼容端点
        assert_eq!(
            anthropic_base_url(""),
            "https://api.minimaxi.com/anthropic"
        );
        // 存量 chat 端点（配流向导按 default_base_url 落盘）→ 同源改写
        assert_eq!(
            anthropic_base_url("https://api.minimaxi.com/v1"),
            "https://api.minimaxi.com/anthropic"
        );
        assert_eq!(
            anthropic_base_url("https://api.minimax.io/v1/"),
            "https://api.minimax.io/anthropic"
        );
        // 用户显式端点原样透传（含自定义按 Anthropic 协议代理的网关）
        assert_eq!(
            anthropic_base_url("https://gw.example.com/anthropic"),
            "https://gw.example.com/anthropic"
        );
    }
}
