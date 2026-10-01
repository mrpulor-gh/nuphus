//! Provider registry — single source of truth for Provider lookup
//!
//! Contract: `docs/refactor/2026-07-11-model-layer-pi-style.md` §4.2
//!
//! Holds `Arc<dyn Provider>` entries keyed by their stable id. The registry
//! replaces the scattered `default_base_url()` / `context_window_heuristic()`
//! switches in `config/model.rs`.
//!
//! All 14 built-in Providers (DeepSeek, Kimi, OpenAI, MiniMax, OpenRouter,
//! Google, Qwen, Zhipu, ByteDance, Anthropic, Custom, Local, OpenCode Go, xAI)
//! are registered in `builtin()`.

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

/// Normalize a model id for **fuzzy (tier-2) matching only**.
///
/// Three naming conventions collide in practice: Nuphus built-in ids use
/// hyphens (`claude-opus-4-8`), OpenRouter publishes dots
/// (`anthropic/claude-opus-4.6`), and relays expose their own aliases with
/// capitals and dots (`Claude-Opus-4.6`). Exact matching always runs first
/// (see [`ProviderRegistry::find_model`]); this only rescues ids that would
/// otherwise miss every built-in table and silently lose their capability
/// metadata.
///
/// lowercase → `.` / `_` → `-` → collapse repeated `-` → trim `-`.
pub fn normalize_model_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    let mut prev_dash = false;
    for ch in id.trim().chars() {
        match ch {
            '.' | '_' | '-' => {
                if !prev_dash {
                    out.push('-');
                    prev_dash = true;
                }
            }
            c => {
                for lc in c.to_lowercase() {
                    out.push(lc);
                }
                prev_dash = false;
            }
        }
    }
    out.trim_matches('-').to_string()
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

    /// Look up `query` inside one built-in Provider (`provider_id`): exact id
    /// first, then aliases. The provider-aware counterpart of
    /// [`Self::find_model`] — a same-name model published by another Provider's
    /// table must not shadow the requested one. `None` when the Provider id is
    /// unknown or it does not publish the model.
    pub fn find_model_for_provider(
        &self,
        provider_id: &str,
        query: &str,
    ) -> Option<&'static ModelDef> {
        let provider = self.get(provider_id)?;
        provider
            .models()
            .iter()
            .find(|m| m.id == query || m.aliases.contains(&query))
    }

    /// Provider-scoped lookup that tolerates naming drift (case, `.` vs `-`).
    ///
    /// **Tier-2 only** — call it after [`Self::find_model_for_provider`]
    /// returns `None`, never instead of it: exact matching decides routing,
    /// this only recovers capability metadata for relay-aliased ids such as
    /// `Claude-Opus-4.6` → built-in `claude-opus-4-6`. Provider-scoped so a
    /// same-name model published by another Provider can never shadow it.
    pub fn find_model_for_provider_fuzzy(
        &self,
        provider_id: &str,
        query: &str,
    ) -> Option<&'static ModelDef> {
        let needle = normalize_model_id(query);
        if needle.is_empty() {
            return None;
        }
        let provider = self.get(provider_id)?;
        provider.models().iter().find(|m| {
            normalize_model_id(m.id) == needle
                || m.aliases.iter().any(|a| normalize_model_id(a) == needle)
        })
    }

    /// Provider-agnostic counterpart of [`Self::find_model_for_provider_fuzzy`].
    ///
    /// Iterates providers in a **sorted** order: `providers` is a HashMap, so
    /// unsorted iteration would make a cross-provider hit nondeterministic
    /// (same reason `list_info` sorts by display name).
    pub fn find_model_fuzzy(&self, query: &str) -> Option<(Arc<dyn Provider>, &'static ModelDef)> {
        let needle = normalize_model_id(query);
        if needle.is_empty() {
            return None;
        }
        let mut ids: Vec<&'static str> = self.providers.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let Some(provider) = self.get(id) else {
                continue;
            };
            if let Some(model) = provider.models().iter().find(|m| {
                normalize_model_id(m.id) == needle
                    || m.aliases.iter().any(|a| normalize_model_id(a) == needle)
            }) {
                return Some((Arc::clone(&provider), model));
            }
        }
        None
    }

    /// Frontend-facing Provider list — sorted by display name (stable A-Z order)
    /// so the UI never depends on HashMap iteration order.
    pub fn list_info(&self) -> Vec<FrontendProviderInfo> {
        let mut infos: Vec<FrontendProviderInfo> = self
            .providers
            .values()
            .map(|p| FrontendProviderInfo {
                id: p.id(),
                name: p.display_name(),
                base_url: p.default_base_url(),
                default_model: p.default_model(),
                auth_header: p.auth_header(),
                auth_prefix: p.auth_prefix(),
            })
            .collect();
        infos.sort_by(|a, b| a.name.cmp(b.name));
        infos
    }

    /// Built-in Provider registry.
    ///
    /// Registers all Chat-Completions-based Providers plus Anthropic (Claude).
    /// Each new Provider drops a single `r.register(…)` line.
    pub fn builtin() -> Self {
        let mut r = Self::new();
        use super::providers::{
            AnthropicProvider, ByteDanceProvider, CustomProvider, DeepSeekProvider, GoogleProvider,
            KimiProvider, LocalProvider, MiniMaxProvider, OpenAIProvider, OpenCodeGoProvider,
            OpenRouterProvider, QwenProvider, XaiProvider, ZhipuProvider,
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
        r.register(Arc::new(OpenCodeGoProvider));
        r.register(Arc::new(XaiProvider));
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_case_and_separators() {
        assert_eq!(normalize_model_id("Claude-Opus-4.6"), "claude-opus-4-6");
        assert_eq!(normalize_model_id("claude_opus_4_6"), "claude-opus-4-6");
        assert_eq!(normalize_model_id("  GLM--5.3..Flash  "), "glm-5-3-flash");
        assert_eq!(normalize_model_id("kimi-k3"), "kimi-k3");
        // 只有分隔符 / 空 → 空（调用方据此跳过，不当成通配）
        assert_eq!(normalize_model_id("..."), "");
        assert_eq!(normalize_model_id("   "), "");
    }

    #[test]
    fn exact_match_still_wins_and_is_unchanged() {
        let r = ProviderRegistry::builtin();
        // 精确路径不依赖归一：内置 id 原样命中
        assert!(r
            .find_model_for_provider("anthropic", "claude-sonnet-5")
            .is_some());
        assert!(r
            .find_model_for_provider("anthropic", "Claude-Sonnet-5")
            .is_none());
    }

    #[test]
    fn fuzzy_recovers_relay_aliases() {
        let r = ProviderRegistry::builtin();
        // 中转站常见写法（大写 + 点）在精确层全 miss，tier-2 救回能力元数据
        let hit = r
            .find_model_for_provider_fuzzy("anthropic", "Claude-Sonnet-5")
            .expect("relay alias should recover built-in metadata");
        assert_eq!(hit.id, "claude-sonnet-5");
        // provider 无关 tier 同样命中，且归属 anthropic 段
        let (provider, model) = r
            .find_model_fuzzy("Claude-Sonnet-5")
            .expect("provider-agnostic fuzzy hit");
        assert_eq!(provider.id(), "anthropic");
        assert_eq!(model.id, "claude-sonnet-5");
    }

    #[test]
    fn fuzzy_is_scoped_and_does_not_invent_models() {
        let r = ProviderRegistry::builtin();
        // 版本段不同 → 不命中：内置清单没有 4-6 一代，归一也无对象可匹配
        // （这正是需要 OpenRouter 全目录兜底的原因，见 aggregator::lookup_generic）
        assert!(r
            .find_model_for_provider_fuzzy("anthropic", "claude-opus-4-6")
            .is_none());
        assert!(r.find_model_fuzzy("claude-opus-4-6").is_none());
        // 段作用域：别的 provider 的表不该被 anthropic 查询命中
        assert!(r
            .find_model_for_provider_fuzzy("anthropic", "gpt-4o")
            .is_none());
        assert!(r.find_model_for_provider_fuzzy("anthropic", "").is_none());
    }
}
