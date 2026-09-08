use crate::config::provider::*;
use crate::transports::chat_completions::{ChatCompletionsConfig, ChatCompletionsTransport};
use crate::transports::Transport;
use std::sync::Arc;

pub struct OpenCodeGoProvider;

impl Provider for OpenCodeGoProvider {
    fn id(&self) -> &'static str {
        "opencode-go"
    }
    fn display_name(&self) -> &'static str {
        "OpenCode Go 套餐"
    }
    fn default_base_url(&self) -> &'static str {
        "https://opencode.ai/zen/go/v1"
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
                id: "qwen3.7-max",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 65_536,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 2.5,
                cost_per_million_out: 7.5,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "longcat-2.0",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 131_072,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.3,
                cost_per_million_out: 1.2,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "deepseek-v4-flash-vision-exp",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 384_000,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.22,
                cost_per_million_out: 0.66,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "qwen3.6-plus",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 65_536,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.5,
                cost_per_million_out: 3.0,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "muse-spark-1.2-contributor",
                aliases: &[],
                context_window: 1_048_576,
                max_output_tokens: 131_072,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.1,
                cost_per_million_out: 0.2,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "minimax-m2.7",
                aliases: &[],
                context_window: 204_800,
                max_output_tokens: 131_072,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.3,
                cost_per_million_out: 1.2,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "kimi-k2.6",
                aliases: &[],
                context_window: 262_144,
                max_output_tokens: 65_536,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.95,
                cost_per_million_out: 4.0,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "glm-5.2",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 131_072,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 1.4,
                cost_per_million_out: 4.4,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "minimax-m2.5",
                aliases: &[],
                context_window: 204_800,
                max_output_tokens: 65_536,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.3,
                cost_per_million_out: 1.2,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "minimax-m3",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 131_072,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.3,
                cost_per_million_out: 1.2,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "deepseek-v4-flash",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 384_000,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.22,
                cost_per_million_out: 0.66,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "kimi-k2.7-code",
                aliases: &[],
                context_window: 262_144,
                max_output_tokens: 262_144,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.95,
                cost_per_million_out: 4.0,
                reasoning_field: "",
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
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "ox-alpha-free",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 131_072,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.0,
                cost_per_million_out: 0.0,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "hy3",
                aliases: &[],
                context_window: 256_000,
                max_output_tokens: 128_000,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.14,
                cost_per_million_out: 0.58,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "hy4-preview",
                aliases: &[],
                context_window: 1_024_000,
                max_output_tokens: 64_000,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.834,
                cost_per_million_out: 2.501,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "omen-alpha",
                aliases: &[],
                context_window: 500_000,
                max_output_tokens: 128_000,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.2,
                cost_per_million_out: 0.66,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "gpt-5.6-luna",
                aliases: &[],
                context_window: 1_050_000,
                max_output_tokens: 128_000,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.2,
                cost_per_million_out: 1.2,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "kimi-k3",
                aliases: &[],
                context_window: 1_048_576,
                max_output_tokens: 131_072,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 3.0,
                cost_per_million_out: 15.0,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "glm-5.3-flash",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 131_072,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.075,
                cost_per_million_out: 0.25,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "muse-spark-1.3-contributor",
                aliases: &[],
                context_window: 1_048_576,
                max_output_tokens: 131_072,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.1,
                cost_per_million_out: 0.2,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "mimo-v2-pro",
                aliases: &[],
                context_window: 1_048_576,
                max_output_tokens: 128_000,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 1.0,
                cost_per_million_out: 3.0,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "qwen3.8-flash",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 131_072,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.15,
                cost_per_million_out: 0.47,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "grok-4.6",
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
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "glm-5",
                aliases: &[],
                context_window: 202_752,
                max_output_tokens: 32_768,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 1.0,
                cost_per_million_out: 3.2,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "mimo-v2-omni",
                aliases: &[],
                context_window: 262_144,
                max_output_tokens: 128_000,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.4,
                cost_per_million_out: 2.0,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "qwen3.8-max",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 131_072,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 2.0,
                cost_per_million_out: 6.0,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "kimi-k2.5",
                aliases: &[],
                context_window: 262_144,
                max_output_tokens: 65_536,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.6,
                cost_per_million_out: 3.0,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "glm-5.1",
                aliases: &[],
                context_window: 202_752,
                max_output_tokens: 32_768,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 1.4,
                cost_per_million_out: 4.4,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "qwen3.7-plus",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 65_536,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.4,
                cost_per_million_out: 1.6,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "deepseek-v4-pro",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 384_000,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.66,
                cost_per_million_out: 1.98,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "glm-5.3",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 131_072,
                supports_streaming: true,
                supports_vision: false,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 1.4,
                cost_per_million_out: 4.4,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "mimo-v2.5",
                aliases: &[],
                context_window: 1_000_000,
                max_output_tokens: 128_000,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.14,
                cost_per_million_out: 0.28,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "mimo-v2.5-pro",
                aliases: &[],
                context_window: 1_048_576,
                max_output_tokens: 128_000,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.435,
                cost_per_million_out: 0.87,
                reasoning_field: "",
                reasoning_efforts: &[],
                default_effort: None,
            },
            ModelDef {
                id: "qwen3.5-plus",
                aliases: &[],
                context_window: 262_144,
                max_output_tokens: 65_536,
                supports_streaming: true,
                supports_vision: true,
                supports_reasoning: true,
                supports_audio: false,
                supports_image_generation: false,
                cost_per_million_in: 0.2,
                cost_per_million_out: 1.2,
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
            content_tool_tags: &[],
            cache_hit_field: "",
        }
    }

    /// 按模型族分派协议（refactor 设计 §4.1/§7 P4）。
    ///
    /// responses 族（models.dev 实测 /responses 200、/chat 500 → 走 Responses 引擎）；
    /// 其余全部落回 ChatCompletions（默认，opencode-go transport() 继续承载）。
    ///
    /// R4 预留：minimax-*/qwen* 官方标 anthropic /messages，因未真机验证
    /// /messages 是否比 chat 稳（设计 R4），本轮不切。真机验证 responses 通路后，
    /// 在下方 match 加一行即可启用，结构已支持：
    /// ```text
    /// s if s.starts_with("minimax-") || s.starts_with("qwen") => TransportKind::Anthropic,
    /// ```
    fn transport_for(&self, model_id: &str) -> TransportKind {
        match model_id {
            // responses 族精确成员（与 models() 实有 id 一致，见分派表交付清单）
            "gpt-5.6-luna" | "grok-4.5" | "grok-4.6" => TransportKind::Responses,
            // muse-spark 系列按前缀归族，新成员免改表
            s if s.starts_with("muse-spark-") => TransportKind::Responses,
            // R4 预留（见上注释）：minimax-*/qwen* → Anthropic 暂不启用
            _ => TransportKind::ChatCompletions,
        }
    }

    fn transport(&self, cfg: &ProviderConfig, model_id: &str) -> Arc<dyn Transport> {
        // OpenCode Go 网关要求每对话稳定 x-opencode-session（400 MissingSessionID
        // 根因）。在 quirks 副本上注入会话 ID，静态 self.quirks() 保持原样（有测试
        // 与语义约束，也避免影响 Responses 路径取到的静态表）。
        let mut quirks = self.quirks();
        quirks.extra_headers.push((
            "x-opencode-session".to_string(),
            crate::transports::opencode_session::opencode_session_id(),
        ));
        Arc::new(ChatCompletionsTransport::new(ChatCompletionsConfig {
            name: "opencode-go".to_string(),
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
            provider_kind: Some(crate::api::ProviderKind::OpenCodeGo),
            quirks,
            reasoning_effort: cfg.reasoning_effort.clone(),
        }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// responses 族精确成员 + muse-spark 前缀命中 → Responses。
    /// 断言 id 全部取自 models() 实有 id（35 全量，防分派表与模型表脱钩）。
    #[test]
    fn transport_for_responses_family() {
        let p = OpenCodeGoProvider;
        for id in [
            "gpt-5.6-luna",
            "grok-4.5",
            "grok-4.6",
            "muse-spark-1.2-contributor",
            "muse-spark-1.3-contributor",
        ] {
            assert_eq!(
                p.transport_for(id),
                TransportKind::Responses,
                "model {id} must route to Responses"
            );
        }
    }

    /// 其余模型族 → ChatCompletions（默认），取 models() 实有 id 断言。
    #[test]
    fn transport_for_chat_family_stays_chat() {
        let p = OpenCodeGoProvider;
        for id in [
            "glm-5.3-flash",
            "kimi-k3",
            "minimax-m3",
            "deepseek-v4-flash",
        ] {
            assert_eq!(
                p.transport_for(id),
                TransportKind::ChatCompletions,
                "model {id} must stay ChatCompletions"
            );
        }
    }

    /// default_transport 保持 ChatCompletions（不覆写）。
    #[test]
    fn default_transport_is_chat() {
        assert_eq!(
            OpenCodeGoProvider.default_transport(),
            TransportKind::ChatCompletions
        );
    }

    /// 分派表覆盖范围检查：models() 里所有 id 都应落入 responses 或 chat 族，
    /// 不允许有第 4 种结果（防止未来 models() 新增 id 静默落入错误分支后无人知）。
    #[test]
    fn dispatch_table_covers_all_models() {
        let p = OpenCodeGoProvider;
        for m in p.models() {
            let kind = p.transport_for(m.id);
            assert!(
                matches!(
                    kind,
                    TransportKind::Responses | TransportKind::ChatCompletions
                ),
                "model {} must dispatch to Responses or ChatCompletions, got {:?}",
                m.id,
                kind
            );
        }
    }
}