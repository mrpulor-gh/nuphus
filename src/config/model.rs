//! Model configuration data structures
//!
//! Supports multiple Providers, multiple models, alias mapping

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Canonical Provider type — re-exported from `api::ProviderKind` for config-layer consumers.
pub use crate::api::ProviderKind;
pub use ProviderKind as KnownProvider;

/// Provenance of one model entry inside its provider segment.
///
/// `Auto` — written by the provider `/v1/models` sync flow. Legacy entries
/// without a `source` key deserialize to `Auto` (serde default), so pre-existing
/// configs keep their old "may be reconciled away" semantics.
/// `Manual` — the user added the id through the「添加模型」entry point; an
/// explicit refresh never removes it (it may live outside the official catalog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ModelSource {
    #[default]
    Auto,
    Manual,
}

impl ModelSource {
    /// Stable wire/TOML representation (`"auto"` / `"manual"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelSource::Auto => "auto",
            ModelSource::Manual => "manual",
        }
    }
}

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
    /// Entry provenance — see [`ModelSource`]. `#[serde(default)]` keeps configs
    /// written before this field existed deserializing as `auto`.
    #[serde(default)]
    pub source: ModelSource,
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
    /// 段级自定义请求头（TOML 落盘为嵌套表 `[providers.<段>.extra_headers]`），
    /// 中转站/网关要求的附加标头（如 `X-Gateway`）逐字注入每个请求。
    /// BTreeMap 保证落盘键序稳定；旧配置无此键 → serde default 空 map。
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
    /// 可选 OAuth2 授权配置（Authorization Code + PKCE + 本地回调）。
    /// TOML 落盘为嵌套表 `[providers.<段>.oauth]`；令牌字段与 api_key 同一套
    /// DPAPI 加密口径（from_toml 透明解密）。None = 该段走静态 api_key 鉴权。
    #[serde(default)]
    pub oauth: Option<ProviderOAuth>,
}

fn default_timeout() -> u64 {
    300
}

/// OAuth2 通用接入配置（Authorization Code + PKCE，本地回调）。
///
/// 五个配置项（authorize_url/token_url/client_id/scopes/use_pkce + redirect_port）
/// 由用户在表单填写（Agent 可代填 providers.toml）；三个令牌字段是运行时状态：
/// 落盘经 DPAPI 加密（`enc:` 前缀，与 api_key 同款），from_toml 读入即透明解密。
///
/// 演进关系：`transports/responses/config.rs` 的 ResponsesConfig 注明 OAuth 令牌
/// 走 credentials 服务是 P3 方向；当前以段内加密存储落地（不过早抽象），届时
/// 令牌的「读取出口」收敛到 `config::oauth::ensure_fresh_oauth_token` 一处，上层
/// 无需感知存储形态即可平移。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderOAuth {
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    /// 空格分隔的 scope 列表；空 = 授权请求不带 scope 参数。
    #[serde(default)]
    pub scopes: String,
    /// PKCE 开关（RFC 7636）；缺省 true（TOML 省略该键 = 开启）。
    #[serde(default = "default_true")]
    pub use_pkce: bool,
    /// 本地回调端口；None = 自动选择空闲端口。
    #[serde(default)]
    pub redirect_port: Option<u16>,
    /// access token（落盘 `enc:` DPAPI，内存为明文）。
    #[serde(default)]
    pub access_token: String,
    /// refresh token（落盘 `enc:` DPAPI，内存为明文）。
    #[serde(default)]
    pub refresh_token: String,
    /// access token 过期时刻（unix 秒）；None = 未知（视为需刷新）。
    #[serde(default)]
    pub expires_at: Option<i64>,
}

impl ProviderOAuth {
    /// ���个授权配置项是否齐备（发起授权登录的前置条件）。
    /// 令牌字段不参与判定——它们是登录产物，不是配置。
    pub fn config_complete(&self) -> bool {
        !self.authorize_url.trim().is_empty()
            && !self.token_url.trim().is_empty()
            && !self.client_id.trim().is_empty()
    }

    /// 是否已登录（持有 access token）。
    pub fn is_logged_in(&self) -> bool {
        !self.access_token.is_empty()
    }
}

/// 敏感字段三态解密（api_key 与 OAuth 令牌共用，禁止复制粘贴出第二份）。
///
/// `enc:` 前缀 → DPAPI 解密（失败则清空 + 告警，视为未配置）；非密文（旧明文
/// 配置 / 空串）原样保留。`label` 仅用于告警定位字段，不携带任何敏感值。
pub(crate) fn decrypt_credential_three_state(field: &mut String, provider: &str, label: &str) {
    let encrypted = field.starts_with("enc:");
    match crate::cookies::decrypt_secret(field) {
        Some(dec) => *field = dec,
        None if encrypted => {
            tracing::warn!(
                "[config] provider '{}' 的 {} 无法解密，视为未配置（请重新配置）",
                provider,
                label
            );
            field.clear();
        }
        None => {}
    }
}

