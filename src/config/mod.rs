//! 配置模块 — 模型配置加载与管理

pub mod last_model;
pub mod model;
pub mod oauth;
pub mod preferences;
pub mod provider;
pub mod providers;
pub mod registry;
pub use last_model::{load_last_model_provider, record_last_model, resolve_model_provider_core};
pub use model::*;
pub use preferences::{
    normalize_pinned_sessions, normalize_session_group_order, normalize_session_sort_key,
    BrowserIdentity, ProjectBookmark, UserPreferences,
};

use std::path::PathBuf;

/// Serializes in-process providers.toml read/modify/write transactions across
/// desktop commands, OAuth refresh and background capability discovery.
/// Never hold this guard across network I/O, await, or another config writer.
static PROVIDER_CONFIG_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn lock_provider_config() -> std::sync::MutexGuard<'static, ()> {
    PROVIDER_CONFIG_WRITE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Replace a complete providers config without exposing a truncated document to
/// lock-free readers. Callers must hold lock_provider_config for the whole
/// read/modify/write transaction. Preserve existing permissions; new Unix files
/// are private because providers config may contain credentials.
pub fn write_provider_config(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut created = false;
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        created = true;
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(content.as_bytes())?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if created && result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// 桌面端注入的规范配置路径（最高优先级）。
/// 防止 cwd / exe_dir 下无关的 config.toml 劫持模型注册表。
/// CLI 不调用 set_config_override，搜索行为保持不变。
static CONFIG_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// 设置规范配置文件路径（进程级，仅桌面端启动时调用一次）。
pub fn set_config_override(path: PathBuf) {
    let _ = CONFIG_OVERRIDE.set(path);
}

/// 返回配置文件的搜索路径列表（按优先级从高到低）。
/// load_registry 与 toml_ops 共享此列表，确保路径探测逻辑唯一。
/// 优先级: override(桌面端) > exe_dir > cwd > ~/.config/nuphus > ~/.nuphus > AppData
pub fn config_search_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    let home = std::env::var("HOME").unwrap_or_default();

    // 0. 显式覆盖（桌面端锚定 AppData providers.toml）
    if let Some(p) = CONFIG_OVERRIDE.get() {
        paths.push(p.clone());
    }

    // 1. 可执行文件目录
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("config.toml"));
            paths.push(dir.join("nuphus.toml"));
        }
    }

    // 2. 当前工作目录
    paths.push(PathBuf::from("config.toml"));
    paths.push(PathBuf::from("nuphus.toml"));

    // 3. 用户配置目录
    paths.push(PathBuf::from(format!(
        "{}/.config/nuphus/config.toml",
        home
    )));
    paths.push(PathBuf::from(format!("{}/.nuphus/config.toml", home)));

    // 4. AppData Roaming (Windows desktop)
    // providers.toml first — the canonical TOML config; config.toml is the JSON key file (plaintext).
    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join("nuphus").join("providers.toml"));
        paths.push(config_dir.join("nuphus").join("config.toml"));
    }

    paths
}

/// 自动发现配置文件并加载模型注册表
/// 优先级: exe_dir > cwd > ~/.config/nuphus > ~/.nuphus > 环境变量
pub fn load_registry() -> crate::Result<ModelRegistry> {
    for path in &config_search_paths() {
        if path.exists() {
            let path_str = path.to_string_lossy().to_string();
            tracing::info!("loading config from: {}", path_str);
            match ModelRegistry::from_toml(&path_str) {
                Ok(registry) => return Ok(registry),
                Err(e) => {
                    tracing::warn!(
                        "failed to load config from {}: {} — trying next",
                        path_str,
                        e
                    );
                    continue;
                }
            }
        }
    }

    tracing::info!("no config file found, falling back to environment variables");
    ModelRegistry::from_env()
}

