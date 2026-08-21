//! 配置模块 — 模型配置加载与管理

pub mod model;
pub mod preferences;
pub mod provider;
pub mod providers;
pub mod registry;
pub use model::*;
pub use preferences::{BrowserIdentity, UserPreferences};

use std::path::PathBuf;

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

    // 主模型直接支持多模态
    let main_supports_vision = registry
        .find_model(&registry.model)
        .map(|(_, m)| m.supports_vision)
        .unwrap_or(false);
    if main_supports_vision {
        return VisionStrategy::Main;
    }

    VisionStrategy::None
}
