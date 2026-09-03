//! Anthropic Messages API Transport
//!
//! Implements the [`Transport`] trait for Anthropic's Messages API
//! (`/v1/messages`). All message/tool format conversion from the
//! Chat-Completions-style [`MessageRequest`] to Anthropic's schema happens
//! in this module.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::api::types::{MessageRequest, ToolDefinition};
use crate::transports::{StreamEvent, Transport};
use crate::Result;

use super::config::AnthropicConfig;
use super::parser::parse_sse;

// ── AnthropicTransport ──────────────────────────────────────────────────────

/// Transport that communicates with Anthropic's Messages API.
///
/// # Protocol differences from Chat Completions
///
/// | Aspect | Chat Completions | Anthropic Messages API |
/// |--------|-----------------|----------------------|
/// | Auth header | `authorization: Bearer <key>` | `x-api-key: <key>` |
/// | Version header | (none) | `anthropic-version: 2023-06-01` |
/// | System prompt | `messages[0].role: "system"` | top-level `system` field |
/// | Tool format | `parameters` (JSON Schema) | `input_schema` (JSON Schema) |
/// | Tool call | `tool_calls` in message | `tool_use` content block |
/// | Tool result | `role: "tool"` message | `tool_result` content block |
/// | SSE events | only `data:` lines | `event:` + `data:` lines |
/// | Thinking | (varies by provider) | native `thinking` content block |
pub(crate) struct AnthropicTransport {
    config: AnthropicConfig,
}

impl AnthropicTransport {
    pub fn new(config: AnthropicConfig) -> Self {
        Self { config }
    }

    /// Build the Anthropic Messages API request body from a Nuphus
    /// `MessageRequest` (which uses Chat-Completions-style message roles).
    fn build_request_body(&self, request: &MessageRequest) -> serde_json::Value {
        // Anthropic Messages API 强制要求 max_tokens（缺失即 HTTP 400）。
        // 显式值优先；None 走配置解析链（providers.toml ModelEntry.max_tokens → builtin → 8192 兜底），
        // 避免长 thinking 流被静默截断（thinking 占满输出预算 → text/tool_call 缺失 → 空交付）。
        let max_tokens = request.max_tokens.unwrap_or_else(|| {
            let model = if request.model.is_empty() {
                &self.config.model
            } else {
                &request.model
            };
            crate::config::resolve_max_output_tokens(model).unwrap_or(8192)
        });
        let mut body = serde_json::json!({
            "model": if request.model.is_empty() {
                &self.config.model
            } else {
                &request.model
            },
            "max_tokens": max_tokens,
            "stream": request.stream,
        });

        // ── system prompt ────────────────────────────────────────────────
        if let Some(ref merged) = request.merged_system {
            body["system"] = serde_json::json!(merged);
        } else {
            let mut system_parts: Vec<String> = Vec::new();
            if let Some(sys) = &request.system {
                system_parts.push(sys.clone());
            }
            for sys_msg in &request.system_messages {
                system_parts.push(sys_msg.clone());
            }
            if !system_parts.is_empty() {
                body["system"] = serde_json::json!(system_parts.join("\n"));
            }
        }

        // ── temperature ──────────────────────────────────────────────────
        // Anthropic optionally supports temperature; forwarded only when explicitly set.
        if let Some(temperature) = request.temperature {
            body["temperature"] = serde_json::json!(temperature);
        }

        // ── messages ─────────────────────────────────────────────────────
        body["messages"] = serde_json::Value::Array(self.convert_messages(&request.messages));

        // ── tools ────────────────────────────────────────────────────────
        if let Some(tools) = &request.tools {
            body["tools"] =
                serde_json::json!(tools.iter().map(convert_tool_def).collect::<Vec<_>>());
        }

        // ── reasoning effort (extended thinking depth) ────────────────────
        // Anthropic Messages API: `{"reasoning": {"effort": "none|low|high|max"}}`.
        // Only emitted when the user configured a value — otherwise the request
        // stays byte-identical to current behavior.
        if let Some(effort) = &self.config.reasoning_effort {
            body["reasoning"] = serde_json::json!({ "effort": effort });
        }

        body
    }