/// Resolve the effective max output token budget for a model.
///
/// Transport layer entry point — called on every request build, so results are
/// cached per model id. **缓存按配置文件的身份（路径 + mtime + 长度）作废**：
/// 配置是可在运行期被改写的权威源，永久缓存会让「改了 max_tokens / 新增模型」
/// 要重启才生效（与「读一次就冻结的副本」同一类缺陷）。
/// Priority follows `ModelRegistry::get_max_output_tokens` (see model.rs).
/// `provider` is the routing binding of the calling transport (known at request
/// build time) — same-named models across providers then resolve to the exact
/// segment instead of the first segment-order hit.
pub fn resolve_max_output_tokens(model_id: &str, provider: Option<&str>) -> Option<u32> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    /// 规范配置文件身份。文件未变 → 缓存有效；变了（或从无到有）→ 整表作废。
    type Identity = Option<(std::path::PathBuf, std::time::SystemTime, u64)>;

    struct Cache {
        identity: Identity,
        /// key = (provider, model_id)：同一 model id 在不同段可配不同 max_tokens，
        /// 缓存必须按绑定区分，否则段间会互相串值。
        entries: HashMap<(Option<String>, String), Option<u32>>,
    }

    fn current_identity() -> Identity {
        let path = config_search_paths().into_iter().find(|p| p.exists())?;
        let meta = std::fs::metadata(&path).ok()?;
        Some((path, meta.modified().ok()?, meta.len()))
    }

    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        Mutex::new(Cache {
            identity: None,
            entries: HashMap::new(),
        })
    });

    let resolve_uncached = || {
        load_registry()
            .ok()
            .and_then(|r| r.get_max_output_tokens(provider, model_id))
    };

    let Ok(mut guard) = cache.lock() else {
        return resolve_uncached();
    };
    let identity = current_identity();
    if guard.identity != identity {
        guard.identity = identity;
        guard.entries.clear();
    }
    let cache_key = (provider.map(str::to_string), model_id.to_string());
    if let Some(v) = guard.entries.get(&cache_key) {
        return *v;
    }
    let resolved = resolve_uncached();
    guard.entries.insert(cache_key, resolved);
    resolved
}
/// Resolve whether the model bound to a capability carries the requested
/// capability — the single disambiguation entry point for capability probes.
///
/// Same-named models can be published by several providers; the legacy
/// `find_model` first-segment-order lookup and the scattered `.unwrap_or(false)`
/// fallbacks that used to sit at every call site silently missed the sibling
/// that declared the capability. This function replaces both:
///
/// - `provider` given (`Some`/non-empty) → that provider's entry decides.
/// - otherwise every same-id candidate is scanned and **any** candidate that
///   declares the capability wins (conservative: never under-report).
/// - unknown model (`find_model_candidates` empty, e.g. a local model absent
///   from the registry) → `presumed` (callers keep a stable default: `false`
///   for capability gating, `true` for unrestricted metadata fields).
///
/// `None` is never returned: the boolean answer is well-defined for every input
/// ("unknown model" is folded into `presumed`). Callers that must distinguish
/// "unknown" from "verified absent" call `ModelRegistry::resolve_capability`
/// directly.
pub fn resolve_capability(
    registry: &ModelRegistry,
    provider: Option<&str>,
    model_id: &str,
    predicate: impl Fn(&ModelEntry) -> bool,
    presumed: bool,
) -> bool {
    if registry.find_model_candidates(model_id).is_empty() {
        // 未知模型：注册表中无该 id（本地/探测中模型）→ 保持既有默认。
        return presumed;
    }
    registry
        .resolve_capability(provider, model_id, predicate)
        .is_some()
}

/// 视觉理解策略
pub enum VisionStrategy {
    /// 主模型直接支持多模态，不需要额外配置
    Main,
    /// 使用 capabilities.vision 配置的独立视觉模型
    Capability(String),
    /// 未配置任何视觉能力
    None,
}

