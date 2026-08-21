//! Transport trait definitions
//!
//! Abstract HTTP transport and protocol format conversion

use crate::Result;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};

/// Raw HTTP response data
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// HTTP streaming event (transport-layer emitted, format-agnostic)
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Text delta
    TextDelta(String),
    /// Reasoning/thinking content (needs to be sent back in DeepSeek thinking mode)
    Reasoning(String),
    /// Tool call
    ToolUse {
        id: String,
        name: String,
        arguments: String,
    },
    /// Image URL (model-native image generation output)
    ImageUrl(String),
    /// Usage
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        cache_hit_tokens: u32,
    },
    /// Stream end
    Done,
    /// Error
    Error(String),
    /// Task has been cancelled
    Cancelled,
}

/// Transport trait - pluggable HTTP + format conversion layer
///
/// Each Transport is responsible for:
/// 1. Constructing the HTTP request for a specific Provider
/// 2. Sending the request (sync/streaming)
/// 3. Converting the raw response into unified StreamEvent
#[async_trait]
pub trait Transport: Send + Sync {
    /// Streaming call - async generation of StreamEvent
    async fn stream(&self, request: crate::api::MessageRequest) -> Result<Vec<StreamEvent>>;

    /// Streaming call (with cancellation flag) - async generation of StreamEvent
    /// If cancel_flag is set, stop parsing and return Cancelled
    async fn stream_with_cancellation(
        &self,
        request: crate::api::MessageRequest,
        cancel_flag: &AtomicBool,
    ) -> Result<Vec<StreamEvent>> {
        let events = self.stream(request).await?;
        // Check cancel flag
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(crate::NuphusError::LLM(crate::LLMError::Cancelled));
        }
        Ok(events)
    }

    /// Streaming request + emit per event (true real-time push)
    /// Default fallback: collect then emit (for non-streaming transports)
    async fn stream_with_emitter(
        &self,
        request: crate::api::MessageRequest,
        cancel_flag: &AtomicBool,
        emitter: Box<dyn Fn(crate::api::AssistantEvent) + Send>,
    ) -> Result<()> {
        let events = self.stream_with_cancellation(request, cancel_flag).await?;
        for event in events {
            emitter(event.into());
        }
        Ok(())
    }

    /// Get Provider name
    fn provider_name(&self) -> &str;

    /// Get model name
    fn model(&self) -> &str;

    /// Get Provider type (default returns None)
    /// Transports that support explicit Provider field detection should override this
    fn provider_kind(&self) -> Option<crate::api::ProviderKind> {
        None
    }
}
