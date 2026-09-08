use crate::config::provider::*;
use crate::transports::chat_completions::{ChatCompletionsConfig, ChatCompletionsTransport};
use crate::transports::Transport;
use std::sync::Arc;

/// xAI（Grok）— OpenAI 兼容的 Chat Completions 端点。
///
/// 模型规格取自 models.dev（2026-09 快照）：grok-4.6 / grok-4.5 / grok-4.3 /
/// grok-build-0.1。上下文与价格以官方发布为准，用户可在模型页刷新列表覆盖。
pub struct XaiProvider;

impl Provider for XaiProvider {
    fn id(&self) -> &'static str {
        "xai"
    }
    fn display_name(&self) -> &'static str {
        "xAI"
    }
    fn default_base_url(&self) -> &'static str {
        "https://api.x.ai/v1"
    }
    fn auth_header(&self) -> &'static str {
        "authorization"
    }
    fn auth_prefix(&self) -> &'static str {
        "Bearer "
    }
    fn default_model(&self) -> &'static str {
        "grok-4.6"
    }

    fn models(&self) -> &'static [ModelDef] {
        &[
            ModelDef {
                id: "grok-4.6",
                aliases: &["grok", "default"],
                context_window: 500_000,
                max_output_tokens: 500_000,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 2.0,
                cost_per_million_out: 6.0,
                reasoning_field: "reasoning_content",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "grok-4.5",
                aliases: &[],
                context_window: 500_000,
                max_output_tokens: 500_000,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 2.0,
                cost_per_million_out: 6.0,
                reasoning_field: "reasoning_content",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "grok-4.3",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 30_000,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 1.25,
                cost_per_million_out: 2.5,
                reasoning_field: "reasoning_content",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "grok-build-0.1",
                aliases: &[],
                context_window: 256_000,
                max_output_tokens: 256_000,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 1.0,
                cost_per_million_out: 2.0,
                reasoning_field: "reasoning_content",
                reasoning_efforts: &[],
                default_effort: None,
            },
        ]
    }

    fn transport(&self, cfg: &ProviderConfig, model_id: &str) -> Arc<dyn Transport> {
        Arc::new(ChatCompletionsTransport::new(ChatCompletionsConfig {
            name: "xai".to_string(),
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
            provider_kind: Some(crate::api::ProviderKind::Xai),
            quirks: self.quirks(),
            reasoning_effort: cfg.reasoning_effort.clone(),
        }))
    }
}