/// 单点判定：当前环境能用什么方式理解图片
///
/// 逻辑：
/// 1. 加载 registry (load_registry)
/// 2. 显式配置的 capabilities.vision 优先 → VisionStrategy::Capability(模型名)
///    （用户显式指定 > 自动推断；专用视觉模型通常比推理主模型快得多）
/// 3. 未配置，检查主模型的 supports_vision → VisionStrategy::Main
/// 4. 都没有 → VisionStrategy::None
///
/// 注意：不自动遍历所有 provider 找视觉模型。用户未显式配置且主模型不支持
/// 时直接报 None，让 desktop_vision 明确告知用户需要配置，而非静默使用
/// 一个用户没选的模型。
pub fn resolve_vision_strategy() -> VisionStrategy {
    let registry = match load_registry() {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("resolve_vision_strategy: load_registry failed: {e}");
            return VisionStrategy::None;
        }
    };

    // 显式配置的 capabilities.vision 优先
    let cap_vision = &registry.capabilities.vision;
    if !cap_vision.is_empty() {
        return VisionStrategy::Capability(cap_vision.clone());
    }

    // 主模型直接支持多模态——绑定取权威成对解析（resolve_main_binding），
    // 不查 [last_model] 影子表（二元组化 P2-a）
    let main_supports_vision = registry
        .resolve_main_binding()
        .map(|(provider, model)| {
            resolve_capability(
                &registry,
                Some(provider.as_str()),
                &model,
                |m| m.supports_vision,
                false,
            )
        })
        .unwrap_or(false);
    if main_supports_vision {
        return VisionStrategy::Main;
    }

    VisionStrategy::None
}

/// 返回显式配置的视觉模型 provider。旧配置没有该字段时返回 None，
/// 由调用方使用 legacy 的 model-id 查找逻辑兼容处理。
pub fn resolve_vision_provider() -> Option<String> {
    load_registry().ok().and_then(|registry| {
        if registry.capabilities.vision.is_empty()
            || registry.capabilities.vision_provider.is_empty()
        {
            None
        } else {
            Some(registry.capabilities.vision_provider)
        }
    })
}

// ── 生成能力（image_generation / video_generation）解析 ──

/// 生成能力策略（图像 / 视频生成共用）。
///
/// 与 [`VisionStrategy`] 的关键差别：**没有 `Main` 分支**。视觉可以落到
/// 「Leader 模型本身支持多模态」，生成端点（MiniMax 私有协议）与主模型是否
/// 聪明毫不相干——回落主模型只会把用户的生成请求打到错的 API 上。因此未绑定
/// 就是 `None`，由调用方明确报错并列出候选，绝不静默发现、绝不静默挑选。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerationStrategy {
    /// 使用 `capabilities.image_generation` / `capabilities.video_generation`
    /// 显式绑定的生成模型
    Capability(String),
    /// 未绑定任何生成能力
    None,
}

/// 生成能力种类（与 `capabilities` 的两个生成字段一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationKind {
    Image,
    Video,
}

impl GenerationKind {
    /// 能力字段名（`capabilities.<key>`，与前端 `setCapabilityBinding` 的 kind 一致）
    pub fn capability_key(self) -> &'static str {
        match self {
            GenerationKind::Image => "image_generation",
            GenerationKind::Video => "video_generation",
        }
    }

    /// 报错文案里的中文名
    pub fn label(self) -> &'static str {
        match self {
            GenerationKind::Image => "图片生成",
            GenerationKind::Video => "视频生成",
        }
    }

    /// 该能力在 `Capabilities` 里的 (模型字段, provider 归属字段)。
    fn fields(self, caps: &Capabilities) -> (&str, &str) {
        match self {
            GenerationKind::Image => (
                caps.image_generation.as_str(),
                caps.image_generation_provider.as_str(),
            ),
            GenerationKind::Video => (
                caps.video_generation.as_str(),
                caps.video_generation_provider.as_str(),
            ),
        }
    }
}

/// 报错文案里直接写 `{kind}`（= label），不必在每个 format! 里重复中文名。
impl std::fmt::Display for GenerationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// 生成能力绑定（模型 + provider 归属）。
///
/// `model` 为空 = 用户从未在模型界面绑定过这一能力；`provider` 为空 = 旧配置
/// 只写了模型 id，由调用方走候选消歧。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GenerationBinding {
    pub model: String,
    pub provider: Option<String>,
}

impl GenerationBinding {
    /// 未绑定（用户没在模型界面配过这一能力）
    pub fn is_unbound(&self) -> bool {
        self.model.is_empty()
    }
}

/// 绑定非空 → Capability；空绑定 → None。不遍历 provider、不猜测、不回落主模型。
fn bound_strategy(bound_model: &str) -> GenerationStrategy {
    if bound_model.is_empty() {
        GenerationStrategy::None
    } else {
        GenerationStrategy::Capability(bound_model.to_string())
    }
}

