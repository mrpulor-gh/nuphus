//! Responses SSE → Nuphus StreamEvent 状态机（纯解析，不依赖网络）。
//!
//! 事件名与载荷以官方 OpenAPI spec 生成的 SDK 源码为准（见 mod.rs 锚点）：
//! - `response.output_text.delta`            → `StreamEvent::TextDelta(delta)`
//! - `response.reasoning_summary_text.delta` / `response.reasoning_text.delta`
//!   → `StreamEvent::Reasoning(delta)`
//! - `response.function_call_arguments.delta` → 内部累积（无中间事件）
//! - `response.output_item.done` (function_call) → `StreamEvent::ToolUse{id,name,arguments}`
//! - `response.completed`                     → `StreamEvent::Done`
//! - `response.failed` / `error`              → 结构化错误（Err，文本含 code/message）
//! - 其余生命周期/忽略事件（created / in_progress / content_part.* / 其它
//!   output_item.done 类型 / output_text.done 等）→ 不进事件流
//!
//! 失败文本刻意拼接 `code` 与 `message` 原文，使上游 `is_retryable_llm_error`
//! 的字符串分类可直接命中（400/401/429/5xx 关键字判定，见 §7 失败层契约）。

use std::collections::HashMap;

use serde_json::Value;

use crate::transports::StreamEvent;
use crate::{LLMError, NuphusError, Result};

/// Read cache-hit tokens from Responses usage, honoring provider-specific quirks.
/// An empty quirk uses the Responses/OpenAI standard
/// `usage.input_tokens_details.cached_tokens` path.
fn read_cache_hit(usage: &Value, field: &str) -> u32 {
    let value = if field.is_empty() {
        usage
            .get("input_tokens_details")
            .and_then(|details| details.get("cached_tokens"))
    } else {
        usage.get(field)
    };
    value.and_then(Value::as_u64).unwrap_or(0) as u32
}

/// 一个 function_call 输出项的暂存态（从 added/delta 累积到 done）。
#[derive(Debug, Default)]
struct PendingFunctionCall {
    name: String,
    /// 累积的 arguments JSON 字符串（response.function_call_arguments.delta）。
    arguments: String,
}

/// Responses SSE 流解析状态机。
///
/// 用法：
/// ```ignore
/// let mut p = ResponsesStreamParser::default();
/// let events = p.push_sse_text(&raw_sse)?;   // 可分多次喂入（多 chunk）
/// ```
/// 单次 `push_sse_text` 会先把文本按 SSE 块拆成 (event, data) 再逐条解析，
/// 输出该批文本产出的事件。跨 chunk 的 function_call arguments 累积在状态机内。
#[derive(Debug, Default)]
pub struct ResponsesStreamParser {
    /// item_id → 待完成 function_call（arguments delta 累积地）。
    pending: HashMap<String, PendingFunctionCall>,
    /// Provider-specific usage field; empty means the Responses standard path.
    cache_hit_field: String,
}

impl ResponsesStreamParser {
    pub fn with_cache_hit_field(field: &str) -> Self {
        Self {
            cache_hit_field: field.to_string(),
            ..Self::default()
        }
    }

    /// 解析一段原始 SSE 文本（0..n 条事件），返回归一化 Nuphus 事件。
    ///
    /// SSE 线格式：`event: <type>` / `data: <json>`，事件间空行分隔；`data:`
    /// 可多行（按 `\n` 拼接）。兼容 OpenAI Responses 实际输出与 mock 单测。
    pub fn push_sse_text(&mut self, sse: &str) -> Result<Vec<StreamEvent>> {
        let mut out = Vec::new();
        let mut current_type: Option<String> = None;
        let mut current_data: Vec<&str> = Vec::new();

        for line in sse.lines() {
            let trimmed = line.trim_end_matches('\r');
            if trimmed.is_empty() {
                // 空行 = 事件结束
                self.flush_event(current_type.take(), &mut current_data, &mut out)?;
                continue;
            }
            if let Some(v) = trimmed.strip_prefix("event:") {
                current_type = Some(v.trim().to_string());
            } else if let Some(v) = trimmed.strip_prefix("data:") {
                current_data.push(v.trim());
            }
            // 其它行（如 `:` 注释、`id:`）忽略。
        }
        // 文件末尾无空行时 flush 最后一条事件。
        self.flush_event(current_type.take(), &mut current_data, &mut out)?;
        Ok(out)
    }

