//! Provider trait + model metadata types
//!
//! Contract: `docs/refactor/2026-07-11-model-layer-pi-style.md` §4.1
//!
//! All 13 built-in Providers (DeepSeek, Kimi, OpenAI, MiniMax, OpenRouter,
//! Google, Qwen, Zhipu, ByteDance, Anthropic, Custom, Local, OpenCode Go) are
//! implemented under `config/providers/`.
//!
//! All accessor methods carry a safe default body so that an empty stub
//! implementation (`impl Provider for EmptyStub {}`) compiles. The
//! `transport()` method deliberately panics — a concrete Provider must
//! override it before any real traffic flows.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::transports::Transport;

/// Re-export ProviderConfig so that `use crate::config::provider::*` in
/// provider files gets both the trait and the config struct.
pub use super::model::ProviderConfig;

/// 计算实际要下发的鉴权头 `(name, value)`；返回 `None` 表示不携带鉴权头。
///
/// 背景：`local`（以及任何声明「无内置鉴权」的 Provider）把 `auth_header()` /
/// `auth_prefix()` 覆写成空串。空串一旦直接喂给 `reqwest::RequestBuilder::header`，
/// http crate 会判为**非法头名**，最终只抛一句极难定位的 `builder error`
/// （用户侧表现：`IPC invoke list_provider_models failed: 请求失败: builder error`）。
///
/// 语义：用户既然显式填了 key，就说明该端点确实要求鉴权。本地网关
/// （llama-swap / vLLM / LiteLLM / 带鉴权的 Ollama 反代）统一是 OpenAI 兼容的
/// `Authorization: Bearer <key>`——实测 llama-swap **只认带 `Bearer ` 前缀**的形式，
/// 裸 key 与空 Bearer 都返回 401。因此这里在「未声明鉴权方案」时补上该约定。
///
/// 已声明 header 的 Provider 完全不受影响（含 `x-api-key` + 空前缀这类 scheme）。
pub fn resolve_auth(
    declared_header: &str,
    declared_prefix: &str,
    api_key: &str,
) -> Option<(String, String)> {
    // 头值里混入首尾空白/换行会直接让 header 非法（粘贴 key 的常见副作用）。
    let key = api_key.trim();
    if key.is_empty() {
        return None;
    }
    if declared_header.is_empty() {
        // 头名与其它 Provider 声明的写法保持一致（全小写）。HTTP 头名大小写不敏感，
        // 纯粹为了仓内一致性。
        return Some(("authorization".to_string(), format!("Bearer {}", key)));
    }
    Some((
        declared_header.to_string(),
        format!("{}{}", declared_prefix, key),
    ))
}

/// Single model definition owned by a Provider.
///
/// Fields mirror the provider-driven metadata approach: each Provider publishes the
/// concrete model list (id + aliases + sizing + capability flags) so the agent
/// layer can pick correctly without string heuristics.
#[derive(Debug, Clone, Serialize)]
pub struct ModelDef {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub supports_streaming: bool,
    pub supports_vision: bool,
    pub supports_reasoning: bool,
    pub supports_audio: bool,
    pub supports_image_generation: bool,
    pub cost_per_million_in: f64,
    pub cost_per_million_out: f64,
    pub reasoning_field: &'static str,
    /// Reasoning-effort levels this model accepts (`"low"`, `"high"`, `"max"`,
    /// `"none"`, …). Empty = the model exposes no user-configurable effort.
    /// DeepSeek v4: flash = [low, high, max], pro = [high, max] (per
    /// api-docs.deepseek.com/guides/thinking_mode effort mapping table).
    pub reasoning_efforts: &'static [&'static str],
    /// Default effort used when the user has not configured one (i.e. the
    /// provider's behavior when `reasoning_effort` is absent). `None` = no
    /// declared default — the UI falls back to showing "默认".
    /// DeepSeek v4: thinking mode defaults to `high` when the parameter is
    /// omitted (see config.example.toml reasoning_effort comment).
    pub default_effort: Option<&'static str>,
}

/// Identifies which request field carries the max-output-tokens cap.
///
/// Different Providers expose different field names (`max_tokens` vs
/// `max_completion_tokens`). The transport layer reads this to pick the right
/// key without sniffing Provider identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxTokensField {
    /// OpenAI Chat Completions legacy — `max_tokens: u32`.
    MaxTokens,
    /// Newer Chat Completions style — `max_completion_tokens: u32`.
    MaxCompletionTokens,
}