fn resolve_generation_strategy(kind: GenerationKind) -> GenerationStrategy {
    match load_registry() {
        Ok(registry) => {
            let (model, _provider) = kind.fields(&registry.capabilities);
            bound_strategy(model)
        }
        Err(e) => {
            tracing::warn!(
                "resolve_{}_strategy: load_registry failed: {e}",
                kind.capability_key()
            );
            GenerationStrategy::None
        }
    }
}

/// 图片生成策略：`capabilities.image_generation` 显式绑定优先，未绑定 → None。
///
/// 与 [`resolve_vision_strategy`] 同构（显式绑定优先、未绑定 None、不静默发现），
/// 区别仅在没有主模型回落——见 [`GenerationStrategy`]。
pub fn resolve_image_generation_strategy() -> GenerationStrategy {
    resolve_generation_strategy(GenerationKind::Image)
}

/// 视频生成策略：`capabilities.video_generation` 显式绑定优先，未绑定 → None。
pub fn resolve_video_generation_strategy() -> GenerationStrategy {
    resolve_generation_strategy(GenerationKind::Video)
}

fn resolve_generation_provider(kind: GenerationKind) -> Option<String> {
    let registry = load_registry().ok()?;
    let (model, provider) = kind.fields(&registry.capabilities);
    if model.is_empty() || provider.is_empty() {
        None
    } else {
        Some(provider.to_string())
    }
}

/// 显式绑定的图片生成 provider。旧配置只有模型 id 没有归属时返回 None，
/// 由调用方走候选消歧（与 [`resolve_vision_provider`] 同构）。
pub fn resolve_image_generation_provider() -> Option<String> {
    resolve_generation_provider(GenerationKind::Image)
}

/// 显式绑定的视频生成 provider（同上）。
pub fn resolve_video_generation_provider() -> Option<String> {
    resolve_generation_provider(GenerationKind::Video)
}

