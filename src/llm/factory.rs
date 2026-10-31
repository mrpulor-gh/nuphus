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
    /// 主模型绑定唯一权威：`ModelRegistry::resolve_main_binding`（成对表解析，
    /// 多候选无绑定 = 报错不猜）。历史上此处曾用 `[last_model]` hint 反查 +
    /// 本地多候选分支，随影子轨道废止一并收编。
    pub fn create_main_client(&self) -> Result<Arc<dyn ApiClient>> {
        let registry = self.registry()?;
        let (provider, model) = registry.resolve_main_binding()?;
        self.create_client_for(&provider, &model)
    }

    /** 模型协议覆盖优先于服务商默认路由，认证来源不参与协议选择。 */
    fn build_transport(
        &self,
        registry: &ModelRegistry,
        provider: &ProviderConfig,
        model_id: &str,
    ) -> Result<Arc<dyn Transport>> {
        let pmeta = ProviderRegistry::builtin()
            .get(provider.provider_type.as_str())
            .ok_or_else(|| crate::NuphusError::Config("unknown provider type".into()))?;
        let model = provider.models.iter().find(|m| m.id == model_id);
        let kind = model
            .and_then(|m| m.transport)
            .unwrap_or_else(|| pmeta.transport_for(model_id));
        let credentials =
            provider
                .oauth
                .as_ref()
                .map(|oauth| crate::config::credentials::TokenSource {
                    config_path: registry.source_path.clone(),
                    provider: provider.name.clone(),
                    oauth: oauth.clone(),
                });
        let mut cfg = provider.clone();
        if kind != TransportKind::Responses {
            if let Some(source) = credentials.clone() {
                cfg.api_key = std::thread::Builder::new()
                    .name("oauth-token-refresh".into())
                    .spawn(move || source.resolve())
                    .map_err(|_| crate::NuphusError::llm("Could not start OAuth resolver"))?
                    .join()
                    .map_err(|_| crate::NuphusError::llm("OAuth resolver failed"))?
                    .map_err(crate::NuphusError::llm)?;
            }
        }
        let base_url = if cfg.base_url.is_empty() {
            pmeta.default_base_url().to_string()
        } else {
            cfg.base_url.clone()
        };
        let auth_header = if cfg.auth_header.is_empty() {
            pmeta.auth_header().to_string()
        } else {
            cfg.auth_header.clone()
        };
        let auth_prefix = if cfg.auth_prefix.is_empty() {
            pmeta.auth_prefix().to_string()
        } else {
            cfg.auth_prefix.clone()
        };
        let mut quirks = pmeta.quirks();
        quirks.extra_headers.extend(
            cfg.extra_headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        match kind {
            TransportKind::Responses => {
                if provider.provider_type == crate::api::ProviderKind::OpenCodeGo {
                    quirks.extra_headers.push((
                        "x-opencode-session".into(),
                        crate::transports::opencode_session::opencode_session_id(),
                    ));
                }
                Ok(Arc::new(ResponsesTransport::new(ResponsesConfig {
                    base_url,
                    api_key: if credentials.is_some() {
                        String::new()
                    } else {
                        cfg.api_key.clone()
                    },
                    model: model_id.into(),
                    timeout_secs: cfg.timeout_secs,
                    auth_header,
                    auth_prefix,
                    provider_kind: Some(cfg.provider_type),
                    quirks,
                    reasoning_effort: cfg.reasoning_effort.clone(),
                    credentials,
                    options: model.and_then(|m| m.responses.clone()).unwrap_or_default(),
                })))
            }
            other if other == pmeta.transport_for(model_id) => Ok(pmeta.transport(&cfg, model_id)),
            TransportKind::ChatCompletions => Ok(Arc::new(
                crate::transports::chat_completions::ChatCompletionsTransport::new(
                    crate::transports::chat_completions::ChatCompletionsConfig {
                        name: cfg.provider_type.as_str().into(),
                        provider_segment: cfg.name.clone(),
                        base_url,
                        api_key: cfg.api_key.clone(),
                        model: model_id.into(),
                        timeout_secs: cfg.timeout_secs,
                        auth_header,
                        auth_prefix,
                        provider_kind: Some(cfg.provider_type),
                        quirks,
                        reasoning_effort: cfg.reasoning_effort.clone(),
                    },
                ),
            )),
            TransportKind::Anthropic => Ok(Arc::new(
                crate::transports::anthropic::AnthropicTransport::new(
                    crate::transports::anthropic::AnthropicConfig {
                        base_url,
                        api_key: cfg.api_key.clone(),
                        model: model_id.into(),
                        timeout_secs: cfg.timeout_secs,
                        provider_kind: Some(cfg.provider_type),
                        reasoning_effort: cfg.reasoning_effort.clone(),
                        extra_headers: quirks.extra_headers,
                        provider_segment: cfg.name.clone(),
                    },
                ),
            )),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ProviderKind;

    #[test]
    fn model_override_roundtrips_without_changing_sibling_or_default_route() {
        let (dir, path, _) = dual_segment_registry();
        let factory = ClientFactory::live_at(&path);
        crate::config::model::set_model_transport(
            &path,
            "seg-b",
            "probe-model",
            Some(TransportKind::Responses),
        )
        .unwrap();
        let mut registry = factory.registry().unwrap();
        assert_eq!(registry.providers[0].models[0].transport, None);
        assert_eq!(
            registry.providers[1].models[0].transport,
            Some(TransportKind::Responses)
        );
        assert_eq!(
            factory
                .build_transport(&registry, &registry.providers[1], "probe-model")
                .unwrap()
                .provider_name(),
            "responses"
        );
        registry.source_path = None;
        registry.providers[1].oauth = Some(crate::config::ProviderOAuth {
            access_token: "memory-token".into(),
            expires_at: Some(i64::MAX),
            ..Default::default()
        });
        assert_eq!(
            factory
                .build_transport(&registry, &registry.providers[1], "probe-model")
                .unwrap()
                .provider_name(),
            "responses"
        );
        crate::config::model::set_model_transport(&path, "seg-b", "probe-model", None).unwrap();
        let registry = factory.registry().unwrap();
        assert_ne!(
            factory
                .build_transport(&registry, &registry.providers[1], "probe-model")
                .unwrap()
                .provider_name(),
            "responses"
        );
        assert!(crate::config::model::set_model_transport(
            &path,
            "absent",
            "probe-model",
            Some(TransportKind::Responses)
        )
        .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 内置 minimax / qwen 默认路由 = Anthropic Messages（官方兼容端点，
    /// x-api-key + anthropic-version）。模型级覆盖可切回 Chat Completions，
    /// 置空还原为默认 Anthropic。
    #[test]
    fn builtin_minimax_qwen_default_route_is_anthropic() {
        let dir = std::env::temp_dir().join(format!(
            "nuphus-factory-builtin-route-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("providers.toml");
        let cfg = "model = \"MiniMax-M3\"\n\n\
             [[providers]]\n\
             name = \"minimax\"\n\
             provider_type = \"minimax\"\n\
             api_key = \"sk-mm\"\n\
             base_url = \"https://api.minimaxi.com/v1\"\n\n\
             [[providers.models]]\n\
             id = \"MiniMax-M3\"\n\
             context_window = 250000\n\n\
             [[providers]]\n\
             name = \"qwen\"\n\
             provider_type = \"qwen\"\n\
             api_key = \"sk-q\"\n\n\
             [[providers.models]]\n\
             id = \"qwen-plus\"\n\
             context_window = 131072\n";
        std::fs::write(&path, cfg).unwrap();
        let factory = ClientFactory::live_at(&path);

        let reg = factory.registry().unwrap();
        for (seg, model) in [("minimax", "MiniMax-M3"), ("qwen", "qwen-plus")] {
            let provider = &reg.providers.iter().find(|p| p.name == seg).unwrap();
            let transport = factory.build_transport(&reg, provider, model).unwrap();
            assert_eq!(transport.provider_name(), "anthropic", "seg {seg}");
        }

        // 模型级覆盖切回 Chat Completions（factory 通用分支；Chat 名取
        // provider_type 字符串， 非 Anthropic 标签即可分辨路由族）
        crate::config::model::set_model_transport(
            &path,
            "minimax",
            "MiniMax-M3",
            Some(TransportKind::ChatCompletions),
        )
        .unwrap();
        let reg = factory.registry().unwrap();
        let provider = &reg.providers.iter().find(|p| p.name == "minimax").unwrap();
        let transport = factory
            .build_transport(&reg, provider, "MiniMax-M3")
            .unwrap();
        assert_eq!(transport.provider_name(), "minimax");
        assert_ne!(transport.provider_name(), "anthropic");

        // 置空还原 → 默认 Anthropic
        crate::config::model::set_model_transport(&path, "minimax", "MiniMax-M3", None).unwrap();
        let reg = factory.registry().unwrap();
        let provider = &reg.providers.iter().find(|p| p.name == "minimax").unwrap();
        let transport = factory
            .build_transport(&reg, provider, "MiniMax-M3")
            .unwrap();
        assert_eq!(transport.provider_name(), "anthropic");

        std::fs::remove_dir_all(dir).unwrap();
    }

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
        // provider 未知（None / ""）→ None，不扫 sibling（二元组化 P1）：
        // probe-model 在两段都有，但缺绑定时任何选择都是猜——显式 unknown。
        assert_eq!(registry.resolve_context_window(None, "probe-model"), None);
        assert_eq!(
            registry.resolve_context_window(Some(""), "probe-model"),
            None
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
    #[tokio::test]
    async fn oauth_responses_never_falls_back_to_an_api_key() {
        let (dir, path, _) = dual_segment_registry();
        let mut doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        doc["providers"].as_array_mut().unwrap()[1]
            .as_table_mut()
            .unwrap()
            .insert(
                "oauth".into(),
                toml::Value::try_from(crate::config::ProviderOAuth {
                    account_ref: Some("oaiapp_missing".into()),
                    credential_file: "test-accounts.json".into(),
                    ..Default::default()
                })
                .unwrap(),
            );
        doc["providers"][1]["models"][0]
            .as_table_mut()
            .unwrap()
            .insert("transport".into(), toml::Value::String("responses".into()));
        std::fs::write(&path, toml::to_string(&doc).unwrap()).unwrap();
        let factory = ClientFactory::live_at(&path);
        let registry = factory.registry().unwrap();
        let provider = &registry.providers[1];
        // The fixture still has sk-b and a custom endpoint. Neither may be
        // used as a fallback when its OAuth account has no credentials.
        let transport = factory
            .build_transport(&registry, provider, "probe-model")
            .unwrap();
        assert_eq!(transport.provider_name(), "responses");
        let error = transport
            .stream(crate::api::MessageRequest::new("probe-model", vec![]))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("OAuth account not found"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
