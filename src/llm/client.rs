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
        }
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
    }
}
