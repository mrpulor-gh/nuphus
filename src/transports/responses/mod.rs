//! OpenAI Responses API Transport（P2 引擎 + P4 接线 opencode-go responses 模型族）
//!
//! 目标：把 OpenAI Responses API（`POST {base}/responses`）的 SSE 流归一到
//! Nuphus 内部 [`StreamEvent`]，供 opencode-go responses 模型族（gpt-5.6-luna /
//! grok-4.5 / grok-4.6 / muse-spark-*）与后续 Codex provider 使用。
//! P4 起由 `llm/factory.rs::build_transport` 按 `transport_for` 分派构造；
//! 其余 chat/anthropic 现网路径不受影响。
//!
//! 事件名与载荷锚点（官方 OpenAPI spec 生成的 SDK 源码，事件名以官方为准）：
//! - 事件全集: https://github.com/openai/openai-python/blob/main/src/openai/types/responses/response_stream_event.py
//! - `response.output_text.delta`:   .../response_text_delta_event.py      (field: delta)
//! - `response.reasoning_summary_text.delta`: .../response_reasoning_summary_text_delta_event.py (field: delta)
//! - `response.function_call_arguments.delta`: .../response_function_call_arguments_delta_event.py (field: delta)
//! - `response.output_item.done`:    .../response_output_item_done_event.py (field: item)
//!   - function_call item 形态:     .../response_function_tool_call.py      (type="function_call",
//!     fields: id/call_id/name/arguments)
//! - `response.failed`:              .../response_failed_event.py
//! - `error`:                        .../response_error_event.py            (fields: code/message/param)
//!
//! 流式 guide: https://developers.openai.com/docs/guides/streaming-responses
//!
//! 与设计文档 §4.4 差异说明（交付时报告 Leader，官方为准）：
//! - Nuphus StreamEvent 无 ToolCallArgumentsDelta 变体（只有完整 ToolUse），故
//!   `function_call_arguments.delta` 在内部**累积**，由 `output_item.done`
//!   (type=function_call) 产出完整 ToolUse——中间态不进事件流。

// P4 factory match 接线：ResponsesConfig/ResponsesTransport 由 factory 分派构造。
// （parser 保持模块内私有——仅 transport.rs 内部使用。）
pub(crate) use config::ResponsesConfig;
pub(crate) use transport::ResponsesTransport;

mod config;
mod parser;
mod transport;