    /// Convert Chat-Completions-style messages to Anthropic format.
    fn convert_messages(&self, messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
        let mut out = Vec::with_capacity(messages.len());

        for msg in messages {
            let role = msg["role"].as_str().unwrap_or("user");
            match role {
                "system" => {
                    // System messages are handled via the top-level `system`
                    // field. We emit a minimal placeholder so the conversation
                    // isn't broken — the actual system content is above.
                    // If the only message is system, it's fully in `system`.
                    out.push(serde_json::json!({
                        "role": "user",
                        "content": "[system prompt — see top-level system field]",
                    }));
                }

                "tool" => {
                    // Chat Completions tool result → Anthropic tool_result block.
                    let tool_call_id = msg["tool_call_id"].as_str().unwrap_or("call_unknown");
                    let content = msg["content"].as_str().unwrap_or("");

                    out.push(serde_json::json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": tool_call_id,
                            "content": content,
                        }],
                    }));
                }

                "assistant" => {
                    out.push(self.convert_assistant_message(msg));
                }

                _ => {
                    // user or any other role — keep as-is when text-only;
                    // if content is already an array (tool_result from
                    // previous round-trip), preserve it.
                    out.push(serde_json::json!({
                        "role": "user",
                        "content": msg["content"],
                    }));
                }
            }
        }

        out
    }

    /// Convert a Chat-Completions assistant message to Anthropic format.
    ///
    /// Handles both plain-text and tool-call messages.
    fn convert_assistant_message(&self, msg: &serde_json::Value) -> serde_json::Value {
        let text = msg["content"].as_str().unwrap_or("");
        let has_text = !text.is_empty();
        let has_tool_calls = msg
            .get("tool_calls")
            .and_then(|t| t.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);

        if !has_tool_calls {
            // Simple text-only assistant message.
            return serde_json::json!({
                "role": "assistant",
                "content": text,
            });
        }

        // Build content array: text block + tool_use blocks.
        let mut content: Vec<serde_json::Value> = Vec::new();

        if has_text {
            content.push(serde_json::json!({
                "type": "text",
                "text": text,
            }));
        }

        if let Some(tool_calls) = msg["tool_calls"].as_array() {
            for tc in tool_calls {
                let tc_id = tc["id"].as_str().unwrap_or("call_unknown");
                let name = tc["function"]["name"].as_str().unwrap_or("unknown");
                let args: serde_json::Value = tc["function"]["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(serde_json::json!({}));

                content.push(serde_json::json!({
                    "type": "tool_use",
                    "id": tc_id,
                    "name": name,
                    "input": args,
                }));
            }
        }

        serde_json::json!({
            "role": "assistant",
            "content": content,
        })
    }
}

// ── Tool definition conversion ──────────────────────────────────────────────

/// Convert a Chat-Completions `ToolDefinition` to Anthropic's tool format
/// (`input_schema` instead of `parameters`).
fn convert_tool_def(tool: &ToolDefinition) -> serde_json::Value {
    serde_json::json!({
        "name": tool.function.name,
        "description": tool.function.description,
        "input_schema": tool.function.parameters,
    })
}

// ── Transport trait implementation ──────────────────────────────────────────

#[async_trait::async_trait]
impl Transport for AnthropicTransport {
    /// Required: non-cancellable streaming call.
    async fn stream(&self, request: MessageRequest) -> Result<Vec<StreamEvent>> {
        self.do_request(&request).await
    }

