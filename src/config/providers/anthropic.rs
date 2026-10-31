use crate::config::provider::*;
use crate::transports::anthropic::{AnthropicConfig, AnthropicTransport};
use crate::transports::Transport;
use std::sync::Arc;

pub struct AnthropicProvider;

impl Provider for AnthropicProvider {
    fn id(&self) -> &'static str {
        "anthropic"
    }
    fn display_name(&self) -> &'static str {
        "Anthropic"
    }
    fn default_base_url(&self) -> &'static str {
        "https://api.anthropic.com"
    }
    fn auth_header(&self) -> &'static str {
        "x-api-key"
    }
    fn auth_prefix(&self) -> &'static str {
        ""
    }
    fn default_model(&self) -> &'static str {
        "claude-sonnet-5"
    }

    fn models(&self) -> &'static [ModelDef] {
        &[
            ModelDef {
                id: "claude-sonnet-5",
                aliases: &["claude", "default", "sonnet"],
                context_window: 200_000,
                max_output_tokens: 8_192,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 3.0,
                cost_per_million_out: 15.0,
                reasoning_field: "",
                // 官方依据（platform.claude.com · Effort，2026-10 核对）：effort 五档
                // low / medium / high / xhigh / max（high 为多数模型默认；Opus 5.5 /
                // Haiku 5.5 默认 medium）。旧列表 none/low/high/max 来自 DeepSeek 对
                // Anthropic 兼容格式的转述，非 Anthropic 官方枚举；none 也不是官方档位
                // （关思考走 thinking.type=disabled，不走 effort）。
                reasoning_efforts: &["low", "medium", "high", "xhigh", "max"],
                default_effort: Some("high"),
            },
            ModelDef {
                id: "claude-opus-4-8",
                aliases: &["opus", "claude-opus"],
                context_window: 200_000,
                max_output_tokens: 8_192,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 15.0,
                cost_per_million_out: 75.0,
                reasoning_field: "",
                // 同上：官方五档 low/medium/high/xhigh/max（availability 页：
                // max 覆盖 Claude Opus 4.8）。
                reasoning_efforts: &["low", "medium", "high", "xhigh", "max"],
                default_effort: Some("high"),
            },
            ModelDef {
                id: "claude-haiku-4-5-20251001",
                aliases: &["haiku", "claude-haiku"],
                context_window: 200_000,
                max_output_tokens: 8_192,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: false,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.80,
                cost_per_million_out: 4.0,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
        ]
    }

    fn quirks(&self) -> ProviderQuirks {
        ProviderQuirks {
            requires_reasoning_echo: false,
            // 官方 Messages API 接受 effort（output_config.effort，见 transport）。
            // 该 quirk 闸门只作用于 chat_completions / responses 引擎；Anthropic
            // 原生 transport 不读它，置 true 仅为元数据与其它 provider 一致。
            supports_reasoning_effort: true,
            effort_excludes_tools: false,
            sanitize_tools: None,
            extra_headers: vec![("anthropic-version".to_string(), "2023-06-01".to_string())],
            forbidden_request_fields: &[],
            max_tokens_field: MaxTokensField::MaxTokens,
            user_agent: None,
            content_tool_tags: &[],
            cache_hit_field: "",
        }
    }

    fn default_transport(&self) -> TransportKind {
        // 声明协议族与既有 transport() 实现一致（返回 AnthropicTransport）。
        TransportKind::Anthropic
    }

    fn transport(&self, cfg: &ProviderConfig, model_id: &str) -> Arc<dyn Transport> {
        Arc::new(AnthropicTransport::new(AnthropicConfig {
            api_key: cfg.api_key.clone(),
            base_url: if cfg.base_url.is_empty() {
                self.default_base_url().to_string()
            } else {
                cfg.base_url.clone()
            },
            model: model_id.to_string(),
            timeout_secs: cfg.timeout_secs,
            provider_kind: Some(crate::api::ProviderKind::Anthropic),
            reasoning_effort: cfg.reasoning_effort.clone(),
            // 段名（providers.toml 段限定键）→ 模型级 max_tokens 精确解析
            provider_segment: cfg.name.clone(),
            // 段级自定义标头（anthropic 兼容中转实例）；官方段为空 → 零生效
            extra_headers: cfg
                .extra_headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }))
    }
}
