use crate::config::provider::*;
use crate::transports::chat_completions::{ChatCompletionsConfig, ChatCompletionsTransport};
use crate::transports::Transport;
use std::sync::Arc;

/// StepFun（阶跃星辰）——OpenAI 兼容 Chat Completions。
///
/// 双通道（官方文档 chat-completion-create / step-plan/overview）：
/// - API Key（按量计费）：`https://api.stepfun.com/v1`
/// - Step Plan（订阅套餐）：`https://api.stepfun.com/step_plan/v1`
///
/// 两条通道共用同一把 Step API Key（Bearer），仅 base_url 不同——默认
/// base_url 取 API Key 通道；Step Plan 用户在「接口地址」里填 step_plan
/// 前缀即可切通道（前端帮助文案已写明），无需单独的提供商条目。
/// `step-router-v1` 仅 Step Plan 通道可用，故不进内置模型清单。
pub struct StepFunProvider;

impl Provider for StepFunProvider {
    fn id(&self) -> &'static str {
        "stepfun"
    }
    fn display_name(&self) -> &'static str {
        "StepFun"
    }
    fn default_base_url(&self) -> &'static str {
        "https://api.stepfun.com/v1"
    }
    fn auth_header(&self) -> &'static str {
        "authorization"
    }
    fn auth_prefix(&self) -> &'static str {
        "Bearer "
    }
    fn default_model(&self) -> &'static str {
        "step-3.7-flash"
    }

    fn models(&self) -> &'static [ModelDef] {
        &[
            // 规格（上下文/输出上限/能力/推理档位）取自官方模型页 2026-10 快照：
            // step-5-preview 1M/64k；step-3.7-flash 256K，low/medium/high；
            // step-3.5-flash 256K，-2603 变体支持 reasoning_effort(low/high)。
            ModelDef {
                id: "step-5-preview",
                aliases: &["stepfun", "default", "step-5"],
                context_window: 1_000_000,
                max_output_tokens: 65_536,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.0,
                cost_per_million_out: 0.0,
                reasoning_field: "",
                reasoning_efforts: &["low", "medium", "high"],
                default_effort: None,
            },
            ModelDef {
                id: "step-3.7-flash",
                aliases: &["step-3.7"],
                context_window: 256_000,
                max_output_tokens: 8_192,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.0,
                cost_per_million_out: 0.0,
                reasoning_field: "",
                reasoning_efforts: &["low", "medium", "high"],
                default_effort: None,
            },
            ModelDef {
                id: "step-3.5-flash",
                aliases: &["step-3.5", "step-3.5-flash-2603"],
                context_window: 256_000,
                max_output_tokens: 8_192,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.0,
                cost_per_million_out: 0.0,
                reasoning_field: "",
                reasoning_efforts: &["low", "medium", "high"],
                default_effort: None,
            },
        ]
    }

    fn quirks(&self) -> ProviderQuirks {
        ProviderQuirks {
            requires_reasoning_echo: false,
            supports_reasoning_effort: true,
            effort_excludes_tools: false,
            sanitize_tools: None,
            extra_headers: vec![],
            forbidden_request_fields: &[],
            max_tokens_field: MaxTokensField::MaxTokens,
            user_agent: None,
            content_tool_tags: &[],
            // StepFun 的缓存命中读数在 usage **顶层** `cached_tokens`（官方
            // prompt-cache 文档：「response.usage.cached_tokens 大于 0 则表明
            // 命中缓存」），不在 OpenAI 标准位 `prompt_tokens_details.cached_tokens`。
            // 留空（走标准位兜底）时 read_cache_hit 恒 0——step-5-preview 等型号
            // 本就支持 Prompt 缓存（>256 token 自动启用），ctx 弹窗会把「无读数」
            // 显示成「缓存命中 0%」的假数（2026-11-03 大王实测）。
            cache_hit_field: "cached_tokens",
        }
    }

    fn transport(&self, cfg: &ProviderConfig, model_id: &str) -> Arc<dyn Transport> {
        Arc::new(ChatCompletionsTransport::new(ChatCompletionsConfig {
            name: "stepfun".to_string(),
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
            provider_kind: Some(crate::api::ProviderKind::StepFun),
            quirks: self.quirks(),
            reasoning_effort: cfg.reasoning_effort.clone(),
        }))
    }
}
