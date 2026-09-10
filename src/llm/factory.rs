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

/// LLM Client Factory
#[derive(Clone)]
pub struct ClientFactory {
    registry: ModelRegistry,
}

impl ClientFactory {
    pub fn new(registry: ModelRegistry) -> Self {
        Self { registry }
    }

    /// Get underlying registry (read-only)
    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }

    /// Create Client for the specified model ID
    pub fn create_client(&self, model_id: &str) -> Result<Arc<dyn ApiClient>> {
        let (provider, model) = self
            .registry
            .find_model(model_id)
            .ok_or_else(|| crate::NuphusError::llm(format!("model '{}' not found", model_id)))?;

        let transport = self.build_transport(provider, &model.id)?;
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
        let (provider, model) = self
            .registry
            .find_model_for_provider(provider_name, model_id)
            .ok_or_else(|| {
                crate::NuphusError::llm(format!(
                    "model '{}' not found for provider '{}'",
                    model_id, provider_name
                ))
            })?;
        let transport = self.build_transport(provider, &model.id)?;
        Ok(Arc::new(
            super::client::LlmClient::with_transport_arc(transport)
                .with_provider_name(provider.name.clone()),
        ))
    }

    /// Create Client for the main model (all text tasks)
    pub fn create_main_client(&self) -> Result<Arc<dyn ApiClient>> {
        if self.registry.model.is_empty() {
            return Err(crate::NuphusError::Config(
                "no model configured".to_string(),
            ));
        }
        self.create_client(&self.registry.model)
    }

    /// Build a Transport for the given Provider + model.
    ///
    /// provider-driven: delegates to `Provider::transport()` which owns its metadata,
    /// quirks, and transport selection. The factory only handles registry
    /// lookup — transport construction is the Provider's responsibility.
    fn build_transport(
        &self,
        provider: &ProviderConfig,
        model_id: &str,
    ) -> Result<Arc<dyn Transport>> {
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
                Ok(pmeta.transport(provider, model_id))
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
    /// 返回 (temp dir, registry)：调用方负责 `remove_dir_all`。
    fn dual_segment_registry() -> (std::path::PathBuf, ModelRegistry) {
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
        (dir, registry)
    }

    /// 工厂创建的 client 必须携带 providers.toml **段名**：这是同 id 跨段
    /// 场景下唯一能区分路由的标识（provider_kind 折叠成 Custom）。
    #[test]
    fn test_create_client_carries_segment_name() {
        let (dir, registry) = dual_segment_registry();
        let factory = ClientFactory::new(registry);

        let a = factory.create_client_for("seg-a", "probe-model").unwrap();
        let b = factory.create_client_for("seg-b", "probe-model").unwrap();
        assert_eq!(a.provider_name(), "seg-a");
        assert_eq!(b.provider_name(), "seg-b");

        // provider_kind 折叠：两者不可区分 —— provider_name 存在的理由。
        assert_eq!(a.provider_kind(), ProviderKind::Custom);
        assert_eq!(a.provider_kind(), b.provider_kind());

        // 段名直接驱动 provider 精确窗口解析（同 id 两段取值不同）。
        assert_eq!(
            factory
                .registry()
                .resolve_context_window(Some(a.provider_name()), "probe-model"),
            Some(200_000)
        );
        assert_eq!(
            factory
                .registry()
                .resolve_context_window(Some(b.provider_name()), "probe-model"),
            Some(64_000)
        );
        // provider 未知（None / ""）→ 回落候选遍历，取首个带值的候选。
        assert_eq!(
            factory
                .registry()
                .resolve_context_window(None, "probe-model"),
            Some(200_000)
        );

        // create_client（find_model 段序首匹配）→ 段序首段，语义不变。
        let first = factory.create_client("probe-model").unwrap();
        assert_eq!(first.provider_name(), "seg-a");

        std::fs::remove_dir_all(&dir).ok();
    }
}
