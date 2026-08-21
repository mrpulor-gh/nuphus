use crate::config::provider::*;
use crate::transports::chat_completions::{ChatCompletionsConfig, ChatCompletionsTransport};
use crate::transports::Transport;
use std::sync::Arc;

pub struct GoogleProvider;

impl Provider for GoogleProvider {
    fn id(&self) -> &'static str {
        "google"
    }
    fn display_name(&self) -> &'static str {
        "Google (Gemini)"
    }
    fn default_base_url(&self) -> &'static str {
        "https://generativelanguage.googleapis.com/v1beta"
    }
    fn auth_header(&self) -> &'static str {
        "x-goog-api-key"
    }
    fn auth_prefix(&self) -> &'static str {
        ""
    }
    fn default_model(&self) -> &'static str {
        "gemini-2.5-pro"
    }

    fn models(&self) -> &'static [ModelDef] {
        &[ModelDef {
            id: "gemini-2.5-pro",
            aliases: &["google", "default"],
            context_window: 2_000_000,
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
            name: "google".to_string(),
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
            provider_kind: Some(crate::api::ProviderKind::Google),
            quirks: self.quirks(),
            reasoning_effort: cfg.reasoning_effort.clone(),
        }))
    }
}