/// 从已加载的 registry 读某一生成能力的绑定，供工具层凭证解析使用。
///
/// 之所以不直接让工具层裸读 `registry.capabilities.*`：**绑定缺失**与
/// **绑定已失效**（provider 段被删 / 模型条目被改名）是两种错误，前者要引导
/// 去模型界面绑定，后者要指名哪个绑定坏了。分开返回，调用方的文案才能说准，
/// 而不是让用户拿着一个悬空 model id 去打一个不存在的端点。
///
/// 与 [`resolve_image_generation_strategy`] / [`resolve_image_generation_provider`]
/// （及 video 版）是同一份字段语义的两种取法：那四个函数自己读盘、给「只需要
/// 模型名 / provider 名」的调用方；本函数接一个已加载的 registry，给需要
/// 同一次加载里顺带做模型查找的工具层（避免读两次盘）。
pub fn resolve_generation_binding(
    registry: &ModelRegistry,
    kind: GenerationKind,
) -> Result<GenerationBinding, String> {
    let (model, provider) = kind.fields(&registry.capabilities);
    if model.is_empty() || provider.is_empty() {
        // 未绑定（或只绑了半个）——交由调用方报「去模型界面绑定」。
        return Ok(GenerationBinding::default());
    }
    // 绑定必须仍能在 registry 中解析，否则显式失败。
    if registry.find_model_for_provider(provider, model).is_none() {
        return Err(format!(
            "已绑定的{}模型 {provider}/{model} 在模型配置中已不存在（provider 段被删除或模型条目被改名）。\n\
             请去模型界面 → 图像音频模型 → {} 重新绑定。",
            kind.label(),
            kind.label()
        ));
    }
    Ok(GenerationBinding {
        model: model.to_string(),
        provider: Some(provider.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 双 provider 注册表：同 id 模型跨段重名（精确绑定的既有语义见 model.rs 测试）。
    fn dual_provider_registry() -> ModelRegistry {
        toml::from_str(
            r#"
model = "main-model"

[[providers]]
name = "seg-a"
provider_type = "minimax"
api_key = "key-a"
base_url = "https://a.example/v1"

[[providers.models]]
id = "image-01"
supports_image_generation = true

[[providers]]
name = "seg-b"
provider_type = "custom"
api_key = "key-b"
base_url = "https://b.example/v1"

[[providers.models]]
id = "image-01"
supports_image_generation = true

[capabilities]
image_generation = "image-01"
image_generation_provider = "seg-b"
"#,
        )
        .expect("registry toml should parse")
    }

    fn with_capabilities(caps: &str) -> ModelRegistry {
        toml::from_str(&format!(
            r#"
[[providers]]
name = "seg-a"
provider_type = "minimax"
api_key = "key-a"
base_url = "https://a.example/v1"

[[providers.models]]
id = "image-01"
supports_image_generation = true

[capabilities]
{caps}
"#
        ))
        .expect("registry toml should parse")
    }

    #[test]
    fn bound_strategy_maps_empty_to_none_without_guessing() {
        // 未绑定 → None（不遍历、不猜、不回落主模型）
        assert_eq!(bound_strategy(""), GenerationStrategy::None);
        // 有绑定 → 原样带上绑定的模型 id
        assert_eq!(
            bound_strategy("image-01"),
            GenerationStrategy::Capability("image-01".to_string())
        );
    }

    #[test]
    fn generation_binding_prefers_explicit_provider_binding() {
        let registry = dual_provider_registry();
        let binding =
            resolve_generation_binding(&registry, GenerationKind::Image).expect("binding resolves");
        assert_eq!(binding.model, "image-01");
        // 同 id 跨段时必须取绑定的那段，而不是段序第一段
        assert_eq!(binding.provider.as_deref(), Some("seg-b"));
        assert!(!binding.is_unbound());
    }

    #[test]
    fn generation_binding_unbound_is_none_not_a_guess() {
        let registry = with_capabilities("");
        assert!(registry.capabilities.image_generation.is_empty());
        let binding = resolve_generation_binding(&registry, GenerationKind::Image)
            .expect("unbound is a normal answer, not an error");
        assert!(binding.is_unbound());
        assert_eq!(binding, GenerationBinding::default());
    }

    #[test]
    fn generation_binding_video_unbound_is_independent_of_image() {
        // 绑了图片、没绑视频 → 视频仍为未绑定（两个能力互不隐含）
        let registry = with_capabilities(
            r#"image_generation = "image-01"
image_generation_provider = "seg-a""#,
        );
        assert!(
            !resolve_generation_binding(&registry, GenerationKind::Image)
                .unwrap()
                .is_unbound()
        );
        assert!(resolve_generation_binding(&registry, GenerationKind::Video)
            .unwrap()
            .is_unbound());
    }

    #[test]
    fn generation_binding_half_bound_is_treated_as_unbound() {
        // 旧配置只写了 model id，没有 provider 归属 → 不当成可用绑定
        let registry = with_capabilities(r#"image_generation = "image-01""#);
        assert!(resolve_generation_binding(&registry, GenerationKind::Image)
            .unwrap()
            .is_unbound());
    }

    #[test]
    fn generation_binding_stale_provider_is_an_error() {
        // provider 段被删/改名 → 明确报错，而不是拿悬空 model id 去打请求
        let registry = with_capabilities(
            r#"image_generation = "image-01"
image_generation_provider = "gone-segment""#,
        );
        let err = match resolve_generation_binding(&registry, GenerationKind::Image) {
            Err(e) => e,
            Ok(_) => panic!("stale provider binding must not resolve"),
        };
        assert!(
            err.contains("gone-segment"),
            "error should name the segment: {err}"
        );
        assert!(
            err.contains("重新绑定"),
            "error should point at re-binding: {err}"
        );
    }

    #[test]
    fn generation_kind_capability_keys_match_capabilities_fields() {
        assert_eq!(GenerationKind::Image.capability_key(), "image_generation");
        assert_eq!(GenerationKind::Video.capability_key(), "video_generation");
    }

    /// resolve_*_strategy / resolve_*_provider 读的是不是各自字段：
    /// 图片与视频两个能力必须互不串字段（绑了图片 ≠ 绑了视频）。
    #[test]
    fn generation_kind_fields_read_the_matching_capability_fields() {
        let caps = Capabilities {
            image_generation: "img-model".to_string(),
            image_generation_provider: "img-seg".to_string(),
            video_generation: "vid-model".to_string(),
            video_generation_provider: "vid-seg".to_string(),
            ..Default::default()
        };
        assert_eq!(
            GenerationKind::Image.fields(&caps),
            ("img-model", "img-seg")
        );
        assert_eq!(
            GenerationKind::Video.fields(&caps),
            ("vid-model", "vid-seg")
        );
    }
}
