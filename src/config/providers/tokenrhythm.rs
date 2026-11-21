use crate::config::provider::*;
use crate::transports::chat_completions::{ChatCompletionsConfig, ChatCompletionsTransport};
use crate::transports::Transport;
use std::sync::Arc;

/// 基元律动（TokenRhythm）—— 对标 OpenRouter 的多模型聚合平台（用户口中的「中转站」）。
///
/// ## 数据来源
/// 2026-11-21 实测拉取 `GET https://tokenrhythm.studio/api/models`（公开、无需鉴权，
/// 目录共 32 条）。字段映射逐条取自目录，不做推断：
///
/// | 基元目录字段 | ModelDef 字段 |
/// |---|---|
/// | `supportsTools` | `supports_tool_calling`（28 个 chat 模型全 true） |
/// | `capabilities.video` / `modalities` 含 `video` | `supports_video`（8 个 chat 模型；目录 video=true 共 9 个，另 1 个是视频生成模型 `wan3.0-video`，不进本表） |
/// | `reasoningMode == "required"` | `reasoning_required`（7 个；optional 21 / unsupported 4） |
/// | `reasoningSupportedEfforts` | `reasoning_efforts`（12 个非空，其余不暴露分级） |
/// | `contextWindow` / `maxOutputTokens` | 同名 |
/// | `supportsVision` / `supportsReasoning` | 同名 |
///
/// 只内置 `type == "chat"` 的 28 个模型：`qwen-image-2.0`、`wan2.7-image`（图像）、
/// `wan3.0-video`（视频）、`NeoHorse-Jev-4B`（决策）不走 Chat Completions，进本表
/// 会污染聊天模型选择器。
///
/// ## 未验证边界（务必如实告知用户）
/// 本机没有基元 API Key，**全部字段来自官方目录，未做任何真实请求验证**。
/// 目录会随上游增删模型，本表是 2026-11-21 的快照，变动需人工同步。
///
/// ## 价格
/// 目录给的是 CNY / 百万 token，而 `ModelDef.cost_per_million_in/out` 契约是 USD。
/// 口径不同不做换算（按 7.1 折算会把人民币当美元显示，差一个数量级），统一填 0.0
/// = 未知，前端不展示价格。
pub struct TokenRhythmProvider;

/// 目录快照：`type == "chat"` 的 28 个模型（`effectiveAvailable` 全为 true）。
///
/// 状态备注（目录 `status` 字段）：`hy4`、`neohorse-1.5-flash-preview`、
/// `egg-alpha-003` 为 testing，`qwen4-max` 为 special_offer，其余 24 个 online。
/// 全部保留——目录对当前访问者均标记为可用，是否选用交给用户。
const MODELS: &[ModelDef] = &[
    m("glm-5.1", 200_000, 128_000, false, false, false, &[]),
    m("minimax-m2.7", 200_000, 192_000, false, false, true, &[]),
    m("kimi-k2.6", 256_000, 128_000, true, false, false, &[]),
    m("qwen3.7-max", 1_000_000, 131_072, false, false, false, &[]),
    m("kimi-k2.7-code", 256_000, 16_000, true, false, true, &[]),
    m("glm-5.2", 1_000_000, 128_000, false, false, false, &[]),
    m(
        "qwen3.8-max",
        1_000_000,
        131_072,
        true,
        false,
        false,
        &["low", "medium", "xhigh"],
    ),
    m(
        "deepseek-v4-flash-0731",
        1_000_000,
        384_000,
        false,
        false,
        false,
        &["low", "high", "max"],
    ),
    m("seed-2.1-turbo", 262_144, 131_072, true, true, false, &[]),
    m("seed-2.1-pro", 262_144, 131_072, true, true, false, &[]),
    m(
        "deepseek-v4-pro-0813",
        1_000_000,
        384_000,
        false,
        false,
        false,
        &["low", "high", "max"],
    ),
    m(
        "glm-5.3",
        1_048_576,
        131_072,
        false,
        false,
        true,
        &["low", "high", "max"],
    ),
    m("qwen3.7-flash", 1_000_000, 131_072, true, true, false, &[]),
    m(
        "qwen3.8-27b",
        1_000_000,
        131_072,
        true,
        true,
        false,
        &["low", "medium", "xhigh"],
    ),
    m("kimi-k3", 1_048_576, 1_048_576, true, true, false, &[]),
    m("longcat-2.0", 1_000_000, 128_000, false, false, true, &[]),
    m("glm-5.3-flash", 1_048_576, 131_072, true, false, true, &[]),
    m(
        "qwen3.8-flash",
        1_000_000,
        131_072,
        true,
        true,
        false,
        &["low", "medium", "xhigh"],
    ),
    m(
        "deepseek-flash",
        1_000_000,
        384_000,
        true,
        false,
        false,
        &["low", "high", "max"],
    ),
    m(
        "glm-5.3-flashx",
        1_048_576,
        131_072,
        true,
        false,
        true,
        &["low", "high", "max"],
    ),
    m("mimo-v2.6-pro", 1_048_576, 131_072, true, false, false, &[]),
    m(
        "mimo-v2.6-flash",
        1_048_576,
        131_072,
        true,
        false,
        false,
        &[],
    ),
    m(
        "deepseek-v4.1-flash-1017",
        1_000_000,
        384_000,
        true,
        false,
        false,
        &["low", "high", "max"],
    ),
    m(
        "minimax-m3.1-flash",
        1_000_000,
        524_288,
        true,
        true,
        true,
        &["low", "medium", "high", "xhigh", "max"],
    ),
    m("hy4", 1_048_576, 65_536, true, false, false, &["high"]),
    m(
        "neohorse-1.5-flash-preview",
        1_048_576,
        128_000,
        true,
        false,
        false,
        &[],
    ),
    m(
        "egg-alpha-003",
        1_000_000,
        192_000,
        false,
        false,
        false,
        &[],
    ),
    m(
        "qwen4-max",
        1_000_000,
        262_144,
        true,
        true,
        false,
        &["low", "high", "xhigh"],
    ),
];