    /// flush 一条完整 SSE 事件（有 event: 类型且有 data 才解析）。
    fn flush_event(
        &mut self,
        event_type: Option<String>,
        data: &mut Vec<&str>,
        out: &mut Vec<StreamEvent>,
    ) -> Result<()> {
        let Some(ty) = event_type else {
            data.clear();
            return Ok(());
        };
        if data.is_empty() {
            return Ok(());
        }
        let data_text = data.join("\n");
        data.clear();
        let value: Value = match serde_json::from_str(&data_text) {
            Ok(v) => v,
            Err(e) => {
                return Err(NuphusError::LLM(LLMError::StreamError {
                    error: format!("responses SSE data 非法 JSON (event={}): {}", ty, e),
                }));
            }
        };
        out.extend(self.push_event(&ty, &value)?);
        Ok(())
    }

    /// 解析单条已拆分的 SSE 事件（event_type + data JSON）。
    pub fn push_event(&mut self, event_type: &str, data: &Value) -> Result<Vec<StreamEvent>> {
        match event_type {
            "response.output_text.delta" => {
                let delta = data
                    .get("delta")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if delta.is_empty() {
                    return Ok(Vec::new());
                }
                Ok(vec![StreamEvent::TextDelta(delta.to_string())])
            }

            // 官方 reasoning 文本两个通道：summary（推理摘要）与 text（完整推理链）。
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let delta = data
                    .get("delta")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if delta.is_empty() {
                    return Ok(Vec::new());
                }
                Ok(vec![StreamEvent::Reasoning(delta.to_string())])
            }

            "response.function_call_arguments.delta" => {
                let item_id = data
                    .get("item_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if item_id.is_empty() {
                    // 无 item_id 的事件无法关联暂存态——记录但不阻塞流。
                    return Ok(Vec::new());
                }
                let delta = data
                    .get("delta")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let entry = self.pending.entry(item_id.to_string()).or_default();
                entry.arguments.push_str(delta);
                Ok(Vec::new())
            }

            "response.output_item.added" => {
                let ty = data.pointer("/item/type").and_then(|v| v.as_str());
                if ty == Some("function_call") {
                    let item_id = data
                        .pointer("/item/id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let name = data
                        .pointer("/item/name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    if !item_id.is_empty() {
                        let entry = self.pending.entry(item_id.to_string()).or_default();
                        if entry.name.is_empty() {
                            entry.name = name.to_string();
                        }
                    }
                }
                Ok(Vec::new())
            }

            "response.output_item.done" => {
                let item_ty = data.pointer("/item/type").and_then(|v| v.as_str());
                if item_ty == Some("function_call") {
                    self.emit_tool_use(data)
                } else {
                    // message / reasoning / web_search_call 等其它输出项：文本已通过
                    // output_text.delta 送达，此处不重复发事件。
                    Ok(Vec::new())
                }
            }

            "response.completed" => {
                let mut events = Vec::with_capacity(2);
                if let Some(usage) = data.pointer("/response/usage") {
                    events.push(StreamEvent::Usage {
                        input_tokens: usage
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32,
                        output_tokens: usage
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32,
                        cache_hit_tokens: read_cache_hit(usage, &self.cache_hit_field),
                    });
                }
                events.push(StreamEvent::Done);
                Ok(events)
            }

            "response.failed" => Err(Self::failed_error(
                "response.failed",
                data.pointer("/response/error").unwrap_or(data),
            )),

            "error" => Err(Self::failed_error("error", data)),

            // created / in_progress / queued / content_part.added|done /
            // output_text.done / reasoning_*.done / function_call_arguments.done
            // / file_search_call.* / web_search_call.* / code_interpreter_call.*
            // —— 状态推进/忽略事件，不进事件流。
            _ => Ok(Vec::new()),
        }
    }

