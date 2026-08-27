//! API type definitions
//!
//! Unified request/response type definitions

use serde::{Deserialize, Serialize};

/// Risk level for security decisions and operations
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

/// Message request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Sampling temperature (provider-side). None → provider default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    pub messages: Vec<serde_json::Value>,
    /// Fixed system prompt (L0 identity/behavior/safety boundaries), as the first system message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Dynamic system messages (L1 memory/principles + L2 GoalType/meta-cognition/environment),
    /// Inserted right after system, keep prefix fixed to maximize KV cache hits
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub system_messages: Vec<String>,
    /// Merged system prompt: L0+L2+L1 combined into one string.
    /// When set, transport layer sends a single system message instead of system + system_messages.
    /// Built once per session to guarantee KV cache prefix stability.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stream: bool,
}

impl MessageRequest {
    pub fn new(model: impl Into<String>, messages: Vec<serde_json::Value>) -> Self {
        Self {
            model: model.into(),
            max_tokens: None,
            temperature: None,
            messages,
            system: None,
            system_messages: vec![],
            merged_system: None,
            tools: None,
            stream: true,
        }
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    pub fn with_system_messages(mut self, msgs: Vec<String>) -> Self {
        self.system_messages = msgs;
        self
    }

    pub fn with_merged_system(mut self, merged: impl Into<String>) -> Self {
        self.merged_system = Some(merged.into());
        self
    }

    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }
}

/// Model info (for frontend display)
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub alias: Vec<String>,
    pub supports_streaming: bool,
    pub supports_vision: bool,
    pub supports_audio: bool,
    pub supports_image_generation: bool,
    /// Context window (tokens) the model declares. `None` = unknown.
    pub context_window: Option<u64>,
    /// Reasoning-effort levels the model declares (from builtin ModelDef).
    /// Empty = the model exposes no user-configurable effort.
    pub reasoning_efforts: Vec<String>,
    /// Default effort used when the user has not configured one (from builtin
    /// ModelDef). `None` = no declared default (UI falls back to 默认).
    pub default_effort: Option<String>,
    /// Cost in USD per 1M prompt tokens (providers.toml explicit value, or
    /// OpenRouter aggregate pricing ×1_000_000). `None` = unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_per_million_in: Option<f64>,
    /// Cost in USD per 1M completion tokens (same sources as above). `None` = unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_per_million_out: Option<f64>,
}

/// Model switch request
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SwitchModelRequest {
    pub model_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
}

impl ToolDefinition {
    pub fn new(name: impl Into<String>, parameters: serde_json::Value) -> Self {
        Self {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: name.into(),
                description: None,
                parameters,
                permission: None,
            },
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.function.description = Some(description.into());
        self
    }
}

/// Assistant events (streaming)
#[derive(Debug, Clone)]
pub enum AssistantEvent {
    /// Text delta
    TextDelta(String),
    /// Reasoning/thinking content (DeepSeek thinking mode, needs to be sent back to API on next turn)
    Reasoning(String),
    /// Tool use
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
    /// Image URL from model-native image generation
    ImageAttachment { url: String },
    /// Usage (input_tokens, output_tokens)
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        cache_hit_tokens: u32,
    },
    /// Message stop
    MessageStop,
    /// Task cancelled
    Cancelled,
    /// Connection status update (Transport layer retry feedback)
    ConnectionStatus(String),
}

impl AssistantEvent {
    /// Whether it is a text event
    pub fn is_text(&self) -> bool {
        matches!(self, Self::TextDelta(_))
    }

    /// Whether it is a tool call
    pub fn is_tool_use(&self) -> bool {
        matches!(self, Self::ToolUse { .. })
    }

    /// Whether it is a message stop
    pub fn is_message_stop(&self) -> bool {
        matches!(self, Self::MessageStop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_request() {
        let msgs = vec![];
        let req = MessageRequest::new("test-model", msgs);
        assert_eq!(req.model, "test-model");
        assert!(req.stream);
    }
}
