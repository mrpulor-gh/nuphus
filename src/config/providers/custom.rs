use crate::config::provider::*;
use crate::transports::chat_completions::{ChatCompletionsConfig, ChatCompletionsTransport};
use crate::transports::Transport;
use std::sync::Arc;

pub struct CustomProvider;

impl Provider for CustomProvider {
    fn id(&self) -> &'static str {
        "custom"
    }
    fn display_name(&self) -> &'static str {
        "自定义 (OpenAI 兼容 / 中转站 API)"
    }
    fn default_base_url(&self) -> &'static str {
        "https://your-custom-api.com/v1"
    }
    fn auth_header(&self) -> &'static str {
        "authorization"
    }
    fn auth_prefix(&self) -> &'static str {
        "Bearer "
    }
    fn default_model(&self) -> &'static str {
        "custom-model"
    }

    fn models(&self) -> &'static [ModelDef] {
        &[ModelDef {
            id: "custom-model",
            aliases: &["custom", "default"],
            context_window: 128_000,
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
        }]
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

    fn transport(&self, cfg: &ProviderConfig, model_id: &str) -> Arc<dyn Transport> {
        Arc::new(ChatCompletionsTransport::new(ChatCompletionsConfig {
            name: "custom".to_string(),
            api_key: cfg.api_key.clone(),
            base_url: if cfg.base_url.is_empty() {
                self.default_base_url().to_string()
            } else {
                cfg.base_url.clone()
            },
            model: model_id.to_string(),
            timeout_secs: cfg.timeout_secs,
            auth_header: if cfg.auth_header.is_empty() {
                self.auth_header().to_string()
            } else {
                cfg.auth_header.clone()
            },
            auth_prefix: if cfg.auth_prefix.is_empty() {
                self.auth_prefix().to_string()
            } else {
                cfg.auth_prefix.clone()
            },
            provider_kind: Some(crate::api::ProviderKind::Custom),
            quirks: self.quirks(),
            reasoning_effort: cfg.reasoning_effort.clone(),
        }))
    }
}