    /// output_item.done (type=function_call) → 完整 ToolUse。
    ///
    /// id 取官方 `call_id`（稳定工具调用 ID，后续 tool 结果回填用它配对）；
    /// 缺失时退回 `id` / SSE item_id。
    fn emit_tool_use(&mut self, data: &Value) -> Result<Vec<StreamEvent>> {
        let item = data.get("item").cloned().unwrap_or_default();
        let call_id = item
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let item_id = item
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let fallback_id = data
            .get("item_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let id = if !call_id.is_empty() {
            call_id.to_string()
        } else if !item_id.is_empty() {
            item_id.clone()
        } else {
            fallback_id.clone()
        };
        if id.is_empty() {
            // 无任何 id 的 function_call 无法参与工具回填——忽略并记录。
            tracing::warn!("[responses] output_item.done function_call 缺 id，忽略");
            return Ok(Vec::new());
        }

        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| self.pending.get(&item_id).map(|p| p.name.clone()))
            .or_else(|| self.pending.get(&fallback_id).map(|p| p.name.clone()))
            .unwrap_or_default();

        // done 的 item.arguments 为完整态；缺省时退回 delta 累积 buffer。
        let arguments = item
            .get("arguments")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| self.pending.get(&item_id).map(|p| p.arguments.clone()))
            .or_else(|| self.pending.get(&fallback_id).map(|p| p.arguments.clone()))
            .unwrap_or_default();

        // 清理暂存态（保留 arguments 供返回前引用）。
        self.pending.remove(&item_id);
        self.pending.remove(&fallback_id);

        Ok(vec![StreamEvent::ToolUse {
            id,
            name,
            arguments,
        }])
    }

    /// 把 failed / error 事件转成结构化 Nuphus 错误。
    ///
    /// 文本保留官方 code 与 message 原文，让 is_retryable_llm_error 的分类
    /// （400/401/403/404/422/429/5xx/rate limit/model 关键字）可直接命中。
    fn failed_error(event: &str, err_obj: &Value) -> NuphusError {
        let code = err_obj
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let message = err_obj
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("(no message)");
        NuphusError::LLM(LLMError::StreamError {
            error: format!("responses event '{event}' failed: code={code}, message={message}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(sse: &str) -> Result<Vec<StreamEvent>> {
        ResponsesStreamParser::default().push_sse_text(sse)
    }

    /// 把 (event type, data JSON) 序列化为标准 SSE 文本块（程序化生成，
    /// 避免手写 JSON 转义错误）。
    fn sse(events: &[(&str, serde_json::Value)]) -> String {
        let mut out = String::new();
        for (ty, data) in events {
            out.push_str("event: ");
            out.push_str(ty);
            out.push_str("\ndata: ");
            out.push_str(&data.to_string());
            out.push_str("\n\n");
        }
        out
    }

    fn created() -> serde_json::Value {
        serde_json::json!({"type": "response.created", "response": {"id": "resp_1"}})
    }

    fn completed() -> serde_json::Value {
        serde_json::json!({"type": "response.completed", "response": {"id": "resp_1"}})
    }

    #[test]
    fn parses_completed_usage_before_done() {
        let sse = sse(&[(
            "response.completed",
            serde_json::json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "usage": {"input_tokens": 123, "output_tokens": 17, "input_tokens_details": {"cached_tokens": 41}}
                }
            }),
        )]);
        let events = parse(&sse).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            StreamEvent::Usage {
                input_tokens: 123,
                output_tokens: 17,
                cache_hit_tokens: 41
            }
        ));
        assert!(matches!(&events[1], StreamEvent::Done));
    }

    #[test]
    fn completed_without_usage_still_emits_done() {
        let events = parse(&sse(&[("response.completed", completed())])).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], StreamEvent::Done));
    }

    /// 典型 Responses 流：reasoning delta → text delta → function_call
    /// arguments delta → output_item.done(function_call) → completed。
    #[test]
    fn parses_typical_stream() {
        let sse = sse(&[
            ("response.created", created()),
            (
                "response.reasoning_summary_text.delta",
                serde_json::json!({
                    "type": "response.reasoning_summary_text.delta",
                    "item_id": "rs_1", "summary_index": 0, "delta": "Let me think"
                }),
            ),
            (
                "response.reasoning_summary_text.delta",
                serde_json::json!({
                    "type": "response.reasoning_summary_text.delta",
                    "item_id": "rs_1", "summary_index": 0, "delta": " step by step."
                }),
            ),
            (
                "response.output_text.delta",
                serde_json::json!({
                    "type": "response.output_text.delta",
                    "item_id": "msg_1", "content_index": 0, "delta": "Hello"
                }),
            ),
            (
                "response.output_text.delta",
                serde_json::json!({
                    "type": "response.output_text.delta",
                    "item_id": "msg_1", "content_index": 0, "delta": " world"
                }),
            ),
            (
                "response.output_item.added",
                serde_json::json!({
                    "type": "response.output_item.added",
                    "item": {
                        "id": "fc_1", "type": "function_call", "call_id": "call_abc",
                        "name": "web_search", "arguments": "", "status": "in_progress"
                    }
                }),
            ),
            (
                "response.function_call_arguments.delta",
                serde_json::json!({
                    "type": "response.function_call_arguments.delta",
                    "item_id": "fc_1", "delta": r#"{"query":"rust asy"#
                }),
            ),
            (
                "response.function_call_arguments.delta",
                serde_json::json!({
                    "type": "response.function_call_arguments.delta",
                    "item_id": "fc_1", "delta": r#"nc"}"#
                }),
            ),
            (
                "response.output_item.done",
                serde_json::json!({
                    "type": "response.output_item.done",
                    "item": {
                        "id": "fc_1", "type": "function_call", "call_id": "call_abc",
                        "name": "web_search", "arguments": r#"{"query":"rust async"}"#,
                        "status": "completed"
                    }
                }),
            ),
            ("response.completed", completed()),
        ]);

        let events = parse(&sse).unwrap();

        // reasoning → Reasoning 通道
        assert!(matches!(
            &events[0],
            StreamEvent::Reasoning(t) if t == "Let me think"
        ));
        assert!(matches!(
            &events[1],
            StreamEvent::Reasoning(t) if t == " step by step."
        ));
        // text → TextDelta
        assert!(matches!(&events[2], StreamEvent::TextDelta(t) if t == "Hello"));
        assert!(matches!(&events[3], StreamEvent::TextDelta(t) if t == " world"));
        // function_call：delta 累积后由 output_item.done 产出完整 ToolUse
        match &events[4] {
            StreamEvent::ToolUse {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "call_abc");
                assert_eq!(name, "web_search");
                let parsed: serde_json::Value =
                    serde_json::from_str(arguments).expect("arguments 应为 JSON 字符串");
                assert_eq!(parsed["query"], "rust async");
            }
            other => panic!("期望 ToolUse，实际 {:?}", other),
        }
        // completed → Done
        assert!(matches!(&events[5], StreamEvent::Done));
        assert_eq!(events.len(), 6);
    }

    /// response.failed → 结构化错误（含 code + message 原文）。
    #[test]
    fn failed_event_returns_structured_error() {
        let sse = sse(&[
            (
                "response.failed",
                serde_json::json!({
                    "type": "response.failed",
                    "response": {
                        "id": "resp_1", "status": "failed",
                        "error": {
                            "code": "rate_limit_exceeded",
                            "message": "Too many requests. Try again later."
                        }
                    }
                }),
            ),
            ("response.completed", completed()),
        ]);
        let err = parse(&sse).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("response.failed"),
            "错误应指明失败事件: {text}"
        );
        assert!(
            text.contains("rate_limit_exceeded"),
            "错误应含官方 code: {text}"
        );
        assert!(
            text.contains("Too many requests"),
            "错误应含 message: {text}"
        );

        // is_retryable 分类可直接命中（429/rate limit 可重试）。
        assert!(
            crate::agent::common::is_retryable_llm_error(&text),
            "rate_limit_exceeded 应可重试: {text}"
        );
    }

    /// 顶层 `error` SSE 事件（ResponseErrorEvent）→ 结构化错误。
    #[test]
    fn error_event_returns_structured_error() {
        let sse = sse(&[(
            "error",
            serde_json::json!({
                "type": "error", "code": "invalid_api_key",
                "message": "Incorrect API key provided.", "param": null
            }),
        )]);
        let err = parse(&sse).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("invalid_api_key"), "{text}");
        assert!(text.contains("Incorrect API key"), "{text}");
        // invalid_api_key → 401 auth 语义 → 不可重试。
        assert!(
            !crate::agent::common::is_retryable_llm_error(&text),
            "invalid_api_key 不应重试: {text}"
        );
    }

    /// 非 function_call 的 output_item.done / 状态事件不产生事件流。
    #[test]
    fn ignores_non_relevant_events() {
        let sse = sse(&[
            (
                "response.in_progress",
                serde_json::json!({"type": "response.in_progress", "response": {"id": "resp_1"}}),
            ),
            (
                "response.output_item.added",
                serde_json::json!({
                    "type": "response.output_item.added",
                    "item": {"id": "msg_1", "type": "message", "role": "assistant", "content": []}
                }),
            ),
            (
                "response.output_text.done",
                serde_json::json!({
                    "type": "response.output_text.done",
                    "item_id": "msg_1", "output_index": 0, "content_index": 0, "text": "full text"
                }),
            ),
            (
                "response.output_item.done",
                serde_json::json!({
                    "type": "response.output_item.done",
                    "item": {
                        "id": "msg_1", "type": "message", "role": "assistant",
                        "content": [{"type": "output_text", "text": "full text"}]
                    }
                }),
            ),
            ("response.completed", completed()),
        ]);
        let events = parse(&sse).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], StreamEvent::Done));
    }

    /// 跨 chunk 喂入：arguments delta 被拆成两段 SSE 文本仍能正确累积。
    #[test]
    fn accumulates_arguments_across_chunks() {
        let chunk1 = sse(&[
            (
                "response.output_item.added",
                serde_json::json!({
                    "type": "response.output_item.added",
                    "item": {
                        "id": "fc_1", "type": "function_call", "call_id": "call_x",
                        "name": "read_file", "arguments": "", "status": "in_progress"
                    }
                }),
            ),
            (
                "response.function_call_arguments.delta",
                serde_json::json!({
                    "type": "response.function_call_arguments.delta",
                    "item_id": "fc_1", "delta": r#"{"path":"/tmp/a""#
                }),
            ),
        ]);
        let chunk2 = sse(&[
            (
                "response.function_call_arguments.delta",
                serde_json::json!({
                    "type": "response.function_call_arguments.delta",
                    "item_id": "fc_1", "delta": "}"
                }),
            ),
            (
                "response.output_item.done",
                serde_json::json!({
                    "type": "response.output_item.done",
                    "item": {
                        "id": "fc_1", "type": "function_call", "call_id": "call_x",
                        "name": "read_file", "arguments": r#"{"path":"/tmp/a"}"#, "status": "completed"
                    }
                }),
            ),
            ("response.completed", completed()),
        ]);

        let mut p = ResponsesStreamParser::default();
        let first = p.push_sse_text(&chunk1).unwrap();
        assert!(first.is_empty(), "delta 阶段不应产出事件");

        let second = p.push_sse_text(&chunk2).unwrap();
        match &second[0] {
            StreamEvent::ToolUse {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "call_x");
                assert_eq!(name, "read_file");
                assert_eq!(arguments, r#"{"path":"/tmp/a"}"#);
            }
            other => panic!("期望 ToolUse，实际 {:?}", other),
        }
        assert!(matches!(&second[1], StreamEvent::Done));
    }

    /// 锁定非法 data JSON 的报错路径（如非 200 响应体被误喂 parser 时）。
    #[test]
    fn invalid_sse_data_reports_error() {
        let sse = "event: response.completed\ndata: {not-json}\n";
        let err = parse(sse).unwrap_err();
        assert!(err.to_string().contains("非法 JSON"));
    }
}

// Cache usage coverage: input_tokens_details.cached_tokens is mapped by read_cache_hit.
#[cfg(test)]
mod cache_usage_regression_tests {
    use super::*;
    #[test]
    fn missing_cache_field_is_zero() {
        let usage = serde_json::json!({"input_tokens": 1, "output_tokens": 2});
        assert_eq!(read_cache_hit(&usage, ""), 0);
    }
    #[test]
    fn cache_field_is_read() {
        let usage = serde_json::json!({"input_tokens_details": {"cached_tokens": 41}});
        assert_eq!(read_cache_hit(&usage, ""), 41);
    }
}
