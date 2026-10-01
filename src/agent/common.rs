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
            AssistantEvent::StreamTruncated { .. } => {
                // 传输截断信号：仅作状态提示（react_loop 转发 HUD / api-health），
                // 不产生任何会话内容（不落 session、不持久化）。
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

/// Emit one newline as a content `LlmTextDelta` — the **inter-iteration text
/// boundary separator** (issue #66 follow-up).
///
/// Why this exists: the frontend accumulates `llm_text_delta` by pure string
/// concatenation (`useEvents.ts`: `m.content + event.text` and
/// `last.text + event.text`) with **no separator of its own** — it cannot know
/// where one LLM attempt ended and the next began. When a run spans multiple
/// react iterations (the normal case: each tool call is followed by another
/// LLM turn), the last sentence of turn N and the first of turn N+1 end up
/// directly adjacent. `MarkdownContent` then sees one contiguous block with no
/// `\n`, takes its single-line branch, and emits no `span.md-line` — so every
/// step's process text collapses into one unbroken run of characters.
///
/// Inserting the break on the Rust side rather than the frontend fixes all
/// consumers at once (desktop bubble + desktop timeline + `mobile/store.ts`).
/// A `None` emitter makes this a no-op.
pub fn emit_text_break(emitter: Option<&dyn EventEmitter>, from_task: bool) {
    if let Some(emitter) = emitter {
        emitter.emit(NuphusEvent::LlmTextDelta {
            text: "\n".to_string(),
            is_thinking: false,
            from_task,
        });
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
///
/// Returns whether **content** text was emitted (reasoning-only chunks, or
/// chunks fully consumed by think-tag handling, return `false`). Callers use
/// this to decide whether an iteration contributed visible text — see
/// [`emit_text_break`].
pub fn route_stream_text_delta(
    text: &str,
    think_state: &AtomicU32,
    extra_tags: &[&str],
    from_task: bool,
    emitter: Option<&dyn EventEmitter>,
) -> bool {
    let (reasoning, text_clean) = crate::utils::process_text_delta(text, think_state, extra_tags);
    let mut emitted_content = false;
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
            emitted_content = true;
        }
    }
    emitted_content
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
        "invalid_api_key",
        "incorrect api key",
        "api key",
        "apikey",
        "api_key",
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

/// LLM 错误类别 —— 用于把「不可重试」的单一结论细化为可行动的指引。
///
/// 由来：`is_retryable_llm_error` 只答「是否重试」，于是所有非重试错误共用一句
/// 「请检查配置或模型状态」。服务商返回 400 内容审核拦截时（StepFun 措辞
/// `Content Exists Risk`、OpenAI 系 `content_policy_violation`、阿里系
/// `DataInspectionFailed`……），用户被引去翻配置 / 换模型 / 查余额，全是白折腾，
/// 而正确动作是「新开会话 / 换服务商 / 精简上下文」。
///
/// 设计约束：
/// - **永远不改重试语义**。本函数与 `is_retryable_llm_error` 平行，独立判定，
///   两边结论必须一致；调用方仍以 `is_retryable_llm_error` 决定是否重试。
/// - **关键词为辅、原文兜底为主**。各服务商风控措辞差异极大且会变，命中不了
///   任何已知词时回落 `Unknown`，文案仍带上服务商原始错误体（调用方负责）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmErrorKind {
    /// 鉴权失败：Key 无效 / 过期 / 无权限。
    Auth,
    /// 余额 / 配额不足。
    Balance,
    /// 内容被服务商安全审核拦截（风控）。
    ContentPolicy,
    /// 模型或参数不被该服务商接受。
    Model,
    /// 上下文超长 / 超出 token 上限。
    TooLong,
    /// 其它不可重试错误（含参数错误）。
    Param,
    /// 未命中任何已知类别 —— 保留原始错误体，让用户/维护者自行判断。
    Unknown,
}

impl LlmErrorKind {
    /// 面向用户的可行动中文指引（不含原始错误体，调用方负责附上）。
    ///
    /// 每条都指向**真正能解决问题的动作**，而不是把用户引向无关方向。
    pub fn guidance_zh(&self) -> &'static str {
        match self {
            LlmErrorKind::Auth => {
                "API Key 无效或无权限，请到设置 → 模型服务商检查该服务商的密钥后重试"
            }
            LlmErrorKind::Balance => "账户余额或配额不足，请到对应服务商控制台查询后重试",
            LlmErrorKind::ContentPolicy => {
                "请求内容被该服务商的安全审核拦截，重试无意义。\
                 可换一个服务商或模型、精简历史上下文，或新开会话后重试"
            }
            LlmErrorKind::Model => {
                "该服务商不接受此模型或参数，请在设置 → 模型服务商更换模型后重试"
            }
            LlmErrorKind::TooLong => {
                "上下文超出该模型的输入上限，请新开会话、精简历史消息，或换用上下文更大的模型"
            }
            LlmErrorKind::Param => {
                "请求参数有误，请检查当前模型与相关配置；若持续出现请附上错误码反馈"
            }
            LlmErrorKind::Unknown => {
                "服务商拒绝了本次请求，重试无意义。请保留下方原始错误信息便于反馈"
            }
        }
    }
}

