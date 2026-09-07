//! Responses Transport：HTTP 请求 + 流式读取 + adapter 转译。
//!
//! P2 交付的是独立引擎，未注册到任何 Provider/factory（接线 P4 做）。本文件
//! 骨架思路对齐既有 chat/anthropic transport（收集响应体 → parser 归一为
//! StreamEvent），不复制大段实现；公共 HTTP 流式/重试/代理的收敛改造见
//! transport_base 后续阶段（P4 接线前如确需，先报 Leader 再动既有文件）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use super::config::ResponsesConfig;
use super::parser::ResponsesStreamParser;
use crate::api::{MessageRequest, ToolDefinition};
use crate::transports::{StreamEvent, Transport};
use crate::{LLMError, NuphusError, Result};

/// OpenAI Responses Transport implementation
#[derive(Debug, Clone)]
pub(crate) struct ResponsesTransport {
    config: ResponsesConfig,
}

impl ResponsesTransport {
    pub fn new(config: ResponsesConfig) -> Self {
        Self { config }
    }

    /// Build the Responses API request body from a Nuphus `MessageRequest`
    /// (which carries Chat-Completions-style message roles + system fields).
    ///
    /// 差异（与 chat 的收敛点，设计 §4.4）：
    /// - `messages` → `input`（Responses items 序列；文本消息 + function_call /
    ///   function_call_output 工具项）
    /// - `tools`：Nuphus 存 {type, function:{...}}，Responses 需要扁平
    ///   {type:"function", name, description, parameters}（官方 Responses tools 形态，
    ///   SDK 生成源码见 mod.rs 锚点注释）
    /// - reasoning：Responses 用 `reasoning: {effort}`（与 anthropic 同款 gate：
    ///   quirks.supports_reasoning_effort；effort_excludes_tools 时带工具请求不发）
    fn build_request_body(&self, request: &MessageRequest) -> Value {
        let model = if request.model.is_empty() {
            self.config.model.as_str()
        } else {
            request.model.as_str()
        };
        let mut body = serde_json::json!({
            "model": model,
            "input": Self::build_input(request),
            // Nuphus 所有下游均以流式消费；非流式 JSON 响应不在此引擎路径（P2 无接线）。
            "stream": true,
        });
        if let Some(temperature) = request.temperature {
            body["temperature"] = serde_json::json!(temperature);
        }
        // Responses API 输出 token 预算参数（官方名 max_output_tokens；若接线
        // 真机校验与官方 spec 有出入，以 API 实测为准——见交付差异报告）。
        if let Some(max_tokens) = request.max_tokens {
            body["max_output_tokens"] = serde_json::json!(max_tokens);
        }
        if let Some(tools) = &request.tools {
            body["tools"] =
                serde_json::json!(tools.iter().map(Self::convert_tool_def).collect::<Vec<_>>());
        }
        if self.config.quirks.supports_reasoning_effort
            && (request.tools.is_none() || !self.config.quirks.effort_excludes_tools)
        {
            if let Some(effort) = &self.config.reasoning_effort {
                body["reasoning"] = serde_json::json!({ "effort": effort });
            }
        }
        body
    }

    /// Nuphus ToolDefinition → Responses function tool。
    fn convert_tool_def(tool: &ToolDefinition) -> Value {
        let mut t = serde_json::json!({
            "type": "function",
            "name": tool.function.name,
            "parameters": tool.function.parameters,
        });
        if let Some(desc) = &tool.function.description {
            t["description"] = serde_json::json!(desc);
        }
        t
    }

