//! LLM Client Factory
//!
//! Dynamically creates Client for the corresponding Provider based on ModelRegistry.
//! All text tasks use the main model. Capability-specific models (vision/stt/tts)
//! are resolved via `get_capability()`.

use crate::{
    api::ApiClient,
    config::registry::ProviderRegistry,
    config::{ModelRegistry, ProviderConfig},
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
        let client = super::client::LlmClient::with_transport_arc(transport);
        Ok(Arc::new(client))
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

    /// Create Client for a specific capability (vision/stt/tts)
    /// Falls back to main model when the capability is not configured.
    pub fn create_capability_client(&self, capability: &str) -> Result<Arc<dyn ApiClient>> {
        let model_id = self.registry.get_capability(capability);
        self.create_client(&model_id)
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

        // Provider.transport() returns the correct transport (ChatCompletions
        // or Anthropic) with quirks embedded. The ProviderConfig from the
        // registry already has defaults filled from the TOML layer.
        Ok(pmeta.transport(provider, model_id))
    }
}
