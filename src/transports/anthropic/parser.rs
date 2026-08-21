//! Anthropic Messages API SSE Event Parser
//!
//! Parses the Anthropic-specific SSE stream format (event:/data: lines)
//! into generic [`StreamEvent`]s so the rest of the system is provider-agnostic.
//!
//! # SSE Event Types
//!
//! | Event | Maps to |
//! |-------|---------|
//! | `message_start` | (usage extraction, no StreamEvent) |
//! | `content_block_start` (text) | starts text accumulation |
//! | `content_block_start` (tool_use) | starts tool accumulation |
//! | `content_block_start` (thinking) | `StreamEvent::Reasoning` |
//! | `content_block_delta` (text_delta) | appended to text accumulator |
//! | `content_block_delta` (thinking_delta) | `StreamEvent::Reasoning` |
//! | `content_block_delta` (input_json_delta) | appended to tool JSON accumulator |
//! | `content_block_stop` | flushes text or tool accumulator |
//! | `message_delta` | `StreamEvent::Usage` |
//! | `message_stop` | (triggers stop, no StreamEvent) |

use crate::transports::StreamEvent;
use crate::Result;

/// Parser state machine for Anthropic SSE responses.
#[derive(Default)]
pub(crate) struct AnthropicSseParser {
    /// Accumulated text content across one or more text blocks.
    text_acc: String,
    /// Accumulated reasoning content.
    reasoning_acc: String,
    /// In-progress tool call: (id, name, partial_json).
    tool_acc: Option<ToolAcc>,
}

#[derive(Default)]
struct ToolAcc {
    id: String,
    name: String,
    input_json: String,
}

impl AnthropicSseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one raw (event_type, data_json) pair into the parser.
    /// Returns any StreamEvents produced by this event.
    pub fn feed(&mut self, _event_type: &str, data: &serde_json::Value) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        let msg_type = data["type"].as_str().unwrap_or("");

        match msg_type {
            "message_start" => {
                // Contains full message with initial content blocks and usage.
                // We only extract usage from the top-level message.
                if let Some(usage) = data.get("usage") {
                    out.push(self.make_usage(usage));
                }
                // message_start can also contain initial content blocks in
                // "content". Anthropic sometimes sends empty text blocks here.
            }

            "content_block_start" => {
                let block = &data["content_block"];
                let block_type = block["type"].as_str().unwrap_or("");
                match block_type {
                    "text" => {
                        // Start a new text block (text_delta will follow).
                        let text = block["text"].as_str().unwrap_or("");
                        if !text.is_empty() {
                            self.text_acc.push_str(text);
                        }
                    }
                    "tool_use" => {
                        // Start accumulating a tool call.
                        let id = block["id"].as_str().unwrap_or("").to_string();
                        let name = block["name"].as_str().unwrap_or("").to_string();
                        let initial = block["input"].to_string();
                        self.tool_acc = Some(ToolAcc {
                            id,
                            name,
                            input_json: if initial == "null" || initial == "{}" {
                                String::new()
                            } else {
                                initial
                            },
                        });
                    }
                    "thinking" => {
                        if let Some(t) = block["thinking"].as_str() {
                            if !t.is_empty() {
                                self.reasoning_acc.push_str(t);
                                out.push(StreamEvent::Reasoning(t.to_string()));
                            }
                        }
                    }
                    "redacted_thinking" => {
                        // Redacted thinking blocks don't contain visible content.
                        // We emit a placeholder so the UI knows thinking happened.
                        out.push(StreamEvent::Reasoning("[redacted]".to_string()));
                    }
                    _ => {}
                }
            }

            "content_block_delta" => {
                let delta = &data["delta"];
                let delta_type = delta["type"].as_str().unwrap_or("");
                match delta_type {
                    "text_delta" => {
                        if let Some(text) = delta["text"].as_str() {
                            self.text_acc.push_str(text);
                        }
                    }
                    "thinking_delta" => {
                        if let Some(t) = delta["thinking"].as_str() {
                            self.reasoning_acc.push_str(t);
                            out.push(StreamEvent::Reasoning(t.to_string()));
                        }
                    }
                    "input_json_delta" => {
                        if let Some(partial) = delta["partial_json"].as_str() {
                            if let Some(ref mut tool) = self.tool_acc {
                                tool.input_json.push_str(partial);
                            }
                        }
                    }
                    "signature_delta" => {
                        // Anthropic Extended Thinking signature — opaque bytes.
                        // We ignore it; the signature is handled server-side.
                    }
                    _ => {}
                }
            }

            "content_block_stop" => {
                // Finalise the current content block.
                // If we accumulated text, emit it.
                if !self.text_acc.is_empty() {
                    let text = std::mem::take(&mut self.text_acc);
                    out.push(StreamEvent::TextDelta(text));
                }
                if !self.reasoning_acc.is_empty() {
                    std::mem::take(&mut self.reasoning_acc);
                    // reasoning was already emitted as events above
                }
                // If we accumulated a tool call, emit it.
                if let Some(tool) = self.tool_acc.take() {
                    out.push(StreamEvent::ToolUse {
                        id: tool.id,
                        name: tool.name,
                        arguments: tool.input_json,
                    });
                }
            }

            "message_delta" => {
                // Final usage info (may include cache metrics).
                if let Some(usage) = data.get("usage") {
                    out.push(self.make_usage(usage));
                }
                // stop_reason from delta can be used but we don't map it
                // to a separate event — the final Done event is sufficient.
            }

            "message_stop" => {
                // Response fully received. No data payload.
            }

            "error" => {
                // API returned an error event.
                let err_msg = data["error"]["message"]
                    .as_str()
                    .unwrap_or("unknown anthropic API error");
                out.push(StreamEvent::Error(err_msg.to_string()));
            }

            "ping" => {
                // Keep-alive — ignore.
            }

            _ => {
                tracing::debug!("Unknown Anthropic SSE event type: {msg_type}");
            }
        }

        out
    }

    /// Finalise the stream: flush any remaining accumulator state.
    /// Call this after the response body has been fully consumed.
    pub fn finalise(&mut self) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        if !self.text_acc.is_empty() {
            out.push(StreamEvent::TextDelta(std::mem::take(&mut self.text_acc)));
        }
        if !self.reasoning_acc.is_empty() {
            std::mem::take(&mut self.reasoning_acc);
        }
        if let Some(tool) = self.tool_acc.take() {
            out.push(StreamEvent::ToolUse {
                id: tool.id,
                name: tool.name,
                arguments: tool.input_json,
            });
        }
        // Always emit Done to signal the end.
        out.push(StreamEvent::Done);
        out
    }

    // ── helpers ──────────────────────────────────────────────────────────

    fn make_usage(&self, usage: &serde_json::Value) -> StreamEvent {
        let input = usage
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let output = usage
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let cache_hit = usage
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        StreamEvent::Usage {
            input_tokens: input,
            output_tokens: output,
            cache_hit_tokens: cache_hit,
        }
    }
}

