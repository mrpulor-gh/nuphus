//! LLM Client Factory
//!
//! Dynamically creates Client for the corresponding Provider based on ModelRegistry.
//! All text tasks use the main model; capability-specific models (vision/stt/tts)
//! are resolved directly by each consumer from `ModelRegistry` fields.

use crate::{
    api::ApiClient,
    config::provider::TransportKind,
    config::registry::ProviderRegistry,
    config::{ModelRegistry, ProviderConfig},
    transports::responses::{ResponsesConfig, ResponsesTransport},
    transports::Transport,
    Result,
};
use std::sync::Arc;

/// 注册表来源。
///
/// `providers.toml` 是模型配置的**唯一权威源**，且它在进程运行期间会被改写
/// （新建自定义模型 / 改密钥 / 加模型）。任何「读一次就冻结」的副本都会让之后的
/// 写入被遮蔽——历史缺陷：新建 provider 段后立刻切模型报
/// `model 'x' not found for provider 'y'`，因为长生命周期 Runtime 里持的是旧快照，
/// 必须重启或新开一轮才恢复。
///
/// 所以长生命周期持有者一律用 [`RegistrySource::Live`]：每次构建客户端时按当前
/// 配置解析。**消除副本**，而不是给副本加失效通知——后者要求穷举所有写盘点，
/// 漏一处就再次断链。
#[derive(Clone)]
enum RegistrySource {
    /// 调用方给定的快照，不随配置文件变化（测试 / CLI 单次运行 / 显式注入）
    Static(Box<ModelRegistry>),
    /// 每次使用都按当前配置解析（`path = None` → 进程规范配置发现）
    Live { path: Option<std::path::PathBuf> },
}

/// LLM Client Factory
#[derive(Clone)]
pub struct ClientFactory {
    source: RegistrySource,
}

impl ClientFactory {
    /// 快照源：注册表内容固定不再变化（测试 / CLI / 显式注入场景）。
    pub fn new(registry: ModelRegistry) -> Self {
        Self {
            source: RegistrySource::Static(Box::new(registry)),
        }
    }

    /// **实时源**：每次构建客户端时按当前配置解析。
    ///
    /// 宿主（桌面 / 插件 / 工作流）一律用本构造器——它是「配置写盘 → 立即可用」
    /// 闭环的实现点：配置改完即生效，无需重启、无需新开一轮。
    pub fn live() -> Self {
        Self {
            source: RegistrySource::Live { path: None },
        }
    }