/// Per-Provider protocol quirks (sanitizers, headers, forbidden fields).
///
/// Replaces the string branches currently living in
/// `transports/chat_completions/schema_fix.rs` and `transport.rs`. Each
/// Provider decides at runtime which quirks apply.
#[derive(Debug, Clone)]
pub struct ProviderQuirks {
    /// Whether the Provider needs the reasoning content echoed back in
    /// subsequent requests (DeepSeek thinking mode, etc.).
    pub requires_reasoning_echo: bool,
    /// Whether the Provider's API accepts a `reasoning_effort` request field
    /// (DeepSeek v4 thinking mode). When false the transport never emits the
    /// parameter even if `ProviderConfig.reasoning_effort` is set — this keeps
    /// the other 11 Providers' request bodies unchanged.
    pub supports_reasoning_effort: bool,
    /// Whether the provider rejects `reasoning_effort` on requests that carry
    /// tools (DeepSeek: effort only valid without tools — transport suppresses
    /// it on tool-carrying requests). False = effort may accompany tools
    /// (verified for Kimi k3 against the live API).
    pub effort_excludes_tools: bool,
    /// Optional tool-schema sanitizer hook. Takes the provider's tool
    /// definitions and returns sanitised copies (some providers e.g. Kimi /
    /// Moonshot require simplified JSON Schema).
    pub sanitize_tools:
        Option<fn(&[crate::api::ToolDefinition]) -> Vec<crate::api::ToolDefinition>>,
    /// Static headers to inject into every request (e.g. `anthropic-version`).
    pub extra_headers: Vec<(String, String)>,
    /// Request fields that the upstream API rejects — stripped before send.
    pub forbidden_request_fields: &'static [&'static str],
    /// Which field carries the max-tokens cap (see [`MaxTokensField`]).
    pub max_tokens_field: MaxTokensField,
    /// Custom User-Agent string for the HTTP client.
    /// Some providers (e.g. Kimi Code API) require a specific User-Agent.
    pub user_agent: Option<&'static str>,
    /// XML tag names that this Provider uses to embed tool calls inside the
    /// `content` text field. The content normalizer (`strip_tool_xml_tags` /
    /// `extract_tool_calls_from_text`) will parse these tags as ToolUse blocks
    /// and strip the raw XML from the message display.
    ///
    /// Each entry is the tag name (without `<>`), e.g. `"function_call"`.
    /// The normalizer matches both `<tag>...</tag>` and `<tag />` forms.
    ///
    /// Default: empty — no additional tags beyond the built-in set
    /// (`tool_call`, `invoke`, `command`, `parameter`).
    pub content_tool_tags: &'static [&'static str],
    /// JSON field name within `usage` that carries cache hit tokens.
    /// "" = try OpenAI-standard path `usage.prompt_tokens_details.cached_tokens`.
    pub cache_hit_field: &'static str,
}

impl Default for ProviderQuirks {
    fn default() -> Self {
        Self {
            requires_reasoning_echo: false,
            supports_reasoning_effort: false,
            effort_excludes_tools: false,
            sanitize_tools: None,
            extra_headers: Vec::new(),
            forbidden_request_fields: &[],
            max_tokens_field: MaxTokensField::MaxTokens,
            user_agent: None,
            content_tool_tags: &[],
            cache_hit_field: "", // default: try OpenAI standard path
        }
    }
}

/// Protocol family used to reach a Provider's API.
///
/// Routing metadata, dispatched since P4 (refactor design §4.3 / §7 of
/// `docs/refactor/2026-09-06-model-routing-auth-system.md`): Providers
/// *declare* their protocol via [`Provider::default_transport`] /
/// [`Provider::transport_for`], and `ClientFactory::build_transport` constructs
/// from the resolved kind — `Responses` builds `ResponsesTransport` in the
/// factory; `ChatCompletions` / `Anthropic` still delegate to the legacy
/// [`Provider::transport`] path (13-provider status quo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportKind {
    /// OpenAI-compatible chat protocol — POST `{base}/chat/completions`.
    ChatCompletions,
    /// Anthropic Messages protocol — POST `{base}/v1/messages`.
    Anthropic,
    /// OpenAI Responses protocol (SSE) — POST `{base}/responses`.
    Responses,
}

