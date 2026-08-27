//! Model configuration data structures
//!
//! Supports multiple Providers, multiple models, alias mapping

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// API 协议类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// OpenAI Chat Completions 兼容协议 (/chat/completions, Bearer Token)
    OpenAIChatCompletions,
}

/// Canonical Provider type — re-exported from `api::ProviderKind` for config-layer consumers.
pub use crate::api::ProviderKind;
/// Backward-compatibility alias for [`ProviderKind`].
pub use ProviderKind as ProviderType;
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
    pub provider_type: ProviderType,
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

    /// Find model configuration
    pub fn find_model(&self, model_id: &str) -> Option<(&ProviderConfig, &ModelEntry)> {
        // Check alias first
        if let Some((provider_name, real_id)) = self.alias_map.get(model_id) {
            let provider = self.providers.iter().find(|p| &p.name == provider_name)?;
            let model = provider.models.iter().find(|m| &m.id == real_id)?;
            return Some((provider, model));
        }
        // Then check direct match
        for provider in &self.providers {
            if let Some(model) = provider.models.iter().find(|m| m.id == model_id) {
                return Some((provider, model));
            }
        }
        None
    }

    /// Get default model
    pub fn default_model_config(&self) -> Option<(&ProviderConfig, &ModelEntry)> {
        self.find_model(&self.model)
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

    /// 获取指定能力的模型（不配则回退到 model）
    pub fn get_capability(&self, capability: &str) -> String {
        let model_id = match capability {
            "vision" if !self.capabilities.vision.is_empty() => &self.capabilities.vision,
            "stt" if !self.capabilities.stt.is_empty() => &self.capabilities.stt,
            "tts" if !self.capabilities.tts.is_empty() => &self.capabilities.tts,
            _ => &self.model,
        };
        model_id.to_string()
    }

    /// Check whether the configured vision capability model exists and supports vision.
    pub fn vision_available(&self) -> bool {
        let vm = &self.capabilities.vision;
        if vm.is_empty() {
            return false;
        }
        self.find_model(vm)
            .map(|(_, m)| m.supports_vision)
            .unwrap_or(false)
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

    /// Get context window size for a model.
    /// Prefers explicitly configured context_window, then tries the metadata
    /// table in `ProviderRegistry::builtin()`, and finally falls back to a
    /// reasonable default (128K).
    pub fn get_context_window(&self, model_id: &str) -> usize {
        // 1. Check registry for explicitly configured context_window
        if let Some((_, model)) = self.find_model(model_id) {
            if let Some(window) = model.context_window {
                return window;
            }
        }
        // 2. Try built-in ProviderRegistry metadata
        let registry = crate::config::registry::ProviderRegistry::builtin();
        if let Some((_, meta)) = registry.find_model(model_id) {
            return meta.context_window as usize;
        }
        // 3. Fallback
        128_000
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
}
