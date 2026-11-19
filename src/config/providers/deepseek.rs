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
        "deepseek-flash"
    }

    fn models(&self) -> &'static [ModelDef] {
        &[
            ModelDef {
                id: "deepseek-flash",
                // 官方已把 deepseek-v4-flash 正名为 deepseek-flash；vision-exp
                // 是同一模型的实验名（服务端对旧名静默映射到 deepseek-flash）。
                // 旧名保留在 aliases，已落盘旧名的配置仍能命中能力元数据。
                aliases: &[
                    "deepseek",
                    "default",
                    "deepseek-v4-flash",
                    "deepseek-v4-flash-vision-exp",
                    "deepseek-vision",
                    "flash-vision",
                ],
                context_window: 1_000_000,
                max_output_tokens: 8_192,
                supports_streaming: true,
                // 视觉实测可读图（image_url + data:image/png;base64），与旧条目
                // 标注的 false 不符——同一模型此前的 vision-exp 条目标注才是对的。
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                supports_video: false,
                supports_tool_calling: true,
                reasoning_required: false,
                cost_per_million_in: 0.07,
                cost_per_million_out: 0.28,
                reasoning_field: "reasoning_content",
                // 官方依据（api-docs.deepseek.com/guides/thinking_mode，2026-10-10 核对）：
                // · OpenAI 格式 Thinking Effort Control = `{"reasoning_effort": "low/high/max"}`；
                // · 映射表证实仅三个**实际生效**档：minimal/low→low、medium/high/xhigh→high、
                //   max/ultra→max。
                // UI 档位 = 官方生效档，不发明映射冗余档（medium 只是 high 的别名）。
                reasoning_efforts: &["low", "high", "max"],
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
                supports_video: false,
                supports_tool_calling: true,
                reasoning_required: false,
                cost_per_million_in: 0.27,
                cost_per_million_out: 1.10,
                reasoning_field: "reasoning_content",
                // 同上：官方 OpenAI 格式三档 low/high/max（映射表外的值均为别名映射）。
                reasoning_efforts: &["low", "high", "max"],
                default_effort: Some("high"),
            },
        ]
    }

    fn quirks(&self) -> ProviderQuirks {
        ProviderQuirks {
            requires_reasoning_echo: true,
            supports_reasoning_effort: true,
            // 官方依据（thinking_mode 页 · Tool Calls 示例）：run_turn 内
            // `tools=tools, reasoning_effort="high", extra_body={"thinking": ...}`
            // 三者同发且正常 —— thinking 模式本就支持工具调用。
            // 曾置 true 让 Agent 主循环（永远带 tools）静默丢失档位 → 服务端恒走
            // 默认 high，UI 换档零行为差异；按官方文档改为同发。
            effort_excludes_tools: false,
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
            provider_kind: Some(crate::api::ProviderKind::DeepSeek),
            quirks: self.quirks(),
            reasoning_effort: cfg.reasoning_effort.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::provider::Provider;

    #[test]
    fn efforts_match_official_thinking_mode_docs() {
        // 官方（api-docs.deepseek.com/guides/thinking_mode，2026-10-10 核对）：
        // OpenAI 格式控制参数 `{"reasoning_effort": "low/high/max"}`；映射表证实
        // 仅这三个实际生效档（minimal/low→low，medium/high/xhigh→high，max/ultra→max）。
        for model in DeepSeekProvider.models() {
            assert_eq!(
                model.reasoning_efforts.to_vec(),
                vec!["low", "high", "max"],
                "{}: 档位必须是官方生效三档",
                model.id
            );
            assert_eq!(
                model.default_effort,
                Some("high"),
                "{}: 官方默认 effort 为 high",
                model.id
            );
        }
    }

    #[test]
    fn effort_not_excluded_with_tools_per_official_sample() {
        // 官方 Tool Calls 示例：tools + reasoning_effort="high" 同发正常
        // （thinking 模式支持工具调用）→ Agent 主循环的档位必须实际生效。
        let quirks = DeepSeekProvider.quirks();
        assert!(
            quirks.supports_reasoning_effort,
            "supports_reasoning_effort 必须保持 true"
        );
        assert!(
            !quirks.effort_excludes_tools,
            "带 tools 请求不得抑制 reasoning_effort（官方同发示例）"
        );
    }
}
