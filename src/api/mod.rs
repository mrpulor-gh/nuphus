//! API module
//!
//! Provides multi-Provider support and unified API abstraction
//!
//! Model configuration (alias resolution, provider metadata) has been moved to config::ModelRegistry.
//! This module only retains the ProviderKind enum and API Client trait definition.

pub mod types;

pub use types::*;

use crate::Result;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

/// Provider type — canonical enum shared across config and API layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenAI,
    MiniMax,
    Kimi,
    DeepSeek,
    OpenRouter,
    Google,
    Qwen,
    Zhipu,
    ByteDance,
    Anthropic,
    Custom,
    Local,
}

impl ProviderKind {
    /// Convert a stable string id to the corresponding enum variant.
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "minimax" => Some(Self::MiniMax),
            "deepseek" => Some(Self::DeepSeek),
            "kimi" => Some(Self::Kimi),
            "openai" => Some(Self::OpenAI),
            "openrouter" => Some(Self::OpenRouter),
            "google" => Some(Self::Google),
            "qwen" => Some(Self::Qwen),
            "zhipu" => Some(Self::Zhipu),
            "bytedance" => Some(Self::ByteDance),
            "anthropic" => Some(Self::Anthropic),
            "custom" => Some(Self::Custom),
            "local" => Some(Self::Local),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAI => "openai",
            Self::MiniMax => "minimax",
            Self::Kimi => "kimi",
            Self::DeepSeek => "deepseek",
            Self::OpenRouter => "openrouter",
            Self::Google => "google",
            Self::Qwen => "qwen",
            Self::Zhipu => "zhipu",
            Self::ByteDance => "bytedance",
            Self::Anthropic => "anthropic",
            Self::Custom => "custom",
            Self::Local => "local",
        }
    }
}

/// API Client trait
///
/// Using async-trait to make it dyn-compatible (supports `Arc<dyn ApiClient>`)
#[async_trait::async_trait]
pub trait ApiClient: Send + Sync {
    /// Streaming call (async)
    async fn stream(&self, request: MessageRequest) -> Result<Vec<AssistantEvent>>;

    /// Streaming call (with cancel flag) — default implementation
    async fn stream_with_cancellation(
        &self,
        request: MessageRequest,
        cancel_flag: &AtomicBool,
    ) -> Result<Vec<AssistantEvent>> {
        let events = self.stream(request).await?;
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(crate::NuphusError::LLM(crate::LLMError::Cancelled));
        }
        Ok(events)
    }

    /// Streaming call + per-event emission (true real-time push)
    /// Defaults to stream_with_cancellation
    async fn stream_with_emitter(
        &self,
        request: MessageRequest,
        cancel_flag: &AtomicBool,
        emitter: Box<dyn Fn(AssistantEvent) + Send>,
    ) -> Result<()> {
        let events = self.stream_with_cancellation(request, cancel_flag).await?;
        for event in events {
            emitter(event);
        }
        Ok(())
    }

    /// Get model name
    fn model_name(&self) -> &str;

    /// Get provider type
    fn provider_kind(&self) -> ProviderKind;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_kind_as_str() {
        assert_eq!(ProviderKind::MiniMax.as_str(), "minimax");
        assert_eq!(ProviderKind::DeepSeek.as_str(), "deepseek");
    }
}
