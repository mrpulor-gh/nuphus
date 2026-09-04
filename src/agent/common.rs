//! common — Pure functions shared by ReactAgent and ExecuteAgent
//!
//! Shared logic: process_events, extract_tool_calls, error classification,
//! progress rendering, and tool parameter formatting.
//! Avoids introducing traits or inheritance — pure functions + data.

use crate::agent::events::{EventEmitter, NuphusEvent};
use crate::api::AssistantEvent;
use crate::session::ContentBlock;
use crate::ToolCall;
use std::sync::atomic::AtomicU32;

/// Return result of process_events
pub struct ProcessEventsResult {
    pub blocks: Vec<ContentBlock>,
    /// Optional token usage (ExecuteAgent uses to emit TokenUsage event)
    pub usage: Option<(u32, u32)>,
    /// Cache hit token count (independent field, not bundled into usage tuple)
    pub cache_hit_tokens: u32,
}

/// Convert LLM AssistantEvent stream to unified ContentBlock list.
///
/// `content_tool_tags` — additional XML tag names to parse as tool calls
/// from text content (provider-specific, e.g. `&["function_call"]` for MiniMax).
/// Pass `&[]` for the built-in tag set only.
pub fn process_events(
    events: Vec<AssistantEvent>,
    content_tool_tags: &[&str],
) -> ProcessEventsResult {
    let mut blocks = Vec::new();
    let mut current_text = String::new();
    let mut current_reasoning = String::new();
    let mut current_tool_id = String::new();
    let mut current_tool_name = String::new();
    let mut current_tool_args_raw = String::new();
    let mut in_tool = false;
    let mut usage: Option<(u32, u32)> = None;
    let mut cache_hit_tokens: u32 = 0;

    for event in events {
        match event {
            AssistantEvent::TextDelta(text) => {
                // Accumulate raw text without stripping — think blocks will be
                // parsed at MessageStop and routed to the reasoning field.
                current_text.push_str(&text);
            }
            AssistantEvent::Reasoning(text) => {
                current_reasoning.push_str(&text);
            }
            AssistantEvent::ToolUse { id, name, input } => {
                // In streaming, parameters arrive in multiple chunks, need to concatenate raw string then parse at once
                if in_tool && id == current_tool_id {
                    if !name.is_empty() {
                        current_tool_name = name;
                    }
                    current_tool_args_raw.push_str(&input);
                    continue;
                }
                // End previous tool (if any)
                if in_tool {
                    let args = serde_json::from_str(&current_tool_args_raw).unwrap_or_else(|e| {
                        let preview: String = current_tool_args_raw.chars().take(200).collect();
                        tracing::warn!(
                            "[common] JSON parse failed for tool '{}': {}. Preview: {}...",
                            current_tool_name,
                            e,
                            preview
                        );
                        serde_json::json!({ "__raw": current_tool_args_raw })
                    });
                    blocks.push(ContentBlock::ToolUse {
                        id: current_tool_id.clone(),
                        name: current_tool_name.clone(),
                        input: args,
                    });
                }
                if !id.is_empty() {
                    current_tool_id = id;
                }
                if !name.is_empty() {
                    current_tool_name = name;
                }
                current_tool_args_raw = input;
                in_tool = true;
            }
            AssistantEvent::MessageStop => {
                let raw_text = std::mem::take(&mut current_text);
                // Extract <think>...</think> reasoning blocks from text.
                // Think content is routed to the reasoning field (shown in execution panel),
                // clean text goes to the chat bubble. This handles cross-chunk splits,
                // incomplete tags, and per-chunk strip remnants.
                let (clean_body, think_reasoning) = crate::utils::extract_think_blocks(&raw_text);

                let mut reasoning = std::mem::take(&mut current_reasoning);
                // Merge think-block reasoning with any reasoning_content from API
                if !think_reasoning.is_empty() {
                    if reasoning.is_empty() {
                        reasoning = think_reasoning;
                    } else {
                        reasoning.push('\n');
                        reasoning.push_str(&think_reasoning);
                    }
                }
                if !reasoning.is_empty() {
                    reasoning = crate::utils::strip_think_tags(&reasoning);
                }
                let reasoning = if reasoning.is_empty() {
                    None
                } else {
                    Some(reasoning)
                };

                let (clean_text, text_calls) = crate::agent::extract_tool_calls_from_text_with_tags(
                    &clean_body,
                    content_tool_tags,
                );
                let clean_text =
                    crate::utils::strip_tool_xml_tags_with_extra(&clean_text, content_tool_tags)
                        .trim()
                        .to_string();
                // Final safety net: strip residual orphaned/truncated close tags
                // (think + built-in tool set + provider extras).
                let clean_text = crate::utils::clean_tag_remnants(&clean_text, content_tool_tags);

                if !clean_text.is_empty() || reasoning.is_some() {
                    blocks.push(ContentBlock::Text {
                        text: clean_text,
                        reasoning,
                    });
                }
                for tc in text_calls {
                    blocks.push(ContentBlock::ToolUse {
                        id: tc.id,
                        name: tc.name,
                        input: tc.arguments,
                    });
                }
                if in_tool {
                    let args = serde_json::from_str(&current_tool_args_raw).unwrap_or_else(|e| {
                        let preview: String = current_tool_args_raw.chars().take(200).collect();
                        tracing::warn!(
                            "[common] JSON parse failed for tool '{}': {}. Preview: {}...",
                            current_tool_name,
                            e,
                            preview
                        );
                        serde_json::json!({ "__raw": current_tool_args_raw })
                    });
                    blocks.push(ContentBlock::ToolUse {
                        id: current_tool_id.clone(),
                        name: current_tool_name.clone(),
                        input: args,
                    });
                    in_tool = false;
                }
            }
            AssistantEvent::Usage {
                input_tokens,
                output_tokens,
                cache_hit_tokens: cache,
            } => {
                usage = Some((input_tokens, output_tokens));
                cache_hit_tokens = cache;
            }
            AssistantEvent::Cancelled => {
                let raw_text = std::mem::take(&mut current_text);
                let (clean_body, think_reasoning) = crate::utils::extract_think_blocks(&raw_text);
                let text =
                    crate::utils::strip_tool_xml_tags_with_extra(&clean_body, content_tool_tags)
                        .trim()
                        .to_string();
                // Final safety net: strip residual orphaned/truncated close tags
                // (think + built-in tool set + provider extras).
                let text = crate::utils::clean_tag_remnants(&text, content_tool_tags);

                let mut reasoning = std::mem::take(&mut current_reasoning);
                if !think_reasoning.is_empty() {
                    if reasoning.is_empty() {
                        reasoning = think_reasoning;
                    } else {
                        reasoning.push('\n');
                        reasoning.push_str(&think_reasoning);
                    }
                }
                let reasoning = if reasoning.is_empty() {
                    None
                } else {
                    Some(reasoning)
                };
                if !text.is_empty() || reasoning.is_some() {
                    blocks.push(ContentBlock::Text { text, reasoning });
                }
                if in_tool {
                    let args = serde_json::from_str(&current_tool_args_raw).unwrap_or_else(|e| {
                        let preview: String = current_tool_args_raw.chars().take(200).collect();
                        tracing::warn!(
                            "[common] JSON parse failed for tool '{}': {}. Preview: {}...",
                            current_tool_name,
                            e,
                            preview
                        );
                        serde_json::json!({ "__raw": current_tool_args_raw })
                    });
                    blocks.push(ContentBlock::ToolUse {
                        id: current_tool_id.clone(),
                        name: current_tool_name.clone(),
                        input: args,
                    });
                }
            }
            AssistantEvent::ConnectionStatus(_) => {
                // Status-only event, no content to accumulate
            }
            AssistantEvent::ImageAttachment { .. } => {
                // Image URL event — handled by the streaming emitter in react_loop,
                // no text content to accumulate here.
            }
        }
    }

    ProcessEventsResult {
        blocks,
        usage,
        cache_hit_tokens,
    }
}