/// 按能力独立配置模型（不配则使用 model）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// 图像理解模型
    #[serde(default)]
    pub vision: String,
    /// 图像理解模型所属服务商（旧配置为空时按模型 ID 兼容解析）
    #[serde(default)]
    pub vision_provider: String,
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
    /// 配置文件来源路径（from_toml 记录；其它构造路径为 None）。
    /// transport 构造链（factory）据此定位段配置，完成 OAuth 令牌的
    /// 过期刷新与注入（见 `config::oauth::ensure_fresh_oauth_token`）。
    #[serde(skip)]
    pub source_path: Option<std::path::PathBuf>,
}

impl ModelRegistry {
    /// Load from TOML config file
    pub fn from_toml(path: &str) -> crate::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::NuphusError::Config(format!("read config failed: {e}")))?;
        let mut doc: toml::Value = content
            .parse()
            .map_err(|e| crate::NuphusError::Config(format!("parse config failed: {e}")))?;
        // 归一化：provider_type 是协议维度（custom/local/官方 id），实例身份只由 name 承载。
        // 早期写入路径曾把实例名（custom-xxx）写进 provider_type，而 ProviderKind 无该变体，
        // 会让整份 providers.toml 反序列化失败（配置全丢）。此处就地折回 custom，保证
        // 老配置仍可加载；实例名仍完整保留在 name 上，路由不受影响。
        if let Some(providers) = doc.get_mut("providers").and_then(|p| p.as_array_mut()) {
            for provider in providers.iter_mut() {
                let needs_fold = provider
                    .get("provider_type")
                    .and_then(|t| t.as_str())
                    .map(|t| t.starts_with("custom-"))
                    .unwrap_or(false);
                if needs_fold {
                    if let Some(map) = provider.as_table_mut() {
                        map.insert(
                            "provider_type".to_string(),
                            toml::Value::String("custom".to_string()),
                        );
                    }
                }
            }
        }
        let mut registry: Self = serde::Deserialize::deserialize(doc.clone())
            .map_err(|e| crate::NuphusError::Config(format!("parse config failed: {e}")))?;
        registry.source_path = Some(std::path::PathBuf::from(path));
        // 敏感字段透明解密（api_key + OAuth 令牌共用一套三态口径）：
        // 落盘为 `enc:v1:`（DPAPI）时还原明文；旧明文配置原样兼容。
        // 旧版 `enc:`（无版本号）密文一并迁移解密；密文但解密失败视为未配置（触发重新导入）。
        // 环境变量来源（from_env）不经此路径，无需解密。
        for p in &mut registry.providers {
            decrypt_credential_three_state(&mut p.api_key, &p.name, "api_key");
            if let Some(oauth) = p.oauth.as_mut() {
                decrypt_credential_three_state(
                    &mut oauth.access_token,
                    &p.name,
                    "oauth.access_token",
                );
                decrypt_credential_three_state(
                    &mut oauth.refresh_token,
                    &p.name,
                    "oauth.refresh_token",
                );
            }
        }
        // 模型真值 = [agent_models].leader（主模型，mode 绑定单一数据源）。
        // providers.toml 顶层 model 字段已退役：不构成覆盖层。leader 可用时以 leader
        // 为准（覆盖 serde 读入的顶层旧值）；leader 空（旧文件未迁移绑定）→ 保留顶层
        // 历史值作一次性兼容兜底，不写回、不参与任何优先级比较。
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
                        .unwrap_or_else(|_| "deepseek-flash".to_string()),
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
                    source: ModelSource::Auto,
                }],
                reasoning_effort: None,
                extra_headers: BTreeMap::new(),
                oauth: None,
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
                    source: ModelSource::Auto,
                }],
                reasoning_effort: None,
                extra_headers: BTreeMap::new(),
                oauth: None,
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
                    source: ModelSource::Auto,
                }],
                reasoning_effort: None,
                extra_headers: BTreeMap::new(),
                oauth: None,
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
                    source: ModelSource::Auto,
                }],
                reasoning_effort: None,
                extra_headers: BTreeMap::new(),
                oauth: None,
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
                    source: ModelSource::Auto,
                }],
                reasoning_effort: None,
                extra_headers: BTreeMap::new(),
                oauth: None,
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
                    source: ModelSource::Auto,
                }],
                reasoning_effort: None,
                extra_headers: BTreeMap::new(),
                oauth: None,
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
            // env 来源没有配置文件：OAuth 令牌注入路径据此跳过（无盘可刷新）
            source_path: None,
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
                    source: ModelSource::Auto,
                }],
                reasoning_effort,
                extra_headers: BTreeMap::new(),
                oauth: None,
            }],
            capabilities: Capabilities::default(),
            alias_map: HashMap::new(),
            // from_single 无文件来源（内存构造）：OAuth 刷新链路自然跳过。
            source_path: None,
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

    /// 兼容：早期写入路径可能把实例名写进 provider_type（应为协议类型 custom）。
    /// 该值无对应 ProviderKind 变体，若不归一化整份 providers.toml 会反序列化失败。
    #[test]
    fn from_toml_folds_custom_instance_provider_type_back_to_custom() {
        let path = std::env::temp_dir().join(format!(
            "nuphus_registry_custom_inst_{}.toml",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::write(
            &path,
            r#"
[[providers]]
name = "custom-team-a"
provider_type = "custom-team-a"
api_key = ""
base_url = "https://gw.example/v1"

[[providers.models]]
id = "gpt-4o"
"#,
        )
        .unwrap();

        let registry = ModelRegistry::from_toml(path.to_str().unwrap()).unwrap();
        let custom = registry
            .providers
            .iter()
            .find(|p| p.name == "custom-team-a")
            .expect("instance segment must survive loading");
        assert_eq!(
            custom.provider_type,
            KnownProvider::Custom,
            "provider_type 必须折回协议类型 custom"
        );
        assert_eq!(custom.name, "custom-team-a", "实例名由 name 承载，不得丢失");
        assert!(
            registry
                .find_model_for_provider("custom-team-a", "gpt-4o")
                .is_some(),
            "折回后仍可按实例名精确解析模型"
        );

        std::fs::remove_file(&path).ok();
    }

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

    /// ProviderOAuth TOML 序列化往返：`use_pkce` 缺省 = true（省略键不改变语义），
    /// 显式 false 必须保真；令牌/过期时间原样往返（加密由读写路径负责，serde 层不感知）。
    #[test]
    fn test_provider_oauth_toml_roundtrip() {
        let toml_with_default_pkce = r#"
authorize_url = "https://sso.example.com/authorize"
token_url = "https://sso.example.com/token"
client_id = "nuphus-cli"
"#;
        let oauth: ProviderOAuth = toml::from_str(toml_with_default_pkce).unwrap();
        assert!(oauth.use_pkce, "缺省 use_pkce 必须为 true");
        assert_eq!(oauth.scopes, "");
        assert_eq!(oauth.redirect_port, None);
        assert_eq!(oauth.access_token, "");
        assert_eq!(oauth.expires_at, None);

        let toml_explicit = r#"
authorize_url = "https://sso.example.com/authorize"
token_url = "https://sso.example.com/token"
client_id = "nuphus-cli"
scopes = "read write"
use_pkce = false
redirect_port = 19110
access_token = "at-1"
refresh_token = "rt-1"
expires_at = 1800000000
"#;
        let oauth: ProviderOAuth = toml::from_str(toml_explicit).unwrap();
        assert!(!oauth.use_pkce, "显式 false 必须保真");
        assert_eq!(oauth.scopes, "read write");
        assert_eq!(oauth.redirect_port, Some(19110));
        assert_eq!(oauth.access_token, "at-1");
        assert_eq!(oauth.refresh_token, "rt-1");
        assert_eq!(oauth.expires_at, Some(1_800_000_000));

        // 往返：serde 序列化 → 反序列化等值
        let re: ProviderOAuth = toml::from_str(&toml::to_string(&oauth).unwrap()).unwrap();
        assert_eq!(re.access_token, oauth.access_token);
        assert_eq!(re.refresh_token, oauth.refresh_token);
        assert_eq!(re.expires_at, oauth.expires_at);
        assert!(!re.use_pkce);
    }

    /// 旧 providers.toml（无 oauth 键）读入 → oauth == None，不破坏既有段。
    #[test]
    fn test_provider_config_oauth_defaults_to_none() {
        let doc: toml::Value = r#"
name = "custom-a"
provider_type = "custom"
api_key = "sk-1"
base_url = "https://relay.example.com/v1"

[[providers.models]]
id = "m1"
"#
        .parse()
        .unwrap();
        // 经与 from_toml 相同的反序列化入口验证缺省行为
        let cfg: ProviderConfig = serde::Deserialize::deserialize(doc).unwrap();
        assert!(cfg.oauth.is_none());
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
                    source: ModelSource::Auto,
                }],
                reasoning_effort: None,
                extra_headers: BTreeMap::new(),
                oauth: None,
            }],
            capabilities: Capabilities::default(),
            alias_map: Default::default(),
            source_path: None,
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

    #[test]
    fn vision_provider_is_optional_for_legacy_configs() {
        let legacy: ModelRegistry = toml::from_str(
            r#"
[[providers]]
name = "custom"
provider_type = "custom"
api_key = ""

[[providers.models]]
id = "m"

[capabilities]
vision = "m"
"#,
        )
        .unwrap();
        assert_eq!(legacy.capabilities.vision, "m");
        assert!(legacy.capabilities.vision_provider.is_empty());

        let current: ModelRegistry = toml::from_str(
            r#"
[[providers]]
name = "custom"
provider_type = "custom"
api_key = ""

[[providers.models]]
id = "m"

[capabilities]
vision = "m"
vision_provider = "custom"
"#,
        )
        .unwrap();
        assert_eq!(current.capabilities.vision_provider, "custom");
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
                    source: ModelSource::Auto,
                })
                .collect(),
            reasoning_effort: None,
            extra_headers: BTreeMap::new(),
            oauth: None,
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
                source: ModelSource::Auto,
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
            source_path: None,
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
