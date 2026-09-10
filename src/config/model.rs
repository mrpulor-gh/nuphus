//! Model configuration data structures
//!
//! Supports multiple Providers, multiple models, alias mapping

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Canonical Provider type — re-exported from `api::ProviderKind` for config-layer consumers.
pub use crate::api::ProviderKind;
pub use ProviderKind as KnownProvider;

/// Model entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    #[serde(default)]
    pub alias: Vec<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub context_window: Option<usize>,
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub supports_audio: bool,
    #[serde(default)]
    pub supports_image_generation: bool,
    /// Reasoning-effort levels this model accepts (e.g. ["low","high","max"]),
    /// discovered from the provider's /models metadata at configure time.
    /// Empty = no configurable effort (frontend hides the selector).
    #[serde(default)]
    pub reasoning_efforts: Vec<String>,
    /// Provider-declared default effort (e.g. Kimi k3 = "high"). None = no
    /// declared default; UI shows the provider-default state.
    #[serde(default)]
    pub default_effort: Option<String>,
    /// Explicit per-million cost (USD) — providers.toml 手写值，信任链最高层。
    /// None = 未手写（list_models 回退 OpenRouter 聚合库定价）。
    #[serde(default)]
    pub cost_per_million_in: Option<f64>,
    /// Explicit per-million completion cost (USD). None = 未手写。
    #[serde(default)]
    pub cost_per_million_out: Option<f64>,
}

fn default_true() -> bool {
    true
}

/// Single Provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub provider_type: ProviderKind,
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub auth_header: String,
    #[serde(default)]
    pub auth_prefix: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
    /// Reasoning depth for models that expose effort control (e.g. DeepSeek v4:
    /// `"low" | "high" | "max"`). None = provider default (transport sends no
    /// `reasoning_effort` parameter). Optional — absent in existing configs.
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

fn default_timeout() -> u64 {
    300
}

/// 按能力独立配置模型（不配则使用 model）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// 图像理解模型
    #[serde(default)]
    pub vision: String,
    /// 语音转文字模型（空 = 使用本地 SenseVoice ONNX）
    #[serde(default)]
    pub stt: String,
    /// 文字转语音模型
    #[serde(default)]
    pub tts: String,
    /// 语音克隆模型（走云端克隆 API，空 = 不支持）
    #[serde(default)]
    pub voice: String,
    /// 图片生成模型（空 = 不支持）
    #[serde(default)]
    pub image_generation: String,
    /// ChatAgent 默认最大推理轮数（不配则 15）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_agent_max_iterations: Option<u32>,
}

/// Model registry — manages all Providers and models
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelRegistry {
    /// 主模型（前端切换入口，所有文本任务默认值）
    #[serde(default, alias = "default_model")]
    pub model: String,
    /// All Provider configurations
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// 按能力独立配置模型
    #[serde(default)]
    pub capabilities: Capabilities,
    /// Model alias mapping: alias -> (provider_name, model_id)
    #[serde(skip)]
    alias_map: HashMap<String, (String, String)>,
}