/// Route one streaming `TextDelta` through the shared text cleaner and forward
/// the split results as frontend events.
///
/// Single entry point used by the three runtime stream emitters
/// (react_loop / sub_task_loop / workflow_agent). Runs
/// [`crate::utils::process_text_delta`] with the provider-declared tool tags,
/// then emits any reasoning chunk first (`is_thinking: true`) and any non-empty
/// content text second (`is_thinking: false`) so the frontend timeline always
/// shows thinking before content. A `None` emitter makes this a no-op.
pub fn route_stream_text_delta(
    text: &str,
    think_state: &AtomicU32,
    extra_tags: &[&str],
    from_task: bool,
    emitter: Option<&dyn EventEmitter>,
) {
    let (reasoning, text_clean) = crate::utils::process_text_delta(text, think_state, extra_tags);
    if let Some(emitter) = emitter {
        if let Some(r) = reasoning {
            emitter.emit(NuphusEvent::LlmTextDelta {
                text: r,
                is_thinking: true,
                from_task,
            });
        }
        if !text_clean.is_empty() {
            emitter.emit(NuphusEvent::LlmTextDelta {
                text: text_clean,
                is_thinking: false,
                from_task,
            });
        }
    }
}

/// Extract tool calls from assistant message blocks (dedup + filter empty params)
pub fn extract_tool_calls(blocks: &[ContentBlock]) -> Vec<ToolCall> {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, input } => {
                // Filter null params (empty tool_call from model hallucination), but keep valid empty object {}
                // Parameterless tools (e.g. desktop_windows_list) calling with {} is normal behavior
                if input.is_null() {
                    return None;
                }
                let key = format!(
                    "{}:{}",
                    name,
                    serde_json::to_string(input).unwrap_or_default()
                );
                if !seen.insert(key) {
                    return None;
                }
                Some(ToolCall {
                    id: id.clone(),
                    tool: name.clone(),
                    params: input.clone(),
                })
            }
            _ => None,
        })
        .collect()
}