    /// 把 Chat-Completions 形态消息（session/transform 产出的扁平结构）转译为
    /// Responses `input` items：
    /// - system / merged_system → {role:"system", content:[{type:"input_text",...}]}
    /// - user 文本 → {role:"user", content:[{type:"input_text",...}]}
    /// - assistant 文本 → {role:"assistant", content:[{type:"output_text",...}]}
    ///   （Responses 标准：assistant 侧 content 必须是 output_text/refusal；
    ///   Console Go 网关校验 input_text 非法——2026-09 实测 400）
    /// - assistant.tool_calls → {type:"function_call", call_id, name, arguments}
    /// - tool 结果 → {type:"function_call_output", call_id, output}
    ///
    /// assistant.reasoning_content 不 echo：Responses 模型自带推理管理
    /// （thinking echo 是 chat 协议特需，DeepSeek thinking 不在 Responses 模型族）。
    fn build_input(request: &MessageRequest) -> Vec<Value> {
        let mut items: Vec<Value> = Vec::new();
        let push_system = |items: &mut Vec<Value>, text: &str| {
            items.push(serde_json::json!({
                "role": "system",
                "content": [{"type": "input_text", "text": text}],
            }));
        };
        if let Some(ref merged) = request.merged_system {
            push_system(&mut items, merged);
        } else {
            if let Some(ref sys) = request.system {
                push_system(&mut items, sys);
            }
            for sys_msg in &request.system_messages {
                push_system(&mut items, sys_msg);
            }
        }
        for msg in &request.messages {
            items.extend(Self::convert_message(msg));
        }
        items
    }

    fn convert_message(msg: &Value) -> Vec<Value> {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let content = msg.get("content").unwrap_or(&Value::Null);
        match role {
            // 工具结果 → function_call_output（官方 call_id 关联）。
            "tool" => {
                let call_id = msg
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                vec![serde_json::json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": Self::extract_text(content),
                })]
            }
            "assistant" => {
                let mut out = Vec::new();
                // 文本部分
                let text = Self::extract_text(content);
                if !text.is_empty() {
                    out.push(serde_json::json!({
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": text}],
                    }));
                }
                // 工具调用：展开成独立 function_call items（Responses input 要求
                // assistant 工具项与文本消息分离）。
                if let Some(tool_calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tool_calls {
                        out.push(serde_json::json!({
                            "type": "function_call",
                            "call_id": tc.get("id").and_then(|v| v.as_str()).unwrap_or_default(),
                            "name": tc
                                .pointer("/function/name")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default(),
                            "arguments": tc
                                .pointer("/function/arguments")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default(),
                        }));
                    }
                }
                out
            }
            "system" => {
                let text = Self::extract_text(content);
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![serde_json::json!({
                        "role": "system",
                        "content": [{"type": "input_text", "text": text}],
                    })]
                }
            }
            // user 及其它 role
            _ => {
                if content.is_null() {
                    Vec::new()
                } else {
                    let text = Self::extract_text(content);
                    if text.is_empty() {
                        Vec::new()
                    } else {
                        vec![serde_json::json!({
                            "role": "user",
                            "content": [{"type": "input_text", "text": text}],
                        })]
                    }
                }
            }
        }
    }

    /// 从 chat content（string 或 [{type:text|..., text:...}] 数组）提取纯文本。
    fn extract_text(content: &Value) -> String {
        match content {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        }
    }

    /// POST `{base}/responses`，收集完整响应体（UTF-8 严格校验，与 chat 一致——
    /// 禁止 lossy 把乱码写进下游）。
    ///
    /// 错误分层：
    /// - 连接层/读取层错误 → 轻量重试一次（骨架：退避 1s）；
    /// - HTTP 4xx/5xx → LLMError::ApiError{status, body}（5xx/429 也重试一次）；
    /// - 取消标志在读取前/读取间隙检查 → LLMError::Cancelled。
    async fn send_sse(&self, body: &Value, cancel_flag: Option<&AtomicBool>) -> Result<String> {
        let url = self.config.endpoint();
        let mut last_error: Option<String> = None;

        for attempt in 0..2u32 {
            if let Some(flag) = cancel_flag {
                if flag.load(Ordering::SeqCst) {
                    return Err(NuphusError::LLM(LLMError::Cancelled));
                }
            }
            if attempt > 0 {
                if let Some(e) = &last_error {
                    tracing::warn!(
                        "[responses] retry {}/1 after {}s ({})",
                        attempt,
                        attempt,
                        e.chars().take(80).collect::<String>()
                    );
                }
                tokio::time::sleep(Duration::from_secs(u64::from(attempt))).await;
            }

            let mut builder = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(self.config.timeout_secs));
            let proxy_url = crate::utils::proxy::detect_proxy_url();
            if let Some(proxy) = proxy_url {
                builder = builder.proxy(reqwest::Proxy::all(&proxy).map_err(|e| {
                    NuphusError::LLM(LLMError::HttpBuildFailed {
                        error: format!("proxy {proxy}: {e}"),
                    })
                })?);
            }
            let client = match builder.build() {
                Ok(c) => c,
                Err(e) => {
                    return Err(NuphusError::LLM(LLMError::HttpBuildFailed {
                        error: e.to_string(),
                    }));
                }
            };

            // 鉴权头：配置 header/prefix（Codex 变体可配 authorization/额外头）。
            let mut req = client.post(&url).header("content-type", "application/json");
            let auth_header = if self.config.auth_header.is_empty() {
                "authorization"
            } else {
                self.config.auth_header.as_str()
            };
            let auth_value = format!("{}{}", self.config.auth_prefix, self.config.api_key);
            req = req.header(auth_header, auth_value);
            for (k, v) in &self.config.quirks.extra_headers {
                req = req.header(k, v);
            }
            let req = req.body(body.to_string());

            let response = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    // 连接层错误（TCP/DNS/TLS/超时）→ 重试
                    last_error = Some(format!("responses request failed: {e}"));
                    continue;
                }
            };
            let status = response.status().as_u16();
            if status == 200 {
                let bytes = match response.bytes().await {
                    Ok(b) => b,
                    Err(e) => {
                        last_error = Some(format!("read response failed: {e}"));
                        continue;
                    }
                };
                return match String::from_utf8(bytes.to_vec()) {
                    Ok(s) => Ok(s),
                    Err(e) => Err(NuphusError::LLM(LLMError::StreamError {
                        error: format!("responses 响应非法 UTF-8: {e}"),
                    })),
                };
            }
            // 非 200：读 body 用于诊断。
            let body_text = response
                .text()
                .await
                .unwrap_or_else(|_| "(read body failed)".to_string());
            let api_err = NuphusError::LLM(LLMError::ApiError {
                status,
                body: body_text.clone(),
            });
            // 5xx / 429 重试一次，其余直接失败。
            if status >= 500 || status == 429 {
                last_error = Some(format!("HTTP {status}: {}", body_text));
                continue;
            }
            return Err(api_err);
        }
        Err(NuphusError::LLM(LLMError::RequestFailed {
            error: last_error.unwrap_or_else(|| "responses request failed".to_string()),
        }))
    }
}