impl ModelRegistry {
    /// Load from TOML config file
    pub fn from_toml(path: &str) -> crate::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::NuphusError::Config(format!("read config failed: {e}")))?;
        let mut registry: Self = toml::from_str(&content)
            .map_err(|e| crate::NuphusError::Config(format!("parse config failed: {e}")))?;
        // API key 透明解密：落盘为 `enc:v1:`（DPAPI）时还原明文；旧明文配置原样兼容。
        // 旧版 `enc:`（无版本号）密文一并迁移解密；密文但解密失败视为未配置（触发重新导入）。
        // 环境变量来源（from_env）不经此路径，无需解密。
        for p in &mut registry.providers {
            let encrypted = p.api_key.starts_with("enc:");
            match crate::cookies::decrypt_secret(&p.api_key) {
                Some(dec) => p.api_key = dec,
                None if encrypted => {
                    tracing::warn!(
                        "[config] provider '{}' 的 api_key 无法解密，视为未配置（请重新配置）",
                        p.name
                    );
                    p.api_key.clear();
                }
                None => {}
            }
        }
        // 模型真值 = [agent_models].leader（主模型，mode 绑定单一数据源）。
        // providers.toml 顶层 model 字段已退役：不构成覆盖层。leader 可用时以 leader
        // 为准（覆盖 serde 读入的顶层旧值）；leader 空（旧文件未迁移绑定）→ 保留顶层
        // 历史值作一次性兼容兜底，不写回、不参与任何优先级比较。
        if let Ok(doc) = content.parse::<toml::Value>() {
            if let Some(leader) = doc
                .get("agent_models")
                .and_then(|a| a.get("leader"))
                .and_then(|v| v.as_str())
            {
                let avail = !leader.is_empty()
                    && registry
                        .providers
                        .iter()
                        .any(|p| p.models.iter().any(|m| m.id == leader));
                if avail {
                    registry.model = leader.to_string();
                }
            }
        }
        registry.build_alias_map();
        Ok(registry)
    }

    /// Auto-build from environment variables (compatible with existing behavior)
    pub fn from_env() -> crate::Result<Self> {
        let mut providers = Vec::new();

        // Try DeepSeek
        if let Ok(api_key) = std::env::var("DEEPSEEK_API_KEY") {
            providers.push(ProviderConfig {
                name: "deepseek".to_string(),
                provider_type: KnownProvider::DeepSeek,
                api_key,
                base_url: std::env::var("DEEPSEEK_BASE_URL")
                    .unwrap_or_else(|_| "https://api.deepseek.com".to_string()),
                auth_header: "authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                timeout_secs: 300,
                models: vec![ModelEntry {
                    id: std::env::var("DEEPSEEK_MODEL")
                        .unwrap_or_else(|_| "deepseek-v4-flash".to_string()),
                    alias: vec!["deepseek".to_string()],
                    max_tokens: None,
                    context_window: None,
                    supports_streaming: true,
                    supports_vision: false,
                    supports_audio: false,
                    supports_image_generation: false,
                    reasoning_efforts: Vec::new(),
                    default_effort: None,
                    cost_per_million_in: None,
                    cost_per_million_out: None,
                }],
                reasoning_effort: None,
            });
        }

        // Try Kimi
        if let Ok(api_key) = std::env::var("KIMI_API_KEY") {
            providers.push(ProviderConfig {
                name: "kimi".to_string(),
                provider_type: KnownProvider::Kimi,
                api_key,
                base_url: std::env::var("KIMI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.kimi.com/coding/v1".to_string()),
                auth_header: "x-api-key".to_string(),
                auth_prefix: "".to_string(),
                timeout_secs: 300,
                models: vec![ModelEntry {
                    id: std::env::var("KIMI_MODEL")
                        .unwrap_or_else(|_| "kimi-for-coding".to_string()),
                    alias: vec!["kimi".to_string()],
                    max_tokens: None,
                    context_window: None,
                    supports_streaming: true,
                    supports_vision: false,
                    supports_audio: false,
                    supports_image_generation: false,
                    reasoning_efforts: Vec::new(),
                    default_effort: None,
                    cost_per_million_in: None,
                    cost_per_million_out: None,
                }],
                reasoning_effort: None,
            });
        }

        // Try MiniMax (fallback)
        if let Ok(api_key) = std::env::var("MINIMAX_API_KEY") {
            providers.push(ProviderConfig {
                name: "minimax".to_string(),
                provider_type: KnownProvider::MiniMax,
                api_key,
                base_url: std::env::var("MINIMAX_BASE_URL")
                    .unwrap_or_else(|_| "https://api.minimaxi.com/v1".to_string()),
                auth_header: "authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                timeout_secs: 300,
                models: vec![ModelEntry {
                    id: std::env::var("MINIMAX_MODEL")
                        .unwrap_or_else(|_| "MiniMax-M2.7".to_string()),
                    alias: vec!["minimax".to_string()],
                    max_tokens: None,
                    context_window: None,
                    supports_streaming: true,
                    supports_vision: false,
                    supports_audio: false,
                    supports_image_generation: false,
                    reasoning_efforts: Vec::new(),
                    default_effort: None,
                    cost_per_million_in: None,
                    cost_per_million_out: None,
                }],
                reasoning_effort: None,
            });
        }

        // Try Qwen (通义千问)
        if let Ok(api_key) = std::env::var("QWEN_API_KEY") {
            providers.push(ProviderConfig {
                name: "qwen".to_string(),
                provider_type: KnownProvider::Qwen,
                api_key,
                base_url: std::env::var("QWEN_BASE_URL").unwrap_or_else(|_| {
                    "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string()
                }),
                auth_header: "authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                timeout_secs: 300,
                models: vec![ModelEntry {
                    id: std::env::var("QWEN_MODEL").unwrap_or_else(|_| "qwen-plus".to_string()),
                    alias: vec!["qwen".to_string()],
                    max_tokens: None,
                    context_window: None,
                    supports_streaming: true,
                    supports_vision: false,
                    supports_audio: false,
                    supports_image_generation: false,
                    reasoning_efforts: Vec::new(),
                    default_effort: None,
                    cost_per_million_in: None,
                    cost_per_million_out: None,
                }],
                reasoning_effort: None,
            });
        }

        // Try Zhipu (智谱)
        if let Ok(api_key) = std::env::var("ZHIPU_API_KEY") {
            providers.push(ProviderConfig {
                name: "zhipu".to_string(),
                provider_type: KnownProvider::Zhipu,
                api_key,
                base_url: std::env::var("ZHIPU_BASE_URL")
                    .unwrap_or_else(|_| "https://open.bigmodel.cn/api/paas/v4".to_string()),
                auth_header: "authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                timeout_secs: 300,
                models: vec![ModelEntry {
                    id: std::env::var("ZHIPU_MODEL").unwrap_or_else(|_| "glm-4-flash".to_string()),
                    alias: vec!["zhipu".to_string()],
                    max_tokens: None,
                    context_window: None,
                    supports_streaming: true,
                    supports_vision: false,
                    supports_audio: false,
                    supports_image_generation: false,
                    reasoning_efforts: Vec::new(),
                    default_effort: None,
                    cost_per_million_in: None,
                    cost_per_million_out: None,
                }],
                reasoning_effort: None,
            });
        }

        // Try ByteDance (豆包)
        if let Ok(api_key) = std::env::var("BYTEDANCE_API_KEY") {
            providers.push(ProviderConfig {
                name: "bytedance".to_string(),
                provider_type: KnownProvider::ByteDance,
                api_key,
                base_url: std::env::var("BYTEDANCE_BASE_URL")
                    .unwrap_or_else(|_| "https://ark.cn-beijing.volces.com/api/v3".to_string()),
                auth_header: "authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                timeout_secs: 300,
                models: vec![ModelEntry {
                    id: std::env::var("BYTEDANCE_MODEL")
                        .unwrap_or_else(|_| "doubao-1-5-pro-32k".to_string()),
                    alias: vec!["bytedance".to_string()],
                    max_tokens: None,
                    context_window: None,
                    supports_streaming: true,
                    supports_vision: false,
                    supports_audio: false,
                    supports_image_generation: false,
                    reasoning_efforts: Vec::new(),
                    default_effort: None,
                    cost_per_million_in: None,
                    cost_per_million_out: None,
                }],
                reasoning_effort: None,
            });
        }

        if providers.is_empty() {
            return Err(crate::NuphusError::Config(
                "no API key found in environment".to_string(),
            ));
        }

        let default_model = providers[0].models[0].id.clone();
        let mut registry = Self {
            model: default_model,
            providers,
            capabilities: Capabilities::default(),
            alias_map: Default::default(),
        };
        registry.build_alias_map();
        Ok(registry)
    }

    /// Find model configuration (legacy first-match semantics).
    ///
    /// Returns the first segment-order hit (status quo). When the same model id
    /// exists in more than one provider the ambiguity is logged as a warning
    /// instead of silently preferring the first — config/UI layers must use
    /// [`Self::find_model_candidates`] to disambiguate (see §4.6 of the
    /// model-routing refactor design). Callers keep their existing behaviour.
    pub fn find_model(&self, model_id: &str) -> Option<(&ProviderConfig, &ModelEntry)> {
        // Check alias first (alias map is de-duplicated: last insert wins).
        if let Some((provider_name, real_id)) = self.alias_map.get(model_id) {
            let provider = self.providers.iter().find(|p| &p.name == provider_name)?;
            let model = provider.models.iter().find(|m| &m.id == real_id)?;
            return Some((provider, model));
        }
        // Then check direct match across all providers — collect every hit so
        // ambiguity can be surfaced, but keep returning the segment-order first
        // (compatible with existing behaviour).
        let mut first: Option<(&ProviderConfig, &ModelEntry)> = None;
        let mut matched: Vec<(&ProviderConfig, &ModelEntry)> = Vec::new();
        for provider in &self.providers {
            if let Some(model) = provider.models.iter().find(|m| m.id == model_id) {
                if first.is_none() {
                    first = Some((provider, model));
                }
                matched.push((provider, model));
            }
        }
        if matched.len() > 1 {
            tracing::warn!(
                "[config] model '{}' 存在跨 provider 重名（{} 个候选：{}），按段序返回首个（{}）；\
                 配置写入/UI 选择请用 find_model_candidates 消歧",
                model_id,
                matched.len(),
                matched
                    .iter()
                    .map(|(p, _)| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                first.map(|(p, _)| p.name.as_str()).unwrap_or(""),
            );
        }
        first
    }

    /// Find a model by its complete provider name and model id.
    pub fn find_model_for_provider(
        &self,
        provider_name: &str,
        model_id: &str,
    ) -> Option<(&ProviderConfig, &ModelEntry)> {
        let provider = self.providers.iter().find(|p| p.name == provider_name)?;
        let model = provider.models.iter().find(|m| m.id == model_id)?;
        Some((provider, model))
    }

    /// Find every provider+model pair matching `model_id` (alias-aware).
    ///
    /// - Alias hit: expands to the canonical model id, then returns every
    ///   provider that publishes that id (the alias owner plus any same-id
    ///   duplicates across segments).
    /// - Otherwise returns all providers whose model list contains the id.
    ///
    /// Returns an empty vec when nothing matches. UI model pickers / config
    /// probes use this to present provider-tagged candidates instead of an
    /// implicit first hit (zhipu vs opencode-go same-name case).
    pub fn find_model_candidates(&self, model_id: &str) -> Vec<(&ProviderConfig, &ModelEntry)> {
        // Alias expansion: alias_map stores (provider_name, canonical model id).
        let lookup_id: &str = match self.alias_map.get(model_id) {
            Some((_provider_name, real_id)) => real_id.as_str(),
            None => model_id,
        };
        self.providers
            .iter()
            .filter_map(|provider| {
                provider
                    .models
                    .iter()
                    .find(|m| m.id == lookup_id)
                    .map(|model| (provider, model))
            })
            .collect()
    }

    /// Resolve a model's context window from the registry (provider-aware).
    ///
    /// Same-id models can live under several providers (official segment vs
    /// gateway/custom segment) and only some of them declare `context_window`.
    /// A first-match lookup therefore returns `None` whenever the segment-order
    /// first hit happens to omit the value, masking a sibling that declares it
    /// (root cause of `deepseek-v4.1-flash-expires-on-0910` falling through to
    /// the builtin table and then to the 128K guess).
    ///
    /// Rules:
    /// 1. `provider_name` given and that provider publishes `model_id` with a
    ///    value → the provider-exact value wins. Callers that know the routing
    ///    binding must pass it (see §4.6, `provider` of `LlamaConfig`/`AgentConfig`).
    /// 2. Otherwise every same-id candidate is scanned in segment order
    ///    ([`Self::find_model_candidates`]) and the first one carrying a value
    ///    is returned — a candidate without a value never masks its siblings.
    /// 3. No candidate carries a value → `None` (caller decides the fallback).
    pub fn resolve_context_window(
        &self,
        provider_name: Option<&str>,
        model_id: &str,
    ) -> Option<usize> {
        if let Some(provider_name) = provider_name.filter(|p| !p.is_empty()) {
            if let Some((_, model)) = self.find_model_for_provider(provider_name, model_id) {
                if let Some(window) = model.context_window {
                    return Some(window);
                }
            }
        }
        self.find_model_candidates(model_id)
            .into_iter()
            .find_map(|(_, model)| model.context_window)
    }

    /// List all available models
    pub fn list_models(&self) -> Vec<(String, String)> {
        let mut result = Vec::new();
        for provider in &self.providers {
            for model in &provider.models {
                result.push((provider.name.clone(), model.id.clone()));
            }
        }
        result
    }

    fn build_alias_map(&mut self) {
        self.alias_map.clear();
        for provider in &self.providers {
            for model in &provider.models {
                for alias in &model.alias {
                    self.alias_map
                        .insert(alias.clone(), (provider.name.clone(), model.id.clone()));
                }
            }
        }
    }

    /// Create a single-provider registry from in-memory config
    /// (used by send_message_cmd when startup-loaded LLM config is available).
    pub fn from_single(
        model: String,
        provider_name: String,
        api_key: String,
        base_url: String,
        reasoning_effort: Option<String>,
    ) -> Self {
        let provider_type = ProviderKind::from_id(&provider_name).unwrap_or(ProviderKind::Custom);
        let mut registry = Self {
            model: model.clone(),
            providers: vec![ProviderConfig {
                name: provider_name,
                provider_type,
                api_key,
                base_url,
                auth_header: "authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                timeout_secs: 300,
                models: vec![ModelEntry {
                    id: model,
                    alias: vec![],
                    max_tokens: None,
                    context_window: None,
                    supports_streaming: true,
                    supports_vision: false,
                    supports_audio: false,
                    supports_image_generation: false,
                    reasoning_efforts: Vec::new(),
                    default_effort: None,
                    cost_per_million_in: None,
                    cost_per_million_out: None,
                }],
                reasoning_effort,
            }],
            capabilities: Capabilities::default(),
            alias_map: HashMap::new(),
        };
        registry.build_alias_map();
        registry
    }

    /// Resolve the effective max output token budget for a model.
    ///
    /// Returns Some ONLY when the user explicitly configured `max_tokens` for
    /// this model in providers.toml. Returns None when unset — callers must
    /// then OMIT the field from the request body so the provider's official
    /// default applies (correct "no limit" semantics).
    ///
    /// ⚠️ Do NOT fall back to builtin metadata here: builtin `max_output_tokens`
    /// values (8192 for most providers) are conservative assumptions that
    /// truncated long thinking streams for reasoning models (see transport).
    pub fn get_max_output_tokens(&self, model_id: &str) -> Option<u32> {
        if let Some((_, model)) = self.find_model(model_id) {
            return model.max_tokens;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_provider_from_id() {
        assert_eq!(
            KnownProvider::from_id("deepseek"),
            Some(KnownProvider::DeepSeek)
        );
        assert_eq!(KnownProvider::from_id("kimi"), Some(KnownProvider::Kimi));
        assert_eq!(KnownProvider::from_id("unknown"), None);
        assert_eq!(KnownProvider::DeepSeek.as_str(), "deepseek");
        assert_eq!(KnownProvider::Kimi.as_str(), "kimi");
    }

    #[test]
    fn test_model_registry_alias_lookup() {
        let mut registry = ModelRegistry {
            model: "deepseek-v4-flash".to_string(),
            providers: vec![ProviderConfig {
                name: "deepseek".to_string(),
                provider_type: KnownProvider::DeepSeek,
                api_key: "test-key".to_string(),
                base_url: "https://api.deepseek.com".to_string(),
                auth_header: "authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                timeout_secs: 300,
                models: vec![ModelEntry {
                    id: "deepseek-v4-flash".to_string(),
                    alias: vec!["deepseek".to_string(), "default".to_string()],
                    max_tokens: None,
                    context_window: None,
                    supports_streaming: true,
                    supports_vision: false,
                    supports_audio: false,
                    supports_image_generation: false,
                    reasoning_efforts: Vec::new(),
                    default_effort: None,
                    cost_per_million_in: None,
                    cost_per_million_out: None,
                }],
                reasoning_effort: None,
            }],
            capabilities: Capabilities::default(),
            alias_map: Default::default(),
        };
        registry.build_alias_map();

        // Lookup by alias
        let (provider, model) = registry.find_model("deepseek").unwrap();
        assert_eq!(provider.name, "deepseek");
        assert_eq!(model.id, "deepseek-v4-flash");

        // Lookup by ID
        let (_provider2, model2) = registry.find_model("deepseek-v4-flash").unwrap();
        assert_eq!(model2.id, "deepseek-v4-flash");

        // List models
        let models = registry.list_models();
        assert_eq!(models.len(), 1);
        assert_eq!(
            models[0],
            ("deepseek".to_string(), "deepseek-v4-flash".to_string())
        );
    }

    #[test]
    fn test_model_registry_toml_roundtrip() {
        let toml_str = r#"
default_model = "kimi-for-coding"

[[providers]]
name = "kimi"
provider_type = "kimi"
api_key = "sk-test"
base_url = "https://api.kimi.com/coding/v1"

[[providers.models]]
id = "kimi-for-coding"
alias = ["kimi"]
supports_streaming = true
"#;

        let mut registry: ModelRegistry = toml::from_str(toml_str).unwrap();
        assert_eq!(registry.model, "kimi-for-coding");
        assert_eq!(registry.providers.len(), 1);
        assert_eq!(registry.providers[0].name, "kimi");
        assert_eq!(registry.providers[0].provider_type, KnownProvider::Kimi);

        // Need to manually build alias map (TOML deserialization does not call build_alias_map)
        registry.build_alias_map();

        // Alias lookup
        let (_provider, model) = registry.find_model("kimi").unwrap();
        assert_eq!(model.id, "kimi-for-coding");
    }

    /// Minimal ProviderConfig helper for candidate-lookup tests.
    fn test_provider(
        name: &str,
        kind: KnownProvider,
        models: &[(&str, &[&str])],
    ) -> ProviderConfig {
        ProviderConfig {
            name: name.to_string(),
            provider_type: kind,
            api_key: "test-key".to_string(),
            base_url: String::new(),
            auth_header: String::new(),
            auth_prefix: String::new(),
            timeout_secs: 300,
            models: models
                .iter()
                .map(|(id, aliases)| ModelEntry {
                    id: id.to_string(),
                    alias: aliases.iter().map(|s| s.to_string()).collect(),
                    max_tokens: None,
                    context_window: None,
                    supports_streaming: true,
                    supports_vision: false,
                    supports_audio: false,
                    supports_image_generation: false,
                    reasoning_efforts: Vec::new(),
                    default_effort: None,
                    cost_per_million_in: None,
                    cost_per_million_out: None,
                })
                .collect(),
            reasoning_effort: None,
        }
    }

    /// `test_provider` variant with explicit per-model context windows
    /// (`None` = the segment declares no window for that model) — context-window
    /// resolution fixtures need to control which same-id candidate carries a value.
    fn provider_with_windows(
        name: &str,
        kind: KnownProvider,
        models: &[(&str, Option<usize>)],
    ) -> ProviderConfig {
        let mut provider = test_provider(name, kind, &[]);
        provider.models = models
            .iter()
            .map(|(id, window)| ModelEntry {
                id: id.to_string(),
                alias: Vec::new(),
                max_tokens: None,
                context_window: *window,
                supports_streaming: true,
                supports_vision: false,
                supports_audio: false,
                supports_image_generation: false,
                reasoning_efforts: Vec::new(),
                default_effort: None,
                cost_per_million_in: None,
                cost_per_million_out: None,
            })
            .collect();
        provider
    }

    fn registry_with(providers: Vec<ProviderConfig>) -> ModelRegistry {
        let model = providers
            .first()
            .and_then(|p| p.models.first())
            .map(|m| m.id.clone())
            .unwrap_or_default();
        let mut registry = ModelRegistry {
            model,
            providers,
            capabilities: Capabilities::default(),
            alias_map: Default::default(),
        };
        registry.build_alias_map();
        registry
    }

    /// find_model_candidates resolves an alias to its canonical id and returns
    /// every segment publishing that id.
    #[test]
    fn test_find_model_candidates_alias_hit() {
        let registry = registry_with(vec![test_provider(
            "deepseek",
            KnownProvider::DeepSeek,
            &[("deepseek-v4-flash", &["deepseek", "default"])],
        )]);

        let candidates = registry.find_model_candidates("deepseek");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0.name, "deepseek");
        assert_eq!(candidates[0].1.id, "deepseek-v4-flash");

        // find_model on the alias is unchanged.
        let (provider, model) = registry.find_model("deepseek").unwrap();
        assert_eq!(provider.name, "deepseek");
        assert_eq!(model.id, "deepseek-v4-flash");
    }

    /// Cross-segment duplicate: candidates lists both, find_model keeps the
    /// segment-order first and only warns.
    #[test]
    fn test_find_model_candidates_cross_segment_duplicate() {
        let registry = registry_with(vec![
            test_provider("zhipu", KnownProvider::Zhipu, &[("glm-4.7", &["glm"])]),
            test_provider(
                "opencode-go",
                KnownProvider::OpenCodeGo,
                &[("glm-4.7", &[]), ("gpt-5.6-luna", &[])],
            ),
        ]);

        // Direct id match → both segments.
        let candidates = registry.find_model_candidates("glm-4.7");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].0.name, "zhipu");
        assert_eq!(candidates[1].0.name, "opencode-go");

        // Alias hit expands to canonical id → same full candidate set.
        let via_alias = registry.find_model_candidates("glm");
        assert_eq!(via_alias.len(), 2);
        assert_eq!(via_alias[0].0.name, "zhipu");
        assert_eq!(via_alias[1].0.name, "opencode-go");

        // Legacy find_model still returns segment-order first (compat) — the
        // ambiguity only emits a warning.
        let (provider, model) = registry.find_model("glm-4.7").unwrap();
        assert_eq!(provider.name, "zhipu");
        assert_eq!(model.id, "glm-4.7");
    }

    /// No match → empty vec (never falls back to the default model).
    #[test]
    fn test_find_model_candidates_no_match() {
        let registry = registry_with(vec![test_provider(
            "zhipu",
            KnownProvider::Zhipu,
            &[("glm-4.7", &[])],
        )]);

        assert!(registry.find_model_candidates("no-such-model").is_empty());
        assert_eq!(registry.find_model_candidates("glm-4.7").len(), 1);
    }

    /// Same id across segments where the segment-order first hit declares no
    /// window: the lookup must keep scanning siblings instead of returning None
    /// (a valueless hit must never mask a sibling — P0-a root cause).
    #[test]
    fn test_resolve_context_window_skips_valueless_candidate() {
        let registry = registry_with(vec![
            provider_with_windows("custom", KnownProvider::Custom, &[("m", None)]),
            provider_with_windows("deepseek", KnownProvider::DeepSeek, &[("m", Some(64_000))]),
        ]);

        // Legacy first-match stops at the "custom" hit and would fall through.
        assert_eq!(registry.find_model("m").unwrap().1.context_window, None);
        assert_eq!(registry.resolve_context_window(None, "m"), Some(64_000));
    }

    /// Provider-exact value wins; an unknown provider (or one that publishes the
    /// model without a value) falls back to the same-id candidate scan; nothing
    /// declares a value → None.
    #[test]
    fn test_resolve_context_window_provider_exact_then_fallback() {
        let registry = registry_with(vec![
            provider_with_windows("deepseek", KnownProvider::DeepSeek, &[("m", Some(64_000))]),
            provider_with_windows("custom", KnownProvider::Custom, &[("m", Some(131_072))]),
        ]);

        // Provider-exact beats the segment-order first hit.
        assert_eq!(
            registry.resolve_context_window(Some("custom"), "m"),
            Some(131_072)
        );
        // Empty provider = "no hint" (same as None).
        assert_eq!(registry.resolve_context_window(Some(""), "m"), Some(64_000));
        // Provider not in the registry → candidate scan (segment order).
        assert_eq!(
            registry.resolve_context_window(Some("gone-provider"), "m"),
            Some(64_000)
        );

        // Provider-exact hit WITHOUT a value must not short-circuit the scan.
        let valueless = registry_with(vec![
            provider_with_windows("custom", KnownProvider::Custom, &[("m", None)]),
            provider_with_windows("deepseek", KnownProvider::DeepSeek, &[("m", Some(64_000))]),
        ]);
        assert_eq!(
            valueless.resolve_context_window(Some("custom"), "m"),
            Some(64_000)
        );

        let unknown = registry_with(vec![provider_with_windows(
            "custom",
            KnownProvider::Custom,
            &[("m", None)],
        )]);
        assert_eq!(unknown.resolve_context_window(None, "m"), None);
        assert_eq!(unknown.resolve_context_window(None, "no-such-model"), None);
    }

    /// Alias lookups keep the same candidate-scan semantics (alias hit expands
    /// to the canonical id before the window is picked up).
    #[test]
    fn test_resolve_context_window_alias_candidate() {
        let mut aliased =
            provider_with_windows("deepseek", KnownProvider::DeepSeek, &[("m", None)]);
        aliased.models[0].alias = vec!["m-alias".to_string()];
        let registry = registry_with(vec![
            aliased,
            provider_with_windows("custom", KnownProvider::Custom, &[("m", Some(32_000))]),
        ]);

        assert_eq!(
            registry.resolve_context_window(None, "m-alias"),
            Some(32_000)
        );
    }
}