    /// 实时源（显式配置文件路径）：测试与非规范配置宿主使用。
    pub fn live_at(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            source: RegistrySource::Live {
                path: Some(path.into()),
            },
        }
    }

    /// 当前生效的注册表（**当时**的权威状态，不做缓存）。
    pub fn registry(&self) -> Result<ModelRegistry> {
        match &self.source {
            RegistrySource::Static(r) => Ok((**r).clone()),
            RegistrySource::Live { path: Some(p) } => {
                ModelRegistry::from_toml(&p.to_string_lossy())
            }
            RegistrySource::Live { path: None } => crate::config::load_registry(),
        }
    }

    /// Create Client for the specified model ID
    pub fn create_client(&self, model_id: &str) -> Result<Arc<dyn ApiClient>> {
        let registry = self.registry()?;
        self.build_client_for(&registry, model_id)
    }

    /// 在给定注册表上按 model id 建客户端（`create_client*` 的唯一公共实现）。
    ///
    /// ⚠️ Legacy first-segment-order 语义：同名模型跨 provider 时取段序首段。
    /// 仅 `create_client`（测试用）保留此兼容语义；生产路径一律走
    /// `create_client_for`（精确 provider+model）或 `create_main_client`
    /// （主模型按 `[last_model]` 绑定消歧）。
    fn build_client_for(
        &self,
        registry: &ModelRegistry,
        model_id: &str,
    ) -> Result<Arc<dyn ApiClient>> {
        let (provider, model) = registry
            .find_model(model_id)
            .ok_or_else(|| crate::NuphusError::llm(format!("model '{}' not found", model_id)))?;
        self.build_client(registry, provider, &model.id)
    }

    /// 建客户端 + 标注段名（同 id 跨段时区分路由的唯一标识）。
    fn build_client(
        &self,
        registry: &ModelRegistry,
        provider: &ProviderConfig,
        model_id: &str,
    ) -> Result<Arc<dyn ApiClient>> {
        let transport = self.build_transport(registry, provider, model_id)?;
        let client = super::client::LlmClient::with_transport_arc(transport)
            .with_provider_name(provider.name.clone());
        Ok(Arc::new(client))
    }

    /// Create a client using an exact provider + model binding.
    pub fn create_client_for(
        &self,
        provider_name: &str,
        model_id: &str,
    ) -> Result<Arc<dyn ApiClient>> {
        let registry = self.registry()?;
        let (provider, model) = registry
            .find_model_for_provider(provider_name, model_id)
            .ok_or_else(|| {
                crate::NuphusError::llm(format!(
                    "model '{}' not found for provider '{}'",
                    model_id, provider_name
                ))
            })?;
        self.build_client(&registry, provider, &model.id)
    }

    /// Create Client for the main model (all text tasks)
    ///
    /// 主模型绑定消歧：`registry.model` 只是模型 id，同名模型跨 provider 时
    /// 无消歧解析会取段序首段（跨段误路由）。这里按绑定优先级解析——
    /// 1. `[last_model]` 磁盘记录（`switch_model` 写；见
    ///    `ModelRegistry::last_model_provider_hint`）→ 精确段；
    /// 2. 无记录且候选唯一 → 该唯一段（安全推断）；
    /// 3. 无记录且多候选 → 报错（对齐 `effective_model_binding` 的
    ///    `"model has multiple providers; provider binding is required"` 契约），
    ///    不静默取首段。
    pub fn create_main_client(&self) -> Result<Arc<dyn ApiClient>> {
        let registry = self.registry()?;
        if registry.model.is_empty() {
            return Err(crate::NuphusError::Config(
                "no model configured".to_string(),
            ));
        }
        if let Some(provider) = registry.last_model_provider_hint() {
            return self.create_client_for(&provider, &registry.model);
        }
        match registry.find_model_candidates(&registry.model).len() {
            1 => {
                let provider = registry.find_model_candidates(&registry.model)[0]
                    .0
                    .name
                    .clone();
                self.create_client_for(&provider, &registry.model)
            }
            0 => Err(crate::NuphusError::llm(format!(
                "model '{}' not found",
                registry.model
            ))),
            n => Err(crate::NuphusError::llm(format!(
                "model '{}' has multiple providers ({} candidates); provider binding is required",
                registry.model, n
            ))),
        }
    }

    /// Build a Transport for the given Provider + model.
    ///
    /// provider-driven: delegates to `Provider::transport()` which owns its metadata,
    /// quirks, and transport selection. The factory only handles registry
    /// lookup — transport construction is the Provider's responsibility.
    ///
    /// `registry` 仅用于 OAuth 段的配置文件来源（令牌自动刷新需要回写路径）。
    fn build_transport(
        &self,
        registry: &ModelRegistry,
        provider: &ProviderConfig,
        model_id: &str,
    ) -> Result<Arc<dyn Transport>> {
        // OAuth 段透明化：配置了 oauth 的段以「新鲜 access token」充当 api_key
        // 传入 transport（transport 本身不感知 oauth，继续走既有 Bearer 链）。
        // 未配 oauth / 无文件来源（CLI 内存构造）→ 原样透传，零行为变化。
        let provider = self.with_fresh_oauth_token(provider, registry.source_path.as_deref());
        let pmeta = ProviderRegistry::builtin()
            .get(provider.provider_type.as_str())
            .ok_or_else(|| {
                crate::NuphusError::Config(format!(
                    "unknown provider type: {}",
                    provider.provider_type.as_str()
                ))
            })?;

        // P4 分派（refactor 设计 §4.3 / §7 P4）：按 pmeta.transport_for(model_id)
        // 解析出的协议族选择构造器。
        // - Responses → 新引擎 ResponsesTransport（P4 接线，仅 opencode-go
        //   responses 模型族会命中；chat/anthropic 路径逐字节不变）。
        // - ChatCompletions / Anthropic → 继续委托 Provider::transport()
        //   （现网 13 provider 路径；旧方法退役留后续阶段）。
        let kind = pmeta.transport_for(model_id);
        match kind {
            TransportKind::Responses => {
                tracing::info!(
                    "[factory] build_transport provider={} model={} transport_kind=Responses \
                     (P4 分派；构造 ResponsesTransport)",
                    provider.provider_type.as_str(),
                    model_id
                );
                // ResponsesConfig 组装字段来源（对齐 custom/local 的 cfg 优先、
                // provider 常量兜底模式——与 opencode-go chat 族 transport() 鉴权
                // 语义一致）：
                // - base_url：cfg.base_url 非空用 cfg，否则 pmeta.default_base_url()
                // - api_key / timeout_secs / reasoning_effort：透传 ProviderConfig
                // - auth_header / auth_prefix：cfg 显式配置优先，否则 pmeta 声明常量
                // - provider_kind：ProviderConfig.provider_type（已规范化 ProviderKind）
                // - quirks：pmeta.quirks() 的副本；仅 opencode-go 追加稳定会话 ID
                //   （x-opencode-session，网关 400 MissingSessionID 根因）。守卫限定：
                //   即便将来有非 opencode-go provider 走到 Responses 分支也不误加头。
                let mut quirks = pmeta.quirks();
                if provider.provider_type == crate::api::ProviderKind::OpenCodeGo {
                    quirks.extra_headers.push((
                        "x-opencode-session".to_string(),
                        crate::transports::opencode_session::opencode_session_id(),
                    ));
                }
                Ok(Arc::new(ResponsesTransport::new(ResponsesConfig {
                    base_url: if provider.base_url.is_empty() {
                        pmeta.default_base_url().to_string()
                    } else {
                        provider.base_url.clone()
                    },
                    api_key: provider.api_key.clone(),
                    model: model_id.to_string(),
                    timeout_secs: provider.timeout_secs,
                    auth_header: if provider.auth_header.is_empty() {
                        pmeta.auth_header().to_string()
                    } else {
                        provider.auth_header.clone()
                    },
                    auth_prefix: if provider.auth_prefix.is_empty() {
                        pmeta.auth_prefix().to_string()
                    } else {
                        provider.auth_prefix.clone()
                    },
                    provider_kind: Some(provider.provider_type),
                    quirks,
                    reasoning_effort: provider.reasoning_effort.clone(),
                })))
            }
            // ChatCompletions / Anthropic：现网路径逐字节不变（含 anthropic 官方
            // provider 的 Anthropic transport；不在此新建构造）。
            other => {
                tracing::info!(
                    "[factory] build_transport provider={} model={} transport_kind={:?} \
                     (委托 Provider::transport 既有路径)",
                    provider.provider_type.as_str(),
                    model_id,
                    other
                );
                Ok(pmeta.transport(&provider, model_id))
            }
        }
    }

    /// OAuth 段的令牌注入：返回携带新鲜 access token 的段配置（或原引用内容）。
    ///
    /// 三级路径：
    /// 1. 未配 oauth / 注册表无文件来源（CLI 内存构造）→ 原样返回，零开销。
    /// 2. 内存令牌未过期（`config::oauth::in_memory_fresh_token`）→ 直接注入，
    ///    零磁盘 IO、零网络（transport 构建高频路径的主形态）。
    /// 3. 临期/已过期 → 经独立线程调用 `ensure_fresh_oauth_token`
    ///    （磁盘刷新 + 落盘）。独立线程是为了与调用方运行时解耦：build_transport
    ///    是同步函数，可能运行在 tokio worker 上，阻塞式 HTTP 客户端在
    ///    async 上下文里有运行时嵌套风险，线程隔离一次到位。
    ///    刷新失败（含 401 → 需重新授权）仅记 warn——不携带任何令牌值——
    ///    transport 仍按原 api_key 构建，认证失败由请求链路报给上层。
    ///
    /// `source_path` 由调用方从**当前注册表**带入（实时源下每次都是最新配置的路径）。
    fn with_fresh_oauth_token(
        &self,
        provider: &ProviderConfig,
        source_path: Option<&std::path::Path>,
    ) -> ProviderConfig {
        let Some(oauth) = provider.oauth.as_ref() else {
            return provider.clone();
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if let Some(token) = crate::config::oauth::in_memory_fresh_token(oauth, now) {
            let mut cfg = provider.clone();
            cfg.api_key = token;
            return cfg;
        }
        let Some(path) = source_path.map(std::path::Path::to_path_buf) else {
            tracing::warn!(
                "[factory] provider '{}' 配置了 OAuth 但注册表无配置文件来源，无法自动刷新",
                provider.name
            );
            return provider.clone();
        };
        let name = provider.name.clone();
        // 网络刷新放独立线程：与宿主 async 运行时隔离（见方法注释）
        let refreshed = std::thread::Builder::new()
            .name("oauth-token-refresh".to_string())
            .spawn(move || crate::config::oauth::ensure_fresh_oauth_token(&path, &name))
            .and_then(|h| {
                h.join()
                    .map_err(|_| std::io::Error::other("refresh thread panicked"))
            });
        match refreshed {
            Ok(Ok(token)) => {
                let mut cfg = provider.clone();
                cfg.api_key = token;
                cfg
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    "[factory] provider '{}' OAuth 令牌刷新失败: {e}",
                    provider.name
                );
                provider.clone()
            }
            Err(e) => {
                tracing::warn!(
                    "[factory] provider '{}' OAuth 刷新线程异常: {e}",
                    provider.name
                );
                provider.clone()
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ProviderKind;

    /// 同 id 双段 fixture，且两段都是 `provider_type = "custom"` —— builtin
    /// 枚举把二者一并折叠成 `ProviderKind::Custom`，只有 providers.toml 段名
    /// 能区分；窗口值也不同，便于断言「段名 → provider 精确取值」。
    ///
    /// 返回 (temp dir, 配置文件路径, registry)：调用方负责 `remove_dir_all`。
    fn dual_segment_registry() -> (std::path::PathBuf, std::path::PathBuf, ModelRegistry) {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nuphus-factory-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            seq
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("providers.toml");
        let cfg = "model = \"probe-model\"\n\n\
             [[providers]]\n\
             name = \"seg-a\"\n\
             provider_type = \"custom\"\n\
             api_key = \"sk-a\"\n\
             base_url = \"https://a.example.com/v1\"\n\n\
             [[providers.models]]\n\
             id = \"probe-model\"\n\
             context_window = 200000\n\n\
             [[providers]]\n\
             name = \"seg-b\"\n\
             provider_type = \"custom\"\n\
             api_key = \"sk-b\"\n\
             base_url = \"https://b.example.com/v1\"\n\n\
             [[providers.models]]\n\
             id = \"probe-model\"\n\
             context_window = 64000\n";
        std::fs::write(&cfg_path, cfg).unwrap();
        let registry = ModelRegistry::from_toml(cfg_path.to_str().unwrap()).unwrap();
        (dir, cfg_path, registry)
    }

    /// 工厂创建的 client 必须携带 providers.toml **段名**：这是同 id 跨段
    /// 场景下唯一能区分路由的标识（provider_kind 折叠成 Custom）。
    #[test]
    fn test_create_client_carries_segment_name() {
        let (dir, _cfg_path, snapshot) = dual_segment_registry();
        let factory = ClientFactory::new(snapshot);

        let a = factory.create_client_for("seg-a", "probe-model").unwrap();
        let b = factory.create_client_for("seg-b", "probe-model").unwrap();
        assert_eq!(a.provider_name(), "seg-a");
        assert_eq!(b.provider_name(), "seg-b");

        // provider_kind 折叠：两者不可区分 —— provider_name 存在的理由。
        assert_eq!(a.provider_kind(), ProviderKind::Custom);
        assert_eq!(a.provider_kind(), b.provider_kind());

        // 段名直接驱动 provider 精确窗口解析（同 id 两段取值不同）。
        let registry = factory.registry().unwrap();
        assert_eq!(
            registry.resolve_context_window(Some(a.provider_name()), "probe-model"),
            Some(200_000)
        );
        assert_eq!(
            registry.resolve_context_window(Some(b.provider_name()), "probe-model"),
            Some(64_000)
        );
        // provider 未知（None / ""）→ 回落候选遍历，取首个带值的候选。
        assert_eq!(
            registry.resolve_context_window(None, "probe-model"),
            Some(200_000)
        );

        // create_client（find_model 段序首匹配）→ 段序首段，语义不变。
        let first = factory.create_client("probe-model").unwrap();
        assert_eq!(first.provider_name(), "seg-a");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 闭环回归：**同一** factory 实例必须看到构造之后写入配置文件的段。
    ///
    /// 历史缺陷：新建自定义模型后立刻切换模型报
    /// `model 'x' not found for provider 'y'`——长生命周期 Runtime 持旧快照，
    /// 写入被遮蔽，必须重启或新开一轮才恢复（用户可见症状是「应用后端版本过旧」）。
    #[test]
    fn live_source_sees_segments_written_after_construction() {
        let (dir, cfg_path, snapshot) = dual_segment_registry();
        let factory = ClientFactory::live_at(&cfg_path);

        // 构造时已存在的段可用
        assert!(factory.create_client_for("seg-a", "probe-model").is_ok());

        // 模拟「新建自定义模型」写盘：追加新段
        let appended = "\n[[providers]]\n\
                        name = \"seg-c\"\n\
                        provider_type = \"custom\"\n\
                        api_key = \"sk-c\"\n\
                        base_url = \"https://c.example.com/v1\"\n\n\
                        [[providers.models]]\n\
                        id = \"live-model\"\n";
        let mut content = std::fs::read_to_string(&cfg_path).unwrap();
        content.push_str(appended);
        std::fs::write(&cfg_path, content).unwrap();

        // 实时源：立即可见
        assert!(
            factory.create_client_for("seg-c", "live-model").is_ok(),
            "Live 源必须看到构造之后写入的配置段"
        );
        // 反向对照：快照源看不到 —— 证明本测试不是恒真（旧实现正是在此失败）
        assert!(ClientFactory::new(snapshot)
            .create_client_for("seg-c", "live-model")
            .is_err());

        std::fs::remove_dir_all(&dir).ok();
    }
}