#[async_trait]
impl Transport for ResponsesTransport {
    async fn stream(&self, request: MessageRequest) -> Result<Vec<StreamEvent>> {
        let body = self.build_request_body(&request);
        tracing::info!(
            "[REQ] responses model={} input_items={}",
            body["model"],
            body["input"].as_array().map(|a| a.len()).unwrap_or(0)
        );
        let sse = self.send_sse(&body, None).await?;
        ResponsesStreamParser::with_cache_hit_field(self.config.quirks.cache_hit_field)
            .push_sse_text(&sse)
    }

    async fn stream_with_cancellation(
        &self,
        request: MessageRequest,
        cancel_flag: &AtomicBool,
    ) -> Result<Vec<StreamEvent>> {
        let body = self.build_request_body(&request);
        let sse = self.send_sse(&body, Some(cancel_flag)).await?;
        let events =
            ResponsesStreamParser::with_cache_hit_field(self.config.quirks.cache_hit_field)
                .push_sse_text(&sse)?;
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(NuphusError::LLM(LLMError::Cancelled));
        }
        Ok(events)
    }

    fn provider_name(&self) -> &str {
        "responses"
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn provider_kind(&self) -> Option<crate::api::ProviderKind> {
        self.config.provider_kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::provider::ProviderQuirks;

    fn config() -> ResponsesConfig {
        ResponsesConfig {
            base_url: "https://api.openai.com/v1".into(),
            api_key: "sk-test".into(),
            model: "gpt-5.6-luna".into(),
            timeout_secs: 30,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: Some(crate::api::ProviderKind::OpenAI),
            quirks: ProviderQuirks::default(),
            reasoning_effort: None,
        }
    }

    /// merged system + 用户文本 + assistant 工具调用 + tool 结果 →
    /// Responses input items 顺序与格式。
    #[test]
    fn build_input_converts_chat_messages() {
        let transport = ResponsesTransport::new(config());
        let request = MessageRequest::new(
            "gpt-5.6-luna",
            vec![
                serde_json::json!({"role": "user", "content": "请查天气"}),
                serde_json::json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "web_search", "arguments": "{\"query\":\"上海\"}"}
                    }]
                }),
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": "call_1",
                    "content": "晴 26°C"
                }),
            ],
        )
        .with_merged_system("你是助手");

        let body = transport.build_request_body(&request);
        assert_eq!(body["model"], "gpt-5.6-luna");
        assert_eq!(body["stream"], true);

        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 4);
        // merged system → system role
        assert_eq!(input[0]["role"], "system");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "你是助手");
        // user 文本
        assert_eq!(input[1]["role"], "user");
        assert_eq!(input[1]["content"][0]["text"], "请查天气");
        // assistant.tool_calls → 独立 function_call item
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[2]["name"], "web_search");
        assert_eq!(input[2]["arguments"], "{\"query\":\"上海\"}");
        // tool 结果 → function_call_output
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_1");
        assert_eq!(input[3]["output"], "晴 26°C");
    }

    /// 无合并 system 时按 Nuphus 顺序发 system + system_messages。
    #[test]
    fn build_input_falls_back_to_separate_system_messages() {
        let transport = ResponsesTransport::new(config());
        let request = MessageRequest::new("m", vec![])
            .with_system("L0")
            .with_system_messages(vec!["L1".to_string(), "L2".to_string()]);
        let body = transport.build_request_body(&request);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3);
        for (i, expected) in ["L0", "L1", "L2"].iter().enumerate() {
            assert_eq!(input[i]["role"], "system");
            assert_eq!(input[i]["content"][0]["text"], *expected);
        }
    }

    /// Nuphus ToolDefinition（{type,function}）→ Responses 扁平 function tool。
    #[test]
    fn build_tools_converts_to_responses_format() {
        let transport = ResponsesTransport::new(config());
        let request = MessageRequest::new("m", vec![]).with_tools(vec![ToolDefinition {
            tool_type: "function".into(),
            function: crate::api::FunctionDefinition {
                name: "web_search".into(),
                description: Some("搜索".into()),
                parameters: serde_json::json!({"type": "object"}),
                permission: None,
            },
        }]);
        let body = transport.build_request_body(&request);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "web_search");
        assert_eq!(tools[0]["description"], "搜索");
        assert_eq!(tools[0]["parameters"]["type"], "object");
        assert!(
            tools[0].get("function").is_none(),
            "Responses 不需要 function 嵌套"
        );
    }

    /// reasoning effort：quirks 支持时发送 `reasoning.effort`。
    #[test]
    fn reasoning_effort_gated_by_quirks() {
        let mut cfg = config();
        let q = ProviderQuirks {
            supports_reasoning_effort: true,
            ..Default::default()
        };
        cfg.quirks = q;
        cfg.reasoning_effort = Some("high".to_string());
        let transport = ResponsesTransport::new(cfg);
        let body = transport.build_request_body(&MessageRequest::new("m", vec![]));
        assert_eq!(
            body["reasoning"],
            serde_json::json!({ "effort": "high" }),
            "configured effort 应发送为 reasoning.effort"
        );
    }
}