// ── top-level entry point ───────────────────────────────────────────────────

/// Parse a full Anthropic SSE response body into `StreamEvent`s.
///
/// `body` is the raw SSE text (lines prefixed with `event:` / `data:`).
pub(crate) fn parse_sse(body: &str) -> Result<Vec<StreamEvent>> {
    let mut parser = AnthropicSseParser::new();
    let mut events = Vec::new();
    let mut current_event = String::new();

    for raw_line in body.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        // Track the event: line (e.g. "event: content_block_delta")
        if let Some(event_name) = line.strip_prefix("event:") {
            current_event = event_name.trim().to_string();
            continue;
        }

        // Parse data: line
        let data_str = match line.strip_prefix("data:") {
            Some(d) => d.trim(),
            None => continue,
        };

        // Skip the stream-end sentinel
        if data_str == "[DONE]" {
            continue;
        }

        let data: serde_json::Value = match serde_json::from_str(data_str) {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!("Failed to parse Anthropic SSE data line: {data_str:?}");
                continue;
            }
        };

        // If no event type was explicitly set, infer it from data["type"]
        let event_type = if current_event.is_empty() {
            data["type"].as_str().unwrap_or("").to_string()
        } else {
            std::mem::take(&mut current_event)
        };

        events.extend(parser.feed(&event_type, &data));
    }

    events.extend(parser.finalise());
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_text_response() {
        let sse = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-20250514","stop_reason":null,"usage":{"input_tokens":10,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello, "}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world!"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":10,"output_tokens":5}}

event: message_stop
data: {"type":"message_stop"}"#;

        let events = parse_sse(sse).unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::TextDelta(t) if t == "Hello, world!")));
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Done)));
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::Usage {
                input_tokens: 10,
                output_tokens: 5,
                ..
            }
        )));
    }

    #[test]
    fn test_tool_use_response() {
        let sse = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_abc","name":"get_weather","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"loc"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"ation\": \"SF\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"input_tokens":20,"output_tokens":10}}

event: message_stop
data: {"type":"message_stop"}"#;

        let events = parse_sse(sse).unwrap();
        let tool_use: Vec<_> = events
            .iter()
            .filter_map(|e| {
                if let StreamEvent::ToolUse {
                    id,
                    name,
                    arguments,
                } = e
                {
                    Some((id.as_str(), name.as_str(), arguments.as_str()))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(tool_use.len(), 1);
        assert_eq!(tool_use[0].0, "toolu_abc");
        assert_eq!(tool_use[0].1, "get_weather");
        assert!(tool_use[0].2.contains("\"location\""));
    }

    #[test]
    fn test_thinking_response() {
        let sse = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"Let me think step"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" by step..."}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Here's the answer."}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":5,"output_tokens":20}}

event: message_stop
data: {"type":"message_stop"}"#;

        let events = parse_sse(sse).unwrap();
        let reasoning: Vec<_> = events
            .iter()
            .filter_map(|e| {
                if let StreamEvent::Reasoning(t) = e {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(!reasoning.is_empty(), "should have reasoning events");
        // text content should also be present
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::TextDelta(t) if t == "Here's the answer.")));
    }

    #[test]
    fn test_multiple_text_blocks() {
        // Some responses have multiple text blocks with different stop_reasons
        let sse = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Part one."}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Part two."}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":10,"output_tokens":5}}

event: message_stop
data: {"type":"message_stop"}"#;

        let events = parse_sse(sse).unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::TextDelta(t) if t == "Part one.")));
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::TextDelta(t) if t == "Part two.")));
    }

    #[test]
    fn test_error_event() {
        let sse = r#"event: error
data: {"type":"error","error":{"type":"invalid_request_error","message":"Invalid API key"}}"#;

        let events = parse_sse(sse).unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::Error(msg) if msg == "Invalid API key")));
    }
}
