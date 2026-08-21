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
                reasoning_efforts: &["none", "low", "high", "max"],
                default_effort: None,
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
                reasoning_efforts: &["none", "low", "high", "max"],
                default_effort: None,
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
            supports_reasoning_effort: false,
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
        }))
    }
}
