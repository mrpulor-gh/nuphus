//! LLM Client
//!
//! Generic LLM client adapted to different Providers via Transport.
//! Supports streaming API calls.
//!
//! ## Usage
//!
//! Client no longer provides hardcoded constructors, created uniformly via ClientFactory from ModelRegistry.
//! ```ignore
//! let factory = ClientFactory::new(registry);
//! let client = factory.create_default_client()?;
//! ```

use crate::{
    api::{ApiClient, AssistantEvent, MessageRequest, ProviderKind},
    Result,
};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// LLM Client (generic LLM client, adapted to different Providers via Transport)
#[derive(Clone)]
pub struct LlmClient {
    /// Transport layer - handles HTTP and protocol format
    transport: Arc<dyn crate::transports::Transport>,
    endpoint: String,
    model: String,
    provider_kind: ProviderKind,
    /// providers.toml segment name backing this client (e.g. "deepseek",
    /// "custom"). Empty when unknown — the Transport cannot supply it: its
    /// `provider_name()` reports the provider *type* id, under which every
    /// custom segment collapses to "custom". Only the factory, which resolves
    /// the segment from the registry, knows the real segment name.
    provider_name: String,
}

impl LlmClient {
    /// Construct directly from ChatCompletionsConfig.
    /// Provider identity is resolved from `provider_kind` (set by the
    /// originating Provider's `transport()` method), or falls back to MiniMax
    /// for direct config construction (legacy / tests).
    pub fn from_config(config: crate::transports::ChatCompletionsConfig) -> Result<Self> {
        use crate::api::ProviderKind;
        let provider_kind = config.provider_kind.unwrap_or(ProviderKind::MiniMax);
        let endpoint = config.base_url.clone();
        let model = config.model.clone();
        let transport: Arc<dyn crate::transports::Transport> =
            Arc::new(crate::transports::ChatCompletionsTransport::new(config));
        Ok(Self {
            transport,
            endpoint,
            model,
            provider_kind,
            // ChatCompletionsConfig.name is the provider *type* id, not the
            // providers.toml segment name — leave unknown rather than guessing.
            provider_name: String::new(),
        })
    }

    /// Construct Client with custom Transport (generic version)
    /// Provider detection: prefer transport.provider_kind(), fallback to MiniMax
    pub fn with_transport(transport: impl crate::transports::Transport + 'static) -> Self {
        let transport = Arc::new(transport);
        let model = transport.model().to_string();
        let provider_kind = transport.provider_kind().unwrap_or(ProviderKind::MiniMax);
        Self {
            transport,
            endpoint: String::new(),
            model,
            provider_kind,
            provider_name: String::new(),
        }
    }

    /// Construct Client with boxed Transport (for factory use)
    /// Provider detection: prefer transport.provider_kind(), fallback to MiniMax
    pub fn with_transport_arc(transport: Arc<dyn crate::transports::Transport>) -> Self {
        let model = transport.model().to_string();
        let provider_kind = transport.provider_kind().unwrap_or(ProviderKind::MiniMax);
        Self {
            transport,
            endpoint: String::new(),
            model,
            provider_kind,
            provider_name: String::new(),
        }
    }

    /// Bind the providers.toml segment name (builder form).
    ///
    /// The factory resolves the segment from the registry and calls this; other
    /// constructors leave the name empty (see the field docs).
    pub fn with_provider_name(mut self, provider_name: impl Into<String>) -> Self {
        self.provider_name = provider_name.into();
        self
    }

    /// Use custom endpoint
    pub fn with_endpoint(mut self, endpoint: String) -> Self {
        self.endpoint = endpoint;
        self
    }

    /// Use custom model (keep original value, alias resolution handled by ModelRegistry)
    pub fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }

    /// Streaming call (async) - via Transport layer
    pub async fn stream_async(&self, request: MessageRequest) -> Result<Vec<AssistantEvent>> {
        tracing::info!("=== stream_async (Transport layer) ===");
        tracing::info!("Model: {}", self.model);

        // Call via Transport, get StreamEvent
        let stream_events = self.transport.stream(request).await?;

        // Convert to AssistantEvent (compatible with existing Agent code)
        let events: Vec<AssistantEvent> = stream_events.into_iter().map(|e| e.into()).collect();

        Ok(events)
    }
}

#[async_trait::async_trait]
impl ApiClient for LlmClient {
    async fn stream(&self, request: MessageRequest) -> Result<Vec<AssistantEvent>> {
        self.stream_async(request).await
    }

    async fn stream_with_cancellation(
        &self,
        request: MessageRequest,
        cancel_flag: &AtomicBool,
    ) -> Result<Vec<AssistantEvent>> {
        let stream_events = self
            .transport
            .stream_with_cancellation(request, cancel_flag)
            .await?;
        let events: Vec<AssistantEvent> = stream_events.into_iter().map(|e| e.into()).collect();
        // Check for cancellation signal from transport
        if events
            .iter()
            .any(|e| matches!(e, AssistantEvent::Cancelled))
        {
            return Err(crate::NuphusError::LLM(crate::LLMError::Cancelled));
        }
        Ok(events)
    }

    async fn stream_with_emitter(
        &self,
        request: MessageRequest,
        cancel_flag: &AtomicBool,
        emitter: Box<dyn Fn(AssistantEvent) + Send>,
    ) -> Result<()> {
        self.transport
            .stream_with_emitter(request, cancel_flag, emitter)
            .await
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn provider_kind(&self) -> ProviderKind {
        self.provider_kind
    }

    fn provider_name(&self) -> &str {
        &self.provider_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transports::ChatCompletionsTransport;

    #[test]
    fn test_with_transport() {
        let transport = ChatCompletionsTransport::new(crate::transports::ChatCompletionsConfig {
            name: "test".to_string(),
            api_key: "test-key".to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            model: "deepseek-v4-flash".to_string(),
            timeout_secs: 300,
            auth_header: "authorization".to_string(),
            auth_prefix: "Bearer ".to_string(),
            provider_kind: None,
            quirks: crate::config::provider::ProviderQuirks::default(),
            reasoning_effort: None,
        });
        let client = LlmClient::with_transport(transport);
        assert_eq!(client.model, "deepseek-v4-flash");
        assert_eq!(client.provider_kind, ProviderKind::MiniMax);
        // Transport 只能提供 provider *类型* id → 段名未知即空串，调用方据此
        // 回落到同名候选遍历（绝不把 provider_kind 当段名用）。
        assert_eq!(client.provider_name(), "");
        // 工厂路径显式绑定 providers.toml 段名后不再折叠。
        let bound = client.clone().with_provider_name("seg-b");
        assert_eq!(bound.provider_name(), "seg-b");
        assert_eq!(bound.model_name(), "deepseek-v4-flash");
        assert_eq!(bound.provider_kind(), ProviderKind::MiniMax);
    }
}