/// Determine if LLM API error is retryable.
///
/// Non-retryable (immediate failure):
/// - 4xx client errors: 400(bad request), 401(auth), 402(insufficient balance), 403(permission), 404, 422
/// - Invalid API key, model not found, bad request format, insufficient balance
///
/// Retryable (exponential backoff):
/// - 5xx server errors: 500, 502, 503, 504, 529
/// - Network layer errors: connection timeout, DNS, TLS, reset, EOF
/// - Rate limit: 429 (handled at transport layer, but agent layer also falls back)
pub fn is_retryable_llm_error(err: &str) -> bool {
    let e = err.to_lowercase();

    // --- Non-retryable: model/auth/balance/param errors ---
    let non_retryable = [
        "400",
        "bad request",
        "invalid request",
        "401",
        "unauthorized",
        "auth",
        "invalid api key",
        "apikey",
        "402",
        "payment",
        "balance",
        "insufficient",
        "quota",
        "credit",
        "403",
        "forbidden",
        "404",
        "not found",
        "422",
        "unprocessable",
        "invalid model",
        "model not found",
        "unknown model",
        "content filter",
        "safety",
        "moderation",
        "context length",
        "too long",
        "max tokens",
    ];
    for pat in &non_retryable {
        if e.contains(pat) {
            return false;
        }
    }

    // --- Retryable: server/network/rate-limit ---
    let retryable = [
        "500",
        "502",
        "503",
        "504",
        "529",
        "service unavailable",
        "bad gateway",
        "gateway timeout",
        "connection refused",
        "connection reset",
        "connection closed",
        "connection timed out",
        "timed out",
        "timeout",
        "dns",
        "tls",
        "eof",
        "broken pipe",
        "no route to host",
        "network unreachable",
        "name or service not known",
        "i/o error",
        "io error",
        "transport error",
        "protocol error",
        "handshake failed",
        "partial data",
        "unexpected eof",
        "429",
        "rate limit",
        "too many requests",
    ];
    for pat in &retryable {
        if e.contains(pat) {
            return true;
        }
    }

    // Default conservative strategy: allow retry for unknown errors (fallback for network instability)
    true
}

pub fn render_ascii_progress(current: usize, total: usize) -> String {
    let width = 20;
    let filled = (current * width).checked_div(total).unwrap_or(0);
    let percent = (current * 100).checked_div(total).unwrap_or(0);
    let bar = "█".repeat(filled) + &"░".repeat(width - filled);
    format!("[Step {}/{}] {} {}%", current, total, bar, percent)
}

pub fn summarize_tool_params(input: &serde_json::Value) -> String {
    for key in &["path", "command", "query", "url", "file_path", "pattern"] {
        if let Some(val) = input.get(*key).and_then(|v| v.as_str()) {
            let s: String = val.chars().take(60).collect();
            return format!("{}=\"{}\"", key, s);
        }
    }
    String::new()
}

pub fn wants_file_output(input: &str) -> bool {
    let lower = input.to_lowercase();
    // File extension requests
    lower.contains(".md") || lower.contains(".txt") || lower.contains(".json") || lower.contains(".yaml") ||
    lower.contains(".html") || lower.contains(".csv") || lower.contains(".toml") ||
    // Creation / output keywords (multi-lingual)
    lower.contains("报告") || lower.contains("文件") || lower.contains("生成") ||
    lower.contains("创建") || lower.contains("写入") || lower.contains("输出") ||
    lower.contains("保存") || lower.contains("report") || lower.contains("generate") ||
    lower.contains("create") || lower.contains("write") || lower.contains("output") ||
    lower.contains("save") || lower.contains("analysis") || lower.contains("analyze")
}

pub fn is_network_error(err: &str) -> bool {
    let err = err.to_lowercase();
    err.contains("connection refused")
        || err.contains("connection reset")
        || err.contains("connection closed")
        || err.contains("connection timed out")
        || err.contains("timed out")
        || err.contains("timeout")
        || err.contains("dns")
        || err.contains("tls")
        || err.contains("eof")
        || err.contains("broken pipe")
        || err.contains("no route to host")
        || err.contains("network unreachable")
        || err.contains("name or service not known")
        || err.contains("500")
        || err.contains("502")
        || err.contains("503")
        || err.contains("504")
        || err.contains("service unavailable")
        || err.contains("bad gateway")
        || err.contains("i/o error")
        || err.contains("io error")
        || err.contains("transport error")
        || err.contains("protocol error")
        || err.contains("handshake failed")
        || err.contains("partial data")
        || err.contains("unexpected eof")
}
