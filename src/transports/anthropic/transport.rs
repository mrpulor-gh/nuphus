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
            // AnthropicConfig carries no provider-segment name (its
            // `provider_name()` is the static protocol label "anthropic", not a
            // providers.toml segment), so max_tokens resolves by model id.
            crate::config::resolve_max_output_tokens(model, None).unwrap_or(8192)
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
        // Anthropic Messages API（官方 Effort 页，2026-10 核对）：effort 设在
        // **顶层 `output_config.effort`**，不在 `thinking` 对象内。旧实现发
        // `reasoning: {effort}` 不是官方字段（会被忽略/拒绝，档位实际不生效）。
        // 仅当用户配置了值时才下发，否则请求与现状逐字节一致。
        if let Some(effort) = &self.config.reasoning_effort {
            body["output_config"] = serde_json::json!({ "effort": effort });
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
                    // user or any other role：content 数组里的 image_url 块转
                    // Anthropic 原生 image 源，其余块（text / tool_result 等）原样。
                    // 2026-10-28：此前原样透传，Anthropic 端点收到未知块类型即
                    // HTTP 400（带图消息必失败）；图片序列化必须按 transport 分支。
                    out.push(serde_json::json!({
                        "role": "user",
                        "content": convert_user_content(&msg["content"]),
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
        // 诊断用主机名（解析事实按 host 归属；URL 非法时回落 "unknown"）
        let host = crate::utils::net_diag::host_of(&endpoint);

        tracing::debug!(
            "Anthropic request: model={}, stream={}, messages={}",
            body["model"],
            body["stream"],
            body["messages"].as_array().map(|a| a.len()).unwrap_or(0),
        );

        // Build HTTP client
        // 网络诊断（只观测，不改连接行为）：解析事实挂在 resolver 上，发送失败时取回
        let diag = crate::utils::net_diag::DiagResolver::new_arc();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.config.timeout_secs))
            .dns_resolver(diag.clone())
            .build()
            .map_err(|e| {
                crate::NuphusError::LLM(crate::LLMError::HttpBuildFailed {
                    error: format!("{e}"),
                })
            })?;

        // Build and send request
        let mut http_req = client
            .post(&endpoint)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");
        // 段级自定义请求头（anthropic 兼容中转实例的网关标头等）。x-api-key 被
        // 跳过：鉴权头被覆盖成双值会让网关随机取其一，直接拒配。
        for (k, v) in &self.config.extra_headers {
            if !k.eq_ignore_ascii_case("x-api-key") {
                http_req = http_req.header(k.as_str(), v.as_str());
            }
        }
        let http_req = http_req.json(&body);

        // Check cancellation before sending
        if cancel_flag.load(Ordering::SeqCst) {
            return Ok(vec![StreamEvent::Cancelled]);
        }

        // 本轮耗时取样点：与请求发出同点（该路径无重试，attempt 恒为 1/1）
        let started = std::time::Instant::now();
        let response = http_req.send().await.map_err(|e| {
            let line = crate::utils::net_diag::format_diag(
                &host,
                diag.last().as_ref(),
                crate::utils::net_diag::classify(&e),
                &crate::utils::net_diag::error_chain(&e),
                started.elapsed().as_millis(),
                1,
                1,
            );
            tracing::warn!(target: "nuphus::net", "{}", line);
            crate::NuphusError::LLM(crate::LLMError::RequestFailed {
                error: format!("{e}; {line}"),
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

// ── content 适配（Chat 风格 → Anthropic 原生） ──────────────────────────────

/// Chat 风格 content → Anthropic content。
///
/// - 字符串（纯文本轮次）：原样返回，不强制数组化；
/// - 数组：逐块转换——`image_url` 块转 Anthropic image block（data URL →
///   `source: base64`；http(s) → `source: url`）；无法识别的图片形态跳过并
///   warn（不把非法块透传给端点制造 400）；其余块（text / tool_result /
///   已转换的 image 等）原样保留。
fn convert_user_content(content: &serde_json::Value) -> serde_json::Value {
    let Some(blocks) = content.as_array() else {
        return content.clone();
    };
    let converted: Vec<serde_json::Value> = blocks
        .iter()
        .filter_map(|block| {
            if block.get("type").and_then(|t| t.as_str()) == Some("image_url") {
                let url = block
                    .pointer("/image_url/url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let image = image_block_from_url(url);
                if image.is_none() {
                    tracing::warn!(
                        "[anthropic] skipped an unsupported image block \
                         (only data:*;base64 with png/jpeg/gif/webp, or http(s) URL)"
                    );
                }
                image
            } else {
                Some(block.clone())
            }
        })
        .collect();
    serde_json::Value::Array(converted)
}

/// data URL / http(s) URL → Anthropic image block；不支持的形态返回 None。
fn image_block_from_url(url: &str) -> Option<serde_json::Value> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest.split_once(";base64,")?;
        let media_type = meta.trim();
        if data.is_empty()
            || !matches!(
                media_type,
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            )
        {
            return None;
        }
        Some(serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": media_type,
                "data": data,
            },
        }))
    } else if url.starts_with("http://") || url.starts_with("https://") {
        Some(serde_json::json!({
            "type": "image",
            "source": { "type": "url", "url": url },
        }))
    } else {
        None
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
            extra_headers: Vec::new(),
        }
    }

    #[test]
    fn test_build_request_body_reasoning_effort_sent() {
        // 官方枚举五档（platform.claude.com · Effort）：low/medium/high/xhigh/max。
        for effort in ["low", "medium", "high", "xhigh", "max"] {
            let transport = AnthropicTransport::new(config_with(Some(effort)));
            let request = MessageRequest::new("claude-sonnet-5", vec![]);
            let body = transport.build_request_body(&request);
            assert_eq!(
                body["output_config"],
                serde_json::json!({ "effort": effort }),
                "configured effort {} should be sent as output_config.effort",
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
            body.get("output_config").is_none(),
            "no output_config block when effort is not configured (current behavior preserved)"
        );
    }

    #[test]
    fn test_convert_user_content_maps_image_url_to_native_block() {
        let transport = AnthropicTransport::new(config_with(None));
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "看图" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,aGVsbG8=" } },
                { "type": "image_url", "image_url": { "url": "https://example.com/a.png" } },
            ],
        })];
        let out = transport.convert_messages(&messages);
        let content = out[0]["content"]
            .as_array()
            .expect("content should be array");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "aGVsbG8=");
        assert_eq!(content[2]["type"], "image");
        assert_eq!(content[2]["source"]["type"], "url");
        assert_eq!(content[2]["source"]["url"], "https://example.com/a.png");
    }

    #[test]
    fn test_convert_user_content_keeps_text_and_skips_unsupported_image() {
        let transport = AnthropicTransport::new(config_with(None));
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "纯文本轮次" },
                { "type": "image_url", "image_url": { "url": "ftp://x/a.png" } },
            ],
        })];
        let out = transport.convert_messages(&messages);
        let content = out[0]["content"].as_array().unwrap();
        assert_eq!(
            content.len(),
            1,
            "unsupported image block must be dropped, not forwarded"
        );
        assert_eq!(content[0]["text"], "纯文本轮次");
        // 纯字符串 content 保持原样（不强制数组化）
        let messages = vec![serde_json::json!({ "role": "user", "content": "hello" })];
        let out = transport.convert_messages(&messages);
        assert_eq!(out[0]["content"], "hello");
    }
}
