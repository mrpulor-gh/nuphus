//! Provider registry — single source of truth for Provider lookup
//!
//! Contract: `docs/refactor/2026-07-11-model-layer-pi-style.md` §4.2
//!
//! Holds `Arc<dyn Provider>` entries keyed by their stable id. The registry
//! replaces the scattered `default_base_url()` / `context_window_heuristic()`
//! switches in `config/model.rs`.
//!
//! All 12 built-in Providers (DeepSeek, Kimi, OpenAI, MiniMax, OpenRouter,
//! Google, Qwen, Zhipu, ByteDance, Anthropic, Custom, Local) are registered
//! in `builtin()`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use super::provider::{ModelDef, Provider};

/// Frontend-facing Provider info view.
///
/// Phase 1 stub — surface mirrors `config::ProviderInfo` for backward
/// compatibility. Later phases will grow this with capability flags (vision,
/// stt, tts) and richer display metadata, then collapse the two views into
/// one. For now, the registry emits enough information for existing UI
/// consumers to recognise each Provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub default_model: &'static str,
    pub auth_header: &'static str,
    pub auth_prefix: &'static str,
}

/// Registry of `Provider` implementations keyed by stable id.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<&'static str, Arc<dyn Provider>>,
}

impl ProviderRegistry {
    /// Construct an empty registry.
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Register a Provider under the id returned by `Provider::id()`.
    ///
    /// If another Provider shares that id, it is silently replaced — useful
    /// for tests and overrides; legitimate collisions signal a duplicate
    /// registration that should be resolved at the call site.
    pub fn register(&mut self, provider: Arc<dyn Provider>) {
        self.providers.insert(provider.id(), provider);
    }

    /// Resolve a model query (id or alias) to the owning Provider + metadata.
    ///
    /// Replaces the `context_window_heuristic` string chain. Matches the
    /// query against every registered Provider's `models()` list — first by
    /// `id`, then by any entry in `aliases`. Returns `None` if no Provider
    /// matches.
    pub fn find_model(&self, query: &str) -> Option<(Arc<dyn Provider>, &'static ModelDef)> {
        for provider in self.providers.values() {
            for model in provider.models() {
                if model.id == query || model.aliases.contains(&query) {
                    return Some((Arc::clone(provider), model));
                }
            }
        }
        None
    }

    /// Resolve a Provider by its stable id.
    pub fn get(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(id).map(Arc::clone)
    }

    /// Snapshot of every registered Provider. Iteration order is unspecified.
    pub fn all(&self) -> Vec<Arc<dyn Provider>> {
        self.providers.values().map(Arc::clone).collect()
    }

    /// Frontend-facing Provider list — same order as `all()`.
    pub fn list_info(&self) -> Vec<FrontendProviderInfo> {
        self.providers
            .values()
            .map(|p| FrontendProviderInfo {
                id: p.id(),
                name: p.display_name(),
                base_url: p.default_base_url(),
                default_model: p.default_model(),
                auth_header: p.auth_header(),
                auth_prefix: p.auth_prefix(),
            })
            .collect()
    }

    /// Built-in Provider registry.
    ///
    /// Registers all Chat-Completions-based Providers plus Anthropic (Claude).
    /// Each new Provider drops a single `r.register(…)` line.
    pub fn builtin() -> Self {
        let mut r = Self::new();
        use super::providers::{
            AnthropicProvider, ByteDanceProvider, CustomProvider, DeepSeekProvider, GoogleProvider,
            KimiProvider, LocalProvider, MiniMaxProvider, OpenAIProvider, OpenRouterProvider,
            QwenProvider, ZhipuProvider,
        };
        r.register(Arc::new(DeepSeekProvider));
        r.register(Arc::new(KimiProvider));
        r.register(Arc::new(OpenAIProvider));
        r.register(Arc::new(MiniMaxProvider));
        r.register(Arc::new(OpenRouterProvider));
        r.register(Arc::new(GoogleProvider));
        r.register(Arc::new(QwenProvider));
        r.register(Arc::new(ZhipuProvider));
        r.register(Arc::new(ByteDanceProvider));
        r.register(Arc::new(AnthropicProvider));
        r.register(Arc::new(CustomProvider));
        r.register(Arc::new(LocalProvider));
        r
    }
}
