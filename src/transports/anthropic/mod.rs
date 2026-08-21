//! Anthropic Messages API Transport
//!
//! provider-driven transport for Anthropic's Messages API (`/v1/messages`).
//! Handles the different auth scheme (x-api-key), SSE event types,
//! and message format (system as top-level, tool_use/tool_result blocks).

mod config;
mod parser;
mod transport;

pub(crate) use config::AnthropicConfig;
pub(crate) use transport::AnthropicTransport;