/// 与 [`is_retryable_llm_error`] 共享同一套关键词表，但把命中项**归类**而非只给 bool。
///
/// 顺序即优先级：先判最具体的类别。内容审核类关键词刻意排在 Auth/Balance 之前 ——
/// 像 `Content Exists Risk` 这类风控错误体里也可能出现 "request"/"invalid"，
/// 若不先判就会被 `Param` 抢走。
pub fn classify_llm_error(err: &str) -> LlmErrorKind {
    let e = err.to_lowercase();

    // --- 内容审核拦截（风控）--- 已知措辞各不相同，只能尽量覆盖
    let content_policy = [
        "content filter",
        "safety",
        "moderation",
        "content policy",
        "content_policy_violation",
        "content exists risk",
        "data inspection failed",
        "data_inspection_failed",
        "risk detected",
        "risky content",
        "敏感",
        "审核",
    ];
    for pat in &content_policy {
        if e.contains(pat) {
            return LlmErrorKind::ContentPolicy;
        }
    }

    // --- 上下文超长 ---
    let too_long = [
        "context length",
        "context_length",
        "too long",
        "max tokens",
        "maximum context",
        "token limit",
        "prompt is too long",
        "reduce the length",
    ];
    for pat in &too_long {
        if e.contains(pat) {
            return LlmErrorKind::TooLong;
        }
    }

    // --- 模型不被接受 --- 只收明确的「模型名」错误，避免与 Param 混同
    let model = [
        "invalid model",
        "model not found",
        "unknown model",
        "model_not_found",
        "unsupported model",
        "model does not exist",
        "no such model",
        "decommissioned",
    ];
    for pat in &model {
        if e.contains(pat) {
            return LlmErrorKind::Model;
        }
    }

    // --- 余额 / 配额 ---
    let balance = [
        "402",
        "payment",
        "payment required",
        "balance",
        "insufficient",
        "quota",
        "credit",
        "billing",
    ];
    for pat in &balance {
        if e.contains(pat) {
            return LlmErrorKind::Balance;
        }
    }

    // --- 鉴权 ---
    let auth = [
        "401",
        "unauthorized",
        "invalid api key",
        "invalid_api_key",
        "incorrect api key",
        "api key",
        "apikey",
        "api_key",
        "403",
        "forbidden",
        "permission denied",
    ];
    for pat in &auth {
        if e.contains(pat) {
            return LlmErrorKind::Auth;
        }
    }

    // --- 参数 / 通用 400 --- 放在最后：上面四类都没命中时，400 系列才归到这里。
    // 这也保证「400 + 内容审核」的响应先被 ContentPolicy 接走（见上方排序说明）。
    let param = [
        "400",
        "bad request",
        "invalid request",
        "unprocessable",
        "422",
        "invalid parameter",
        "missing parameter",
        "malformed",
    ];
    for pat in &param {
        if e.contains(pat) {
            return LlmErrorKind::Param;
        }
    }

    LlmErrorKind::Unknown
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

#[cfg(test)]
mod tests {
    use super::*;

    /// issue #92 的原始触发样本：StepFun 的 400 风控响应。
    /// 关键点：`Content Exists Risk` 不在 `is_retryable_llm_error` 的关键词表里，
    /// 靠 `"400"` 命中（不可重试结论正确），但分类必须落在 ContentPolicy，
    /// 而不是被 Param 抢走 —— 否则用户仍被引去「检查配置」。
    #[test]
    fn classifies_stepfun_content_risk_as_content_policy() {
        let err = "LLM error: API error 400: {\"error\":{\"message\":\"Content Exists Risk \
                    (request_id: abc123)\",\"type\":\"invalid_request_error\",\"param\":null,\
                    \"code\":\"invalid_request_error\"}}";
        assert_eq!(
            classify_llm_error(err),
            LlmErrorKind::ContentPolicy,
            "400 + Content Exists Risk 必须归类为内容审核拦截"
        );
        // 不可重试语义未被改动
        assert!(!is_retryable_llm_error(err), "400 仍须判定为不可重试");
    }

    /// 各服务商风控措辞差异极大，逐个钉住已知形态。
    #[test]
    fn classifies_vendor_specific_moderation_phrasings() {
        assert_eq!(
            classify_llm_error("400 content_policy_violation"),
            LlmErrorKind::ContentPolicy,
            "OpenAI 系措辞"
        );
        assert_eq!(
            classify_llm_error("DataInspectionFailed: input data inspection failed"),
            LlmErrorKind::ContentPolicy,
            "阿里系措辞"
        );
        assert_eq!(
            classify_llm_error("The response was filtered due to the content filter"),
            LlmErrorKind::ContentPolicy,
            "content filter 措辞"
        );
    }

    /// 关键反模式：分类必须比「400」更具体。
    /// 若 ordering 写错（把 Param 的 400 判在前面），风控错误会被误判成参数错误。
    #[test]
    fn content_policy_wins_over_generic_400() {
        // 同时含 400 与风控词
        assert_eq!(
            classify_llm_error("400 Bad Request: content exists risk"),
            LlmErrorKind::ContentPolicy
        );
    }

    /// 纯 400（无风控词）→ Param，不再一律说「检查配置或模型状态」以外的领域。
    #[test]
    fn plain_400_falls_back_to_param() {
        assert_eq!(classify_llm_error("API error 400"), LlmErrorKind::Param);
    }

    #[test]
    fn classifies_auth_and_balance() {
        assert_eq!(
            classify_llm_error("401 Unauthorized: invalid api key"),
            LlmErrorKind::Auth
        );
        assert_eq!(
            classify_llm_error("402 Payment Required: insufficient balance"),
            LlmErrorKind::Balance
        );
        assert_eq!(
            classify_llm_error("You exceeded your current quota"),
            LlmErrorKind::Balance
        );
    }

    #[test]
    fn classifies_model_and_too_long() {
        assert_eq!(
            classify_llm_error("404 model not found: gpt-4o-mini"),
            LlmErrorKind::Model,
            "模型名错误应归 Model，不是 Param"
        );
        assert_eq!(
            classify_llm_error("400 This model's maximum context length is 8192 tokens"),
            LlmErrorKind::TooLong,
            "上下文超长应有独立类别"
        );
    }

    /// 未知错误必须保留 Unknown，且每条类别都要有非空指引（不能在界面上出现空白）。
    #[test]
    fn unknown_kind_has_nonempty_guidance() {
        assert_eq!(
            classify_llm_error("something entirely unexpected happened"),
            LlmErrorKind::Unknown
        );
        for kind in [
            LlmErrorKind::Auth,
            LlmErrorKind::Balance,
            LlmErrorKind::ContentPolicy,
            LlmErrorKind::Model,
            LlmErrorKind::TooLong,
            LlmErrorKind::Param,
            LlmErrorKind::Unknown,
        ] {
            assert!(
                !kind.guidance_zh().is_empty(),
                "{kind:?} 的用户指引不能为空"
            );
        }
    }

    /// 网络类错误仍须判为可重试（分类与重试语义互不干扰）。
    #[test]
    fn network_errors_stay_retryable_and_uncategorized() {
        assert!(is_retryable_llm_error("503 service unavailable"));
        assert!(is_retryable_llm_error("connection reset by peer"));
        // 分类对网络错误仍会给出类别（关键词可能擦边），但重试决策不受它影响
        assert!(is_retryable_llm_error("429 Too Many Requests"));
    }
}
