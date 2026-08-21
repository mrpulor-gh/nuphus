//! Transports module — pluggable HTTP transport layer
//!
//! - Abstract HTTP transport and protocol format conversion
//! - Each Transport implements a specific Provider's API call
//! - Agent/LLM layer only depends on Transport trait, doesn't care about HTTP details
//!
//! ## Architecture
//!
//! ReactAgent depends only on the `Transport` trait (`stream()`, `provider_name()`, `model()`).
//! Concrete transports (ChatCompletions, Bedrock) implement the trait and delegate
//! HTTP calls to `reqwest`. Adding a provider requires only a new struct implementing `Transport`
//! plus a constructor in `TransportFactory` — no changes needed in the Agent layer.
//!
//! ## Adding a new Provider
//! 1. Add a new Transport struct in this file
//! 2. Implement `Transport` trait
//! 3. Add constructor method in `TransportFactory`
//! 4. No changes needed in Agent layer

pub mod anthropic;
pub mod chat_completions;
pub mod mock_transport;
pub mod transport_base;

pub use chat_completions::{ChatCompletionsConfig, ChatCompletionsTransport};
pub use mock_transport::MockTransport;
pub use transport_base::{StreamEvent, Transport};

use crate::api;

/// Convert StreamEvent to AssistantEvent (compatible with existing Agent code)
impl From<StreamEvent> for api::AssistantEvent {
    fn from(event: StreamEvent) -> Self {
        match event {
            StreamEvent::TextDelta(text) => api::AssistantEvent::TextDelta(text),
            StreamEvent::Reasoning(text) => api::AssistantEvent::Reasoning(text),
            StreamEvent::ToolUse {
                id,
                name,
                arguments,
            } => api::AssistantEvent::ToolUse {
                id,
                name,
                input: arguments,
            },
            StreamEvent::ImageUrl(url) => api::AssistantEvent::ImageAttachment { url },
            StreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_hit_tokens,
            } => api::AssistantEvent::Usage {
                input_tokens,
                output_tokens,
                cache_hit_tokens,
            },
            StreamEvent::Done => api::AssistantEvent::MessageStop,
            StreamEvent::Cancelled => api::AssistantEvent::Cancelled,
            StreamEvent::Error(msg) => {
                tracing::error!("Transport error: {}", msg);
                api::AssistantEvent::TextDelta(format!("[Error] {}", msg))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_event_to_assistant_event() {
        let text_event = StreamEvent::TextDelta("hello".to_string());
        let assistant: api::AssistantEvent = text_event.into();
        assert!(matches!(assistant, api::AssistantEvent::TextDelta(_)));

        let tool_event = StreamEvent::ToolUse {
            id: "1".to_string(),
            name: "test".to_string(),
            arguments: "{}".to_string(),
        };
        let assistant: api::AssistantEvent = tool_event.into();
        assert!(matches!(assistant, api::AssistantEvent::ToolUse { .. }));
    }
}
