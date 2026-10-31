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

/// 窗口余量 clamp 的预留量（tokens）：Anthropic 校验 `input + max_tokens ≤
/// 上下文窗口`，留 1K 给系统提示词增量 / 工具调用等提示词外占用。
const WINDOW_HEADROOM_TOKENS: usize = 1024;

/// 窗口余量 clamp 的下限：再小的输出预算会让长 thinking / 工具流直接空交付，
/// 宁可以 400 / stop_reason 的形式把超窗暴露出来，也不发一个必然被截断的预算。
const MIN_OUTPUT_TOKENS_FLOOR: usize = 256;

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
        // max_tokens 解析链（A2，8192 兜底已删）：
        //   request.max_tokens（调用方显式意图）→ 配置解析（providers.toml
        //   用户手写 max_tokens > 采集 max_output_tokens）→ None。
        // 仍 None → **不写字段**：Anthropic 对缺失 max_tokens 报 400
        // 「max_tokens: Field required」，错误路径会打可定位 warn（见
        // `stream_with_cancellation`），采集由 desktop 侧上限探测兜底。
        // 绝不回落静态猜值：8192 曾把 reasoning 模型的长 thinking 流截断
        // （thinking 占满输出预算 → text/tool_call 缺失 → 空交付）。
        let model = if request.model.is_empty() {
            &self.config.model
        } else {
            &request.model
        };
        // 段名绑定（`provider_segment`）：同名模型在多个段可配不同上限，
        // 必须按 providers.toml 段名解析，否则 resolve_capability 直接返回
        // None（空段名 → 不猜兄弟段）。
        let segment = (!self.config.provider_segment.is_empty())
            .then_some(self.config.provider_segment.as_str());
        let cap = request
            .max_tokens
            .or_else(|| crate::config::resolve_max_output_tokens(model, segment));
        // 窗口余量 clamp：长会话里 input 已占满窗口时，`input + max_tokens`
        // 会超出上下文窗口。hint / window 任一缺失 → 不 clamp（原样发送）。
        let cap = cap.map(|cap| {
            let window = crate::agent::goal_types::try_get_context_window_for(model, segment);
            clamp_output(cap, request.input_tokens_hint, window, model)
        });
        let mut body = serde_json::json!({
            "model": model,
            "stream": request.stream,
        });
        // 必填字段的「不写」是有意为之：见上方解析链注释。条件插入保证
        // 未解析到上限时报文里根本没有这个键（而不是 null / 猜值）。
        if let Some(max_tokens) = cap {
            body["max_tokens"] = serde_json::json!(max_tokens);
        }

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
            // A2 诊断：未解析到输出上限时 Anthropic 对缺失 max_tokens 报 400
            // 「max_tokens: Field required」。只 warn、不改错误返回语义——让
            // 「该段没采到上限」在日志里可定位（采集由 desktop 侧上限探测兜底）。
            // 段名一并打出：多段同名模型时按段补配置 / 补探测。
            if status.as_u16() == 400 && error_text.contains("max_tokens") {
                tracing::warn!(
                    "[anthropic] 400 mentioning max_tokens — model={}, segment={}, detail={}",
                    body["model"],
                    self.config.provider_segment,
                    detail,
                );
            }
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

    /// providers.toml 段名（非协议 id）：段级配置解析的唯一绑定。
    fn provider_segment(&self) -> &str {
        &self.config.provider_segment
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn provider_kind(&self) -> Option<crate::api::ProviderKind> {
        self.config.provider_kind
    }
}

// ── private helpers ─────────────────────────────────────────────────────────