/// 目录条目 → ModelDef。只承载**目录里有据**的差异位，其余取基元全量 chat 模型的
/// 共同事实：流式、推理、工具调用全支持，音频 / 图像生成全不支持。
const fn m(
    id: &'static str,
    context_window: u32,
    max_output_tokens: u32,
    vision: bool,
    video: bool,
    required: bool,
    efforts: &'static [&'static str],
) -> ModelDef {
    ModelDef {
        id,
        aliases: &[],
        context_window,
        max_output_tokens,
        supports_streaming: true,
        supports_vision: vision,
        supports_reasoning: true,
        supports_audio: false,
        supports_image_generation: false,
        supports_video: video,
        supports_tool_calling: true,
        reasoning_required: required,
        cost_per_million_in: 0.0,
        cost_per_million_out: 0.0,
        reasoning_field: "",
        reasoning_efforts: efforts,
        default_effort: None,
    }
}

impl Provider for TokenRhythmProvider {
    fn id(&self) -> &'static str {
        "tokenrhythm"
    }
    fn display_name(&self) -> &'static str {
        "TokenRhythm（基元律动）"
    }
    fn default_base_url(&self) -> &'static str {
        "https://tokenrhythm.studio/v1"
    }
    fn auth_header(&self) -> &'static str {
        "authorization"
    }
    fn auth_prefix(&self) -> &'static str {
        "Bearer "
    }
    fn default_model(&self) -> &'static str {
        "deepseek-v4.1-flash-1017"
    }

    fn models(&self) -> &'static [ModelDef] {
        MODELS
    }

    fn quirks(&self) -> ProviderQuirks {
        ProviderQuirks {
            requires_reasoning_echo: false,
            // 目录里 12 个模型显式声明了 reasoningSupportedEfforts（low/high/max 等），
            // 说明该网关接受 `reasoning_effort`。开闸是安全的：transport 只在用户
            // **显式**配置了 reasoning_effort 时才下发，而 UI 的档位选择器只在
            // `reasoning_efforts` 非空时出现——没声明档位的模型不会带这个字段。
            supports_reasoning_effort: true,
            // 无互斥证据：基元文档只要求 DeepSeek 的 tool_choice 用 none/auto/required
            // （不得传对象形式），未声明 effort 与 tools 冲突。
            effort_excludes_tools: false,
            sanitize_tools: None,
            extra_headers: vec![],
            forbidden_request_fields: &[],
            max_tokens_field: MaxTokensField::MaxTokens,
            user_agent: None,
            content_tool_tags: &[],
            // 目录 cacheRead 字段存在，但 OpenAI 协议下缓存命中走标准路径
            // `usage.prompt_tokens_details.cached_tokens`，无需自定义字段名。
            cache_hit_field: "",
        }
    }

    fn transport(&self, cfg: &ProviderConfig, model_id: &str) -> Arc<dyn Transport> {
        // 段级自定义标头并入 quirks.extra_headers：与 custom 段同一条注入链。
        let mut quirks = self.quirks();
        quirks.extra_headers.extend(
            cfg.extra_headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        Arc::new(ChatCompletionsTransport::new(ChatCompletionsConfig {
            name: "tokenrhythm".to_string(),
            // 协议/类型 id vs providers.toml 段名：模型级配置（max_tokens 等）必须按
            // 段名精确查找，用类型 id 查会静默查不到。
            provider_segment: cfg.name.clone(),
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
            provider_kind: Some(crate::api::ProviderKind::TokenRhythm),
            quirks,
            reasoning_effort: cfg.reasoning_effort.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目录快照的规模与唯一性：28 个 chat 模型，id 不得重复。
    #[test]
    fn publishes_28_unique_chat_models() {
        let models = TokenRhythmProvider.models();
        assert_eq!(models.len(), 28, "目录 chat 模型数应保持 28");
        let mut ids: Vec<&str> = models.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 28, "模型 id 必须唯一");
    }

    /// 默认模型必须在内置表内，否则空 model 配置会打到不存在的模型。
    #[test]
    fn default_model_is_published() {
        let default = TokenRhythmProvider.default_model();
        assert!(
            TokenRhythmProvider.models().iter().any(|m| m.id == default),
            "default_model {default} 不在模型表内"
        );
    }

    /// 三个能力位的真实分布（2026-11-21 目录快照，只统计 chat 模型）：
    /// 视频输入 8、强制思考 7、工具调用 28 全开、视觉输入 19、声明推理档位 12。
    /// 这几个数直接对应 agent 层的请求闸门与 UI chip，改动模型表必须同步更新
    /// 本断言——防止快照被悄悄改错。
    #[test]
    fn capability_distribution_matches_catalog_snapshot() {
        let models = TokenRhythmProvider.models();
        let video = models.iter().filter(|m| m.supports_video).count();
        let required = models.iter().filter(|m| m.reasoning_required).count();
        let tools = models.iter().filter(|m| m.supports_tool_calling).count();
        let vision = models.iter().filter(|m| m.supports_vision).count();
        let with_efforts = models
            .iter()
            .filter(|m| !m.reasoning_efforts.is_empty())
            .count();
        assert_eq!(video, 8, "视频输入模型数（chat 模型）");
        assert_eq!(required, 7, "强制思考模型数");
        assert_eq!(tools, 28, "支持工具调用的模型数");
        assert_eq!(vision, 19, "视觉输入模型数");
        assert_eq!(with_efforts, 12, "声明推理档位的模型数");
        // 视频输入的 8 个模型必然同时支持图片输入（目录 modalities 含 image）
        for m in models.iter().filter(|m| m.supports_video) {
            assert!(m.supports_vision, "{} 支持视频却不支持图片", m.id);
        }
        // 强制思考的模型不得声明空档位以外的矛盾配置：required 与 efforts 无必然关系，
        // 但 required 必须同时 supports_reasoning。
        for m in models.iter().filter(|m| m.reasoning_required) {
            assert!(m.supports_reasoning, "{} 强制思考却不支持推理", m.id);
        }
    }

    /// 价格口径：目录是 CNY，ModelDef 契约是 USD，未换算前必须保持 0.0（未知）。
    #[test]
    fn prices_stay_unknown_until_currency_is_converted() {
        for m in TokenRhythmProvider.models() {
            assert_eq!(m.cost_per_million_in, 0.0, "{} 输入价未换算却已填值", m.id);
            assert_eq!(m.cost_per_million_out, 0.0, "{} 输出价未换算却已填值", m.id);
        }
    }
}
