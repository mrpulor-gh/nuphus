//! Provider trait + model metadata types
//!
//! Contract: `docs/refactor/2026-07-11-model-layer-pi-style.md` §4.1
//!
//! All 12 built-in Providers (DeepSeek, Kimi, OpenAI, MiniMax, OpenRouter,
//! Google, Qwen, Zhipu, ByteDance, Anthropic, Custom, Local) are implemented
//! under `config/providers/`.
//!
//! All accessor methods carry a safe default body so that an empty stub
//! implementation (`impl Provider for EmptyStub {}`) compiles. The
//! `transport()` method deliberately panics — a concrete Provider must
//! override it before any real traffic flows.

use serde::Serialize;
use std::sync::Arc;

use crate::transports::Transport;

/// Re-export ProviderConfig so that `use crate::config::provider::*` in
/// provider files gets both the trait and the config struct.
pub use super::model::ProviderConfig;

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