/// LLM Provider abstraction.
///
/// Each concrete Provider (DeepSeek, Kimi, OpenAI, …) owns its metadata,
/// quirks, and transport selection. The registry (§4.2) holds
/// `Arc<dyn Provider>` entries; agent code looks models up by id or alias
/// via `ProviderRegistry::find_model` instead of the current string
/// heuristics.
///
/// ## Send + Sync
///
/// The trait is `Send + Sync` so `Arc<dyn Provider>` can be shared across
/// threads (agent runtime, Tauri command handlers, …).
///
/// ## Defaults
///
/// Every accessor has a no-op default returning the empty/safe value. A
/// placeholder Provider that only overrides a subset of methods compiles.
/// `transport()` is the only method that cannot have a useful default; it
/// `unimplemented!()`s to make accidental use loud.
pub trait Provider: Send + Sync {
    /// Stable identifier (`"deepseek"`, `"kimi"`, `"openai"`, …) — also the
    /// key under which the registry stores this Provider.
    fn id(&self) -> &'static str {
        ""
    }

    /// Human-readable name for the frontend.
    fn display_name(&self) -> &'static str {
        ""
    }

    /// Default API base URL (without trailing slash). Used when the TOML
    /// config does not override `base_url`.
    fn default_base_url(&self) -> &'static str {
        ""
    }

    /// HTTP header name carrying the API key.
    fn auth_header(&self) -> &'static str {
        "Authorization"
    }

    /// Auth header value prefix (e.g. `"Bearer "`, `""` for `x-api-key`).
    fn auth_prefix(&self) -> &'static str {
        "Bearer "
    }

    /// Default model when the TOML `model` field is empty.
    fn default_model(&self) -> &'static str {
        ""
    }

    /// Concrete model list published by this Provider. Phase 2 fills these
    /// in for each real Provider; an empty slice is the safe default.
    fn models(&self) -> &'static [ModelDef] {
        &[]
    }

    /// Per-Provider quirks consumed by the transport layer.
    fn quirks(&self) -> ProviderQuirks {
        ProviderQuirks::default()
    }

    /// Default protocol family for this Provider.
    ///
    /// Used when no per-model dispatch is declared. Most chat providers do not
    /// override this → `ChatCompletions` (status quo). `anthropic.rs`
    /// overrides it to `Anthropic` to mirror its existing `transport()`
    /// implementation.
    fn default_transport(&self) -> TransportKind {
        TransportKind::ChatCompletions
    }

    /// Resolve the protocol family for a concrete model id.
    ///
    /// Defaults to [`Self::default_transport`]. Only Providers with a
    /// per-model-family dispatch table (opencode-go responses/anthropic
    /// families) override this. The resolved kind drives transport
    /// construction via the P4 dispatch in `ClientFactory::build_transport`.
    fn transport_for(&self, _model_id: &str) -> TransportKind {
        self.default_transport()
    }

    /// Build a Transport bound to the supplied user config and model id.
    /// Concrete Providers return either a `ChatCompletionsTransport` or the
    /// Anthropic Messages API transport. Must be overridden.
    fn transport(&self, _cfg: &ProviderConfig, _model_id: &str) -> Arc<dyn Transport> {
        unimplemented!(
            "Provider::transport() must be overridden by the concrete Provider \
             (Phase 1 stub — Provider files arrive in Phase 2 of \
             docs/refactor/2026-07-11-model-layer-pi-style.md)"
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// TransportKind serde kebab-case roundtrip (P1 routing metadata contract).
    #[test]
    fn transport_kind_serde_roundtrip() {
        let cases = [
            (TransportKind::ChatCompletions, "chat-completions"),
            (TransportKind::Anthropic, "anthropic"),
            (TransportKind::Responses, "responses"),
        ];
        for (kind, text) in cases {
            let s = serde_json::to_string(&kind).unwrap();
            assert_eq!(s, format!("\"{}\"", text), "serialize {}", text);
            let back: TransportKind = serde_json::from_str(&s).unwrap();
            assert_eq!(back, kind, "roundtrip {}", text);
            // TOML roundtrip：实际形态是 providers.toml 中某字段 = "chat-completions"
            // 等 kebab-case 字符串（config-layer 经 serde 反序列化），故用字段
            // wrapper 承载而非裸 enum 顶层解析。
            #[derive(serde::Deserialize)]
            struct KindField {
                transport: TransportKind,
            }
            let via_toml: KindField =
                toml::from_str(&format!("transport = \"{}\"\n", text)).unwrap();
            assert_eq!(via_toml.transport, kind, "toml roundtrip {}", text);
        }
    }

    /// anthropic.rs is the only Provider that overrides default_transport —
    /// it must declare Anthropic to mirror its AnthropicTransport transport().
    #[test]
    fn anthropic_default_transport_is_anthropic() {
        let p = crate::config::providers::AnthropicProvider;
        assert_eq!(p.default_transport(), TransportKind::Anthropic);
        // No per-model dispatch declared → transport_for falls back to default.
        assert_eq!(p.transport_for("claude-sonnet-5"), TransportKind::Anthropic);
        assert_eq!(p.transport_for("anything"), TransportKind::Anthropic);
    }

    /// Every other built-in Provider keeps the ChatCompletions default.
    #[test]
    fn chat_providers_default_to_chat_completions() {
        use crate::config::providers::{
            ByteDanceProvider, CustomProvider, DeepSeekProvider, GoogleProvider, KimiProvider,
            LocalProvider, MiniMaxProvider, OpenAIProvider, OpenCodeGoProvider, OpenRouterProvider,
            QwenProvider, ZhipuProvider,
        };
        let providers: Vec<Box<dyn Provider>> = vec![
            Box::new(ByteDanceProvider),
            Box::new(CustomProvider),
            Box::new(DeepSeekProvider),
            Box::new(GoogleProvider),
            Box::new(KimiProvider),
            Box::new(LocalProvider),
            Box::new(MiniMaxProvider),
            Box::new(OpenAIProvider),
            Box::new(OpenCodeGoProvider),
            Box::new(OpenRouterProvider),
            Box::new(QwenProvider),
            Box::new(ZhipuProvider),
        ];
        for p in &providers {
            assert_eq!(
                p.default_transport(),
                TransportKind::ChatCompletions,
                "provider {} must keep ChatCompletions default",
                p.id()
            );
            // transport_for: 未覆写的 provider（P4 后仅 opencode-go 覆写）
            // 一律落回 default_transport；这里用 "any-model" 断言兜底路径。
            assert_eq!(
                p.transport_for("any-model"),
                p.default_transport(),
                "transport_for must default to default_transport for {}",
                p.id()
            );
        }
    }

    // ── resolve_auth：未声明鉴权方案时补 OpenAI 兼容约定 ──

    /// 空 key（含只有空白）一律不发鉴权头。
    ///
    /// llama-swap 等严格网关把「空 Bearer」判为 401，所以「没填 key」必须表现为
    /// 「完全不带该头」，而不是带一个空值。
    #[test]
    fn resolve_auth_empty_key_sends_nothing() {
        assert_eq!(resolve_auth("authorization", "Bearer ", ""), None);
        assert_eq!(resolve_auth("authorization", "Bearer ", "   "), None);
        // 粘贴 key 时常见的首尾换行/制表符
        assert_eq!(resolve_auth("x-api-key", "", "\n\t "), None);
        // 未声明方案 + 空 key 同样不发头
        assert_eq!(resolve_auth("", "", ""), None);
    }

    /// 未声明鉴权方案（`auth_header()` 为空串，如 local）→ 补 Authorization: Bearer。
    ///
    /// 这是本次修复的核心分支：原实现把空串当头名交给 reqwest，只抛 builder error。
    #[test]
    fn resolve_auth_undeclared_scheme_falls_back_to_bearer() {
        assert_eq!(
            resolve_auth("", "", "sk-local"),
            Some(("authorization".to_string(), "Bearer sk-local".to_string()))
        );
        // 空 header 但 prefix 非空属异常配置，仍按「未声明方案」处理，不让空头名漏出去
        assert_eq!(
            resolve_auth("", "Bearer ", "sk-local"),
            Some(("authorization".to_string(), "Bearer sk-local".to_string()))
        );
    }

    /// 已声明 header 的 Provider 行为必须与改造前逐字一致（零回归）。
    #[test]
    fn resolve_auth_declared_scheme_is_untouched() {
        // 主流 OpenAI 兼容：authorization + "Bearer "
        assert_eq!(
            resolve_auth("authorization", "Bearer ", "sk-x"),
            Some(("authorization".to_string(), "Bearer sk-x".to_string()))
        );
        // 无前缀 scheme：kimi 的 x-api-key / google 的 x-goog-api-key 保持裸值
        assert_eq!(
            resolve_auth("x-api-key", "", "sk-x"),
            Some(("x-api-key".to_string(), "sk-x".to_string()))
        );
        assert_eq!(
            resolve_auth("x-goog-api-key", "", "sk-x"),
            Some(("x-goog-api-key".to_string(), "sk-x".to_string()))
        );
    }

    /// key 首尾空白必须被裁掉：头值含空白同样会让 header 非法（builder error 复现路径）。
    #[test]
    fn resolve_auth_trims_key_whitespace() {
        assert_eq!(
            resolve_auth("authorization", "Bearer ", "  sk-x \n"),
            Some(("authorization".to_string(), "Bearer sk-x".to_string()))
        );
        assert_eq!(
            resolve_auth("", "", " sk-x "),
            Some(("authorization".to_string(), "Bearer sk-x".to_string()))
        );
    }
}