    /// Streaming call with cancellation support.
    async fn stream_with_cancellation(
        &self,
        request: MessageRequest,
        cancel_flag: &AtomicBool,
    ) -> Result<Vec<StreamEvent>> {
        let body = self.build_request_body(&request);
        let endpoint = self.config.endpoint();

        tracing::debug!(
            "Anthropic request: model={}, stream={}, messages={}",
            body["model"],
            body["stream"],
            body["messages"].as_array().map(|a| a.len()).unwrap_or(0),
        );

        // Build HTTP client
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.config.timeout_secs))
            .build()
            .map_err(|e| {
                crate::NuphusError::LLM(crate::LLMError::HttpBuildFailed {
                    error: format!("{e}"),
                })
            })?;

        // Build and send request
        let http_req = client
            .post(&endpoint)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body);

        // Check cancellation before sending
        if cancel_flag.load(Ordering::SeqCst) {
            return Ok(vec![StreamEvent::Cancelled]);
        }

        let response = http_req.send().await.map_err(|e| {
            crate::NuphusError::LLM(crate::LLMError::RequestFailed {
                error: format!("{e}"),
            })
        })?;

        // Check cancellation after response
        if cancel_flag.load(Ordering::SeqCst) {
            return Ok(vec![StreamEvent::Cancelled]);
        }

        let status = response.status();
        let body_bytes = response.bytes().await.map_err(|e| {
            crate::NuphusError::LLM(crate::LLMError::ReadResponseFailed {
                error: format!("{e}"),
            })
        })?;

        if !status.is_success() {
            let error_text = String::from_utf8_lossy(&body_bytes);
            let detail =
                if let Ok(err_json) = serde_json::from_str::<serde_json::Value>(&error_text) {
                    err_json["error"]["message"]
                        .as_str()
                        .unwrap_or(&error_text)
                        .to_string()
                } else {
                    error_text.to_string()
                };
            return Err(crate::NuphusError::LLM(crate::LLMError::ApiError {
                status: status.as_u16(),
                body: detail,
            }));
        }

        let body_str = String::from_utf8_lossy(&body_bytes);
        let events = parse_sse(&body_str)?;

        // Surface error events
        for event in &events {
            if let StreamEvent::Error(msg) = event {
                return Err(crate::NuphusError::LLM(crate::LLMError::StreamError {
                    error: msg.clone(),
                }));
            }
        }

        Ok(events)
    }

    fn provider_name(&self) -> &'static str {
        "anthropic"
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn provider_kind(&self) -> Option<crate::api::ProviderKind> {
        self.config.provider_kind
    }
}

// ── private helpers ─────────────────────────────────────────────────────────

impl AnthropicTransport {
    /// Non-cancellable request — used by the required `stream()` method.
    async fn do_request(&self, request: &MessageRequest) -> Result<Vec<StreamEvent>> {
        let bogus = AtomicBool::new(false);
        self.stream_with_cancellation(request.clone(), &bogus).await
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(effort: Option<&str>) -> AnthropicConfig {
        AnthropicConfig {
            api_key: "sk-ant-test".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            model: "claude-sonnet-5".to_string(),
            timeout_secs: 30,
            provider_kind: Some(crate::api::ProviderKind::Anthropic),
            reasoning_effort: effort.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_build_request_body_reasoning_effort_sent() {
        for effort in ["none", "low", "high", "max"] {
            let transport = AnthropicTransport::new(config_with(Some(effort)));
            let request = MessageRequest::new("claude-sonnet-5", vec![]);
            let body = transport.build_request_body(&request);
            assert_eq!(
                body["reasoning"],
                serde_json::json!({ "effort": effort }),
                "configured effort {} should be sent as reasoning.effort",
                effort
            );
        }
    }

    #[test]
    fn test_build_request_body_reasoning_effort_not_configured() {
        let transport = AnthropicTransport::new(config_with(None));
        let request = MessageRequest::new("claude-sonnet-5", vec![]);
        let body = transport.build_request_body(&request);
        assert!(
            body.get("reasoning").is_none(),
            "no reasoning block when effort is not configured (current behavior preserved)"
        );
    }
}