/// 窗口余量 clamp（A2）：`min(cap, window - hint - [`WINDOW_HEADROOM_TOKENS`]`，
/// 下限 [`MIN_OUTPUT_TOKENS_FLOOR`]。
///
/// `hint` / `window` 任一缺失 → 不 clamp：没有依据就不改用户/采集得到的值。
/// 收紧时打 warn（model / cap / effective）——clamp 改的是线上报文，必须可观测，
/// 禁止静默改用户显式值。
fn clamp_output(cap: u32, hint: Option<u32>, window: Option<usize>, model: &str) -> u32 {
    let (Some(hint), Some(window)) = (hint, window) else {
        return cap;
    };
    let allowance = window
        .saturating_sub(hint as usize)
        .saturating_sub(WINDOW_HEADROOM_TOKENS)
        .max(MIN_OUTPUT_TOKENS_FLOOR);
    let effective = u32::try_from(allowance).map_or(cap, |a| cap.min(a));
    if effective < cap {
        tracing::warn!(
            "[anthropic] max_tokens clamped to fit the context window: \
             model={}, cap={}, effective={} (window={}, input_tokens_hint={})",
            model,
            cap,
            effective,
            window,
            hint,
        );
    }
    effective
}

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
            provider_segment: String::new(), // 测试/直构：无段绑定
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

    /// 本轮接线事实：Anthropic 通道原先是「按 model id、无段绑定」解析配置
    /// （`resolve_max_output_tokens(model, None)` 恒 None），采集到的
    /// `max_output_tokens` 永远解析不到。现在段名随 transport 携带。
    ///
    /// A2 契约：8192 兜底已删——该段在本机配置里不存在 → 解析不到上限 →
    /// **不写 `max_tokens` 字段**（静态猜值会把长 thinking 流截断；缺失字段
    /// 由服务端 400 + desktop 侧上限探测补采，见 `stream_with_cancellation`
    /// 的 400 诊断 warn）。
    ///
    /// 为什么不做端到端断言：`resolve_max_output_tokens` 读的是全局
    /// `config_search_paths()`（用户真实 providers.toml），测试进程无法注入
    /// 一份隔离配置而不污染用户配置、也不该靠环境变量造假绿。解析链本身由
    /// `config::model::tests::test_get_max_output_tokens_user_then_reported`
    /// 覆盖；此处只钉「段名被传入」与「未解析到 → 省略字段」两个事实。
    #[test]
    fn test_provider_segment_wired_and_unresolved_omits_max_tokens() {
        let mut cfg = config_with(None);
        cfg.provider_segment = "nuphus-test-nonexistent-segment".to_string();
        let transport = AnthropicTransport::new(cfg);
        assert_eq!(
            transport.provider_segment(),
            "nuphus-test-nonexistent-segment",
            "段名必须原样暴露给配置解析（不是协议 id \"anthropic\"）"
        );
        let request = MessageRequest::new("claude-sonnet-5", vec![]);
        let body = transport.build_request_body(&request);
        assert!(
            body.get("max_tokens").is_none(),
            "未解析到上限时不写 max_tokens 字段——禁止回落任何静态猜值"
        );
    }

    /// 空段名（内存 / 直构 transport，无段绑定）同样解析不到上限 → 不写字段。
    /// 与段名绑定用例并列：有没有段绑定都不该产出静态猜值。
    #[test]
    fn test_build_request_body_unbound_segment_omits_max_tokens() {
        let transport = AnthropicTransport::new(config_with(None));
        let request = MessageRequest::new("claude-sonnet-5", vec![]);
        let body = transport.build_request_body(&request);
        assert!(
            body.get("max_tokens").is_none(),
            "无段绑定时解析链同样走到 None——不写字段，不回落静态猜值"
        );
    }

    /// clamp 纯函数契约（A2）：有 hint + window 时取
    /// `min(cap, window - hint - 1024)`，下限 256；hint / window 缺失 → 不 clamp。
    #[test]
    fn test_clamp_output_window_remainder() {
        // 窗口余量充足 → 不动（cap 优先）
        assert_eq!(
            clamp_output(64_000, Some(1_000), Some(200_000), "m"),
            64_000
        );
        // 余量收紧：min(64000, 200000-190000-1024=8976) → 8976 < cap
        assert_eq!(
            clamp_output(64_000, Some(190_000), Some(200_000), "m"),
            8_976
        );
        // 下限 256：余量算成负数也发 256，不让输出预算归零
        assert_eq!(clamp_output(64_000, Some(199_999), Some(200_000), "m"), 256);
        // cap 小于窗口余量 → cap 兜底（不因窗口大就超模型自然上限）
        assert_eq!(clamp_output(4_096, Some(1_000), Some(200_000), "m"), 4_096);
        // hint / window 缺失 → 没有依据就不改值
        assert_eq!(clamp_output(64_000, None, Some(200_000), "m"), 64_000);
        assert_eq!(clamp_output(64_000, Some(1_000), None, "m"), 64_000);
    }

    /// 显式 `request.max_tokens` 仍然最高优先，不受段名/配置解析影响；
    /// 且无 hint 时 clamp 不介入（窗口余量无从算起）。
    #[test]
    fn test_build_request_body_explicit_max_tokens_wins_over_config() {
        let mut cfg = config_with(None);
        cfg.provider_segment = "nuphus-test-nonexistent-segment".to_string();
        let transport = AnthropicTransport::new(cfg);
        let mut request = MessageRequest::new("claude-sonnet-5", vec![]);
        request.max_tokens = Some(1234);
        let body = transport.build_request_body(&request);
        assert_eq!(body["max_tokens"], 1234);
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
