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
        "Google"
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
            supports_video: false,
            supports_tool_calling: true,
            reasoning_required: false,
            cost_per_million_in: 0.0,
            cost_per_million_out: 0.0,
            reasoning_field: "",
            // 官方依据（ai.google.dev · OpenAI compatibility，2026-10 核对）：
            // 兼容端点接受 reasoning_effort，映射表 minimal/low→low、
            // medium→medium、high→high（2.5 系再映射到 thinking_budget）。
            // gemini-2.5-pro **不允许** "none"（思考不可关）→ 只列三档；不指定
            // 时用模型官方默认，故 default_effort 留空。
            reasoning_efforts: &["low", "medium", "high"],
            default_effort: None,
        }]
    }

    fn quirks(&self) -> ProviderQuirks {
        ProviderQuirks {
            requires_reasoning_echo: false,
            // 官方 OpenAI 兼容端点接受 reasoning_effort（见 models() 注释的依据）。
            supports_reasoning_effort: true,
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
            // 协议/类型 id —— Transport::provider_name 的语义（每个 custom 实例都折叠成 "custom"）
            // providers.toml 段名 —— 模型级配置（max_tokens 等）必须按段精确查找，
            // 用类型 id 查会静默查不到（段名是 custom-xxx 而非 custom）。
            provider_segment: cfg.name.clone(),
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
