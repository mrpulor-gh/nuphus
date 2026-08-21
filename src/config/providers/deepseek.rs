use crate::config::provider::*;
use crate::transports::chat_completions::{ChatCompletionsConfig, ChatCompletionsTransport};
use crate::transports::Transport;
use std::sync::Arc;

pub struct DeepSeekProvider;

impl Provider for DeepSeekProvider {
    fn id(&self) -> &'static str {
        "deepseek"
    }
    fn display_name(&self) -> &'static str {
        "DeepSeek"
    }
    fn default_base_url(&self) -> &'static str {
        "https://api.deepseek.com"
    }
    fn auth_header(&self) -> &'static str {
        "authorization"
    }
    fn auth_prefix(&self) -> &'static str {
        "Bearer "
    }
    fn default_model(&self) -> &'static str {
        "deepseek-v4-flash"
    }

    fn models(&self) -> &'static [ModelDef] {
        &[
            ModelDef {
                id: "deepseek-v4-flash",
                aliases: &["deepseek", "default"],
                context_window: 1_000_000,
                max_output_tokens: 8_192,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.07,
                cost_per_million_out: 0.28,
                reasoning_field: "reasoning_content",
                reasoning_efforts: &["high", "max"],
                default_effort: Some("high"),
            },
            ModelDef {
                id: "deepseek-v4-pro",
                aliases: &["deepseek-pro"],
                context_window: 1_000_000,
                max_output_tokens: 8_192,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.27,
                cost_per_million_out: 1.10,
                reasoning_field: "reasoning_content",
                reasoning_efforts: &["high", "max"],
                default_effort: Some("high"),
            },
        ]
    }

    fn quirks(&self) -> ProviderQuirks {
        ProviderQuirks {
            requires_reasoning_echo: true,
            supports_reasoning_effort: true,
            effort_excludes_tools: true,
            sanitize_tools: None,
            extra_headers: vec![],
            forbidden_request_fields: &[],
            max_tokens_field: MaxTokensField::MaxTokens,
            user_agent: None,
            content_tool_tags: &[],
            cache_hit_field: "prompt_cache_hit_tokens",
        }
    }

    fn transport(&self, cfg: &ProviderConfig, model_id: &str) -> Arc<dyn Transport> {
        Arc::new(ChatCompletionsTransport::new(ChatCompletionsConfig {
            name: "deepseek".to_string(),
            api_key: cfg.api_key.clone(),
            base_url: if cfg.base_url.is_empty() {
                self.default_base_url().to_string()
            } else {
                cfg.base_url.clone()
            },
            model: model_id.to_string(),
            timeout_secs: cfg.timeout_secs,
            auth_header: self.auth_header().to_string(),
            auth_prefix: self.auth_prefix().to_string(),
            provider_kind: Some(crate::api::ProviderKind::DeepSeek),
            quirks: self.quirks(),
            reasoning_effort: cfg.reasoning_effort.clone(),
        }))
    }
}
