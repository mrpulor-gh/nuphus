use super::super::transport_base::StreamEvent;
use super::config::ChatCompletionsConfig;
use super::schema_fix::sanitize_tool_name;
use crate::api::AssistantEvent;
use crate::Result;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Read cache hit tokens from usage JSON, using provider-specific field name.
/// Falls back to OpenAI standard `prompt_tokens_details.cached_tokens` when field is empty.
fn read_cache_hit(usage: &serde_json::Value, field: &str) -> u32 {
    if field.is_empty() {
        // Try OpenAI standard path
        usage
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32
    } else {
        usage.get(field).and_then(|v| v.as_u64()).unwrap_or(0) as u32
    }
}

/// Chat Completions Transport implementation
#[derive(Debug, Clone)]
pub struct ChatCompletionsTransport {
    config: ChatCompletionsConfig,
}

impl ChatCompletionsTransport {
    pub fn new(config: ChatCompletionsConfig) -> Self {
        Self { config }
    }

    pub fn with_model(mut self, model: &str) -> Self {
        self.config.model = model.to_string();
        self
    }

    /// 判断错误是否为网络连接层错误（TCP/DNS/TLS），而非 HTTP 服务端错误
    fn is_connection_error(err_str: &str) -> bool {
        let e = err_str.to_lowercase();
        e.contains("connect")
            || e.contains("timeout")
            || e.contains("timed out")
            || e.contains("dns")
            || e.contains("tls")
            || e.contains("refused")
            || e.contains("reset")
            || e.contains("eof")
            || e.contains("broken pipe")
            || e.contains("no route to host")
            || e.contains("network unreachable")
            || e.contains("name or service not known")
    }

    /// Send chat.completions request with retry on transient errors.
    /// Retried: 529/503/502 status codes (4 attempts, backoff 2s/4s/8s),
    /// and connection errors (3 attempts, backoff 1s/2s).
    /// When cancel_flag is provided, the response body is streamed chunk-by-chunk
    /// so cancellation can interrupt an in-flight response.
    async fn send_chat_request(
        &self,
        body: serde_json::Value,
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<String> {
        let url = self.config.endpoint();
        let mut last_error = String::new();
        let proxy_url = crate::utils::proxy::detect_proxy_url();
        let mut use_proxy = false;
        let mut connection_errors = 0u32;
        let max_connection_retries: u32 = 2;

        // Log summary
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            let asst_count = messages
                .iter()
                .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
                .count();
            tracing::info!(
                "[REQ] {} total messages, {} assistant, to {}",
                messages.len(),
                asst_count,
                url
            );
        }

        for attempt in 0..4 {
            if let Some(flag) = cancel_flag {
                if flag.load(Ordering::SeqCst) {
                    return Err(crate::NuphusError::LLM(crate::LLMError::Cancelled));
                }
            }

            if attempt > 0 {
                let is_conn_err = Self::is_connection_error(&last_error);
                let delay: u64 = if is_conn_err {
                    connection_errors as u64 // 1s, 2s
                } else {
                    2u64.pow(attempt as u32) // 2s, 4s, 8s
                };
                let preview: String = last_error.chars().take(80).collect();
                tracing::warn!("Retry {}/3 after {}s ({})", attempt, delay, preview);
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }

            let mut client_builder = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(self.config.timeout_secs))
                .pool_max_idle_per_host(0);

            // 先直连，连不上再 fallback 到代理
            if use_proxy {
                if let Some(ref url) = proxy_url {
                    tracing::info!("[PROXY] Fallback to proxy: {}", url);
                    if let Some(proxy) = crate::utils::proxy::build_reqwest_proxy(url) {
                        client_builder = client_builder.proxy(proxy);
                    } else {
                        tracing::warn!("[PROXY] Invalid proxy URL: {}", url);
                    }
                }
            } else {
                tracing::debug!("[PROXY] Direct connection (attempt {})", attempt);
            }

            // Custom User-Agent from Provider quirks (e.g. Kimi Code API requires a specific one)
            if let Some(ua) = &self.config.quirks.user_agent {
                client_builder = client_builder.user_agent(*ua);
            }

            let client = match client_builder.build() {
                Ok(c) => c,
                Err(e) => {
                    last_error = format!("HTTP client build error: {}", e);
                    continue;
                }
            };

            let auth_value = format!("{}{}", self.config.auth_prefix, self.config.api_key);
            tracing::debug!(
                "[REQ] Auth: header={}, key_len={}",
                self.config.auth_header,
                self.config.api_key.len()
            );
            let response = match client
                .post(&url)
                .header(&self.config.auth_header, &auth_value)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let is_connect_err = e.is_connect() || e.is_timeout();
                    last_error = format!("Request failed: {}", e);
                    if e.is_timeout() {
                        tracing::warn!(
                            timeout_s = self.config.timeout_secs,
                            "LLM API request timeout"
                        );
                    }
                    if Self::is_connection_error(&last_error) {
                        connection_errors += 1;
                        if connection_errors > max_connection_retries {
                            break;
                        }
                        // 直连失败 → fallback 到代理重试
                        if !use_proxy && proxy_url.is_some() && is_connect_err {
                            tracing::info!(
                                "[PROXY] Direct connection failed, falling back to proxy"
                            );
                            use_proxy = true;
                        }
                    }
                    continue;
                }
            };

            let status = response.status().as_u16();

            // Stream response body chunk-by-chunk so cancellation can interrupt mid-response
            let mut body_bytes = Vec::new();
            let stream_start = std::time::Instant::now();
            let mut total_chunks = 0u64;
            let mut last_bytes_len = 0usize;
            const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
            {
                let mut stream = response.bytes_stream();
                use futures_util::StreamExt;
                loop {
                    // Check cancel flag BEFORE waiting for next chunk
                    if let Some(flag) = cancel_flag {
                        if flag.load(Ordering::SeqCst) {
                            return Err(crate::NuphusError::LLM(crate::LLMError::Cancelled));
                        }
                    }
                    // Per-chunk timeout: prevents hang when server pauses between SSE chunks
                    let timed = tokio::time::timeout(CHUNK_TIMEOUT, stream.next()).await;
                    let chunk = match timed {
                        Ok(Some(Ok(b))) => b,
                        Ok(Some(Err(e))) => {
                            last_error = format!("Failed to read chunk (HTTP {}): {}", status, e);
                            tracing::error!(error = %e, status = status, "LLM stream read error");
                            break;
                        }
                        Ok(None) => break, // stream ended
                        Err(_elapsed) => {
                            last_error = format!(
                                "Chunk read timeout after {}s (HTTP {})",
                                CHUNK_TIMEOUT.as_secs(),
                                status
                            );
                            tracing::error!(
                                timeout_s = CHUNK_TIMEOUT.as_secs(),
                                status = status,
                                "LLM stream chunk timeout"
                            );
                            break;
                        }
                    };
                    body_bytes.extend_from_slice(&chunk);
                    total_chunks += 1;
                    // Log progress every 50 chunks or >1MB growth
                    let new_len = body_bytes.len();
                    if total_chunks.is_multiple_of(50) || new_len - last_bytes_len > 1_000_000 {
                        last_bytes_len = new_len;
                        let elapsed = stream_start.elapsed().as_secs();
                        tracing::debug!(
                            "[STREAM] chunk={}, bytes={}, elapsed={}s",
                            total_chunks,
                            new_len,
                            elapsed
                        );
                    }
                }
            }
            let stream_elapsed = stream_start.elapsed().as_millis();
            tracing::debug!(
                "[STREAM] complete: {} chunks, {} bytes, {}ms",
                total_chunks,
                body_bytes.len(),
                stream_elapsed
            );

            // If chunk reading failed, the error was set in last_error → retry
            if !last_error.is_empty() {
                continue;
            }

            let resp_body = String::from_utf8_lossy(&body_bytes).to_string();

            if status == 200 {
                if attempt > 0 {
                    tracing::info!(attempt = attempt + 1, url = %url, "LLM API retry succeeded");
                }
                return Ok(resp_body);
            }

            tracing::warn!(status = status, url = %url, "LLM API returned non-200");
            // Retry on transient server errors
            if status == 529 || status == 503 || status == 502 {
                last_error = format!(
                    "API error {}: {}",
                    status,
                    resp_body.chars().take(200).collect::<String>()
                );
                continue;
            }

            // For auth errors, include diagnostic info
            if status == 401 {
                tracing::warn!(
                    "[AUTH] 401 from {} (key length {})",
                    url,
                    self.config.api_key.len()
                );
                return Err(crate::NuphusError::LLM(crate::LLMError::ApiError {
                    status: 401,
                    body: resp_body.chars().take(300).collect::<String>(),
                }));
            }

            return Err(crate::NuphusError::LLM(crate::LLMError::ApiError {
                status,
                body: resp_body.chars().take(500).collect::<String>(),
            }));
        }

        Err(crate::NuphusError::LLM(
            crate::LLMError::RetryLoopExhausted { last_error },
        ))
    }

    /// Streaming send: callback emitter after reading each SSE line (for real-time TextDelta push)
    /// Parse SSE while reading, avoiding full buffering
    async fn send_chat_request_streaming(
        &self,
        body: serde_json::Value,
        cancel_flag: Option<&AtomicBool>,
        emitter: Box<dyn Fn(AssistantEvent) + Send>,
    ) -> Result<()> {
        let url = self.config.endpoint();
        let mut last_error = String::new();
        let proxy_url = crate::utils::proxy::detect_proxy_url();
        let mut use_proxy = false;
        let mut connection_errors = 0u32;
        let max_connection_retries: u32 = 2;

        for attempt in 0..4 {
            if let Some(flag) = cancel_flag {
                if flag.load(Ordering::SeqCst) {
                    return Err(crate::NuphusError::LLM(crate::LLMError::Cancelled));
                }
            }

            if attempt > 0 {
                let is_conn_err = Self::is_connection_error(&last_error);
                let delay: u64 = if is_conn_err {
                    connection_errors as u64 // 1s, 2s
                } else {
                    2u64.pow(attempt as u32) // 2s, 4s, 8s
                };
                let preview: String = last_error.chars().take(80).collect();
                tracing::warn!(
                    "[STREAM] Retry {}/3 after {}s ({})",
                    attempt,
                    delay,
                    preview
                );
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }

            let mut client_builder = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(self.config.timeout_secs))
                .pool_max_idle_per_host(0);

            // 先直连，连不上再 fallback 到代理
            if use_proxy {
                if let Some(ref url) = proxy_url {
                    tracing::info!("[PROXY-STREAM] Fallback to proxy: {}", url);
                    if let Some(proxy) = crate::utils::proxy::build_reqwest_proxy(url) {
                        client_builder = client_builder.proxy(proxy);
                    } else {
                        tracing::warn!("[PROXY-STREAM] Invalid proxy URL: {}", url);
                    }
                }
            } else {
                tracing::debug!("[PROXY-STREAM] Direct connection (attempt {})", attempt);
            }

            // Custom User-Agent from Provider quirks (e.g. Kimi Code API requires a specific one)
            if let Some(ua) = &self.config.quirks.user_agent {
                client_builder = client_builder.user_agent(*ua);
            }

            let client = match client_builder.build() {
                Ok(c) => c,
                Err(e) => {
                    last_error = format!("HTTP client build error: {}", e);
                    continue;
                }
            };

            let auth_value = format!("{}{}", self.config.auth_prefix, self.config.api_key);
            let response = match client
                .post(&url)
                .header(&self.config.auth_header, &auth_value)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let is_connect_err = e.is_connect() || e.is_timeout();
                    last_error = format!("Request failed: {}", e);
                    if e.is_timeout() {
                        tracing::warn!(
                            timeout_s = self.config.timeout_secs,
                            "LLM API streaming request timeout"
                        );
                    }
                    if Self::is_connection_error(&last_error) {
                        connection_errors += 1;
                        if connection_errors > max_connection_retries {
                            break;
                        }
                        emitter(AssistantEvent::ConnectionStatus(format!(
                            "正在连接服务器...（第{}次重试）",
                            connection_errors
                        )));
                        // 直连失败 → fallback 到代理重试
                        if !use_proxy && proxy_url.is_some() && is_connect_err {
                            tracing::info!(
                                "[PROXY-STREAM] Direct connection failed, falling back to proxy"
                            );
                            use_proxy = true;
                        }
                    }
                    continue;
                }
            };

            let status = response.status().as_u16();
            if status != 200 {
                let body_text = response.text().await.unwrap_or_default();
                if status == 529 || status == 503 || status == 502 {
                    last_error = format!(
                        "API error {}: {}",
                        status,
                        &body_text[..body_text.len().min(200)]
                    );
                    continue;
                }
                return Err(crate::NuphusError::LLM(crate::LLMError::ApiError {
                    status,
                    body: body_text[..body_text.len().min(500)].to_string(),
                }));
            }

            // Stream reading, parse SSE chunk by chunk
            let mut sse_buffer = String::new();
            let mut current_text = String::new();
            let mut current_reasoning = String::new();
            let mut tool_calls_map: std::collections::HashMap<usize, (String, String, String)> =
                std::collections::HashMap::new();
            let mut tool_id_order: Vec<usize> = Vec::new();
            let mut final_usage: Option<StreamEvent> = None;

            {
                use futures_util::StreamExt;
                let mut stream = response.bytes_stream();
                const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);

                loop {
                    if let Some(flag) = cancel_flag {
                        if flag.load(Ordering::SeqCst) {
                            return Err(crate::NuphusError::LLM(crate::LLMError::Cancelled));
                        }
                    }

                    let timed = tokio::time::timeout(CHUNK_TIMEOUT, stream.next()).await;
                    let chunk = match timed {
                        Ok(Some(Ok(b))) => b,
                        Ok(Some(Err(e))) => {
                            last_error = format!("Failed to read chunk: {}", e);
                            tracing::error!(error = %e, "LLM stream read error");
                            break;
                        }
                        Ok(None) => break,
                        Err(_) => {
                            last_error =
                                format!("Chunk read timeout after {}s", CHUNK_TIMEOUT.as_secs());
                            tracing::error!(
                                timeout_s = CHUNK_TIMEOUT.as_secs(),
                                "LLM stream chunk timeout"
                            );
                            break;
                        }
                    };

                    let chunk_str = String::from_utf8_lossy(&chunk);
                    sse_buffer.push_str(&chunk_str);

                    // Split by \n to process complete lines
                    let mut lines: Vec<String> = Vec::new();
                    while let Some(pos) = sse_buffer.find('\n') {
                        lines.push(sse_buffer[..pos].to_string());
                        sse_buffer = sse_buffer[pos + 1..].to_string();
                    }

                    for line in &lines {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }

                        let data = match line.strip_prefix("data:") {
                            Some(d) => d.trim(),
                            None => continue,
                        };

                        if data == "[DONE]" {
                            continue;
                        }

                        let json: serde_json::Value = match serde_json::from_str(data) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };

                        // Parse delta
                        if let Some(delta) = json.pointer("/choices/0/delta") {
                            // reasoning_content → emit Reasoning immediately (streaming, real-time)
                            if let Some(r) = delta.get("reasoning_content").and_then(|c| c.as_str())
                            {
                                if !r.is_empty() {
                                    current_reasoning.push_str(r);
                                    emitter(AssistantEvent::Reasoning(r.to_string()));
                                }
                            }

                            // content → emit TextDelta immediately (string form)
                            //        or parse array form for image_url parts
                            if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
                                if !text.is_empty() {
                                    current_text.push_str(text);
                                    emitter(AssistantEvent::TextDelta(text.to_string()));
                                }
                            } else if let Some(parts) =
                                delta.get("content").and_then(|c| c.as_array())
                            {
                                // Multimodal content array: e.g. [{"type":"text","text":"..."}, {"type":"image_url","image_url":{"url":"..."}}]
                                for part in parts {
                                    if let Some(t) = part.get("type").and_then(|v| v.as_str()) {
                                        match t {
                                            "text" => {
                                                if let Some(txt) =
                                                    part.get("text").and_then(|v| v.as_str())
                                                {
                                                    if !txt.is_empty() {
                                                        current_text.push_str(txt);
                                                        emitter(AssistantEvent::TextDelta(
                                                            txt.to_string(),
                                                        ));
                                                    }
                                                }
                                            }
                                            "image_url" => {
                                                if let Some(url) = part
                                                    .pointer("/image_url/url")
                                                    .and_then(|v| v.as_str())
                                                {
                                                    if !url.is_empty() {
                                                        tracing::info!(
                                                            "[IMAGE] model generated image: {}",
                                                            url
                                                        );
                                                        emitter(AssistantEvent::ImageAttachment {
                                                            url: url.to_string(),
                                                        });
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            } else if let Some(text) = delta.get("text").and_then(|c| c.as_str()) {
                                if !text.is_empty() {
                                    current_text.push_str(text);
                                    emitter(AssistantEvent::TextDelta(text.to_string()));
                                }
                            }

                            // tool_calls → accumulate
                            if let Some(tool_calls) =
                                delta.get("tool_calls").and_then(|t| t.as_array())
                            {
                                for tool_call in tool_calls {
                                    let index = tool_call
                                        .get("index")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0)
                                        as usize;
                                    let id =
                                        tool_call.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                    let name = tool_call
                                        .get("function")
                                        .and_then(|f| f.get("name"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let args = tool_call
                                        .get("function")
                                        .and_then(|f| f.get("arguments"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");

                                    let entry = tool_calls_map.entry(index).or_insert_with(|| {
                                        tool_id_order.push(index);
                                        (String::new(), String::new(), String::new())
                                    });
                                    if !id.is_empty() {
                                        entry.0 = id.to_string();
                                    }
                                    if !name.is_empty() {
                                        entry.1 = name.to_string();
                                    }
                                    entry.2.push_str(args);
                                }
                            }
                        }

                        // reasoning_content from /choices/0/message (end-of-stream).
                        // Only emit if deltas didn't already cover it — prevents duplicate emission.
                        if let Some(msg) = json.pointer("/choices/0/message") {
                            if let Some(r) = msg.get("reasoning_content").and_then(|c| c.as_str()) {
                                if current_reasoning.is_empty() {
                                    emitter(AssistantEvent::Reasoning(r.to_string()));
                                }
                                current_reasoning.push_str(r);
                            }
                        }

                        // usage
                        if let Some(usage) = json.get("usage") {
                            let prompt_tokens = usage
                                .get("prompt_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                as u32;
                            let cache_hit =
                                read_cache_hit(usage, self.config.quirks.cache_hit_field);
                            let output_tokens = usage
                                .get("completion_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                as u32;
                            final_usage = Some(StreamEvent::Usage {
                                input_tokens: prompt_tokens,
                                output_tokens,
                                cache_hit_tokens: cache_hit,
                            });
                        }
                    }
                }
            }

            if !last_error.is_empty() {
                // ── 断流 salvage：传输中途被切断时抢救已接收内容 ──
                // 背景：长流式响应易被中间链路掐断（实测 Retry×3 空转 25 分钟零产出）。
                // 文本已实时推给 UI、工具调用已累积——全部丢弃从零重试的代价
                // 远高于「部分内容 + 截断标记」，agent loop 下轮可基于现状继续。
                let salvageable_tools: Vec<(String, String, String)> = tool_id_order
                    .iter()
                    .filter_map(|idx| tool_calls_map.get(idx))
                    .filter(|t| {
                        // 工具调用必须完整才可 salvage：name 非空且 args 是合法 JSON
                        // （流可能在 arguments 字符串中间断掉，截断的 JSON 不可执行）
                        !t.1.is_empty()
                            && (t.2.is_empty()
                                || serde_json::from_str::<serde_json::Value>(&t.2).is_ok())
                    })
                    .cloned()
                    .collect();
                let dropped_tools = tool_id_order.len() - salvageable_tools.len();
                let text_len = current_text.chars().count();

                // 内容太少不值得抢救（重试成本低），否则立即 salvage 避免重复生成
                if !salvageable_tools.is_empty() || text_len >= 100 {
                    tracing::warn!(
                        "[STREAM] Salvaging partial response after transport error \
                         (text={} chars, tools={}, dropped_incomplete={}, err={})",
                        text_len,
                        salvageable_tools.len(),
                        dropped_tools,
                        last_error
                    );
                    for (id, name, args) in salvageable_tools {
                        let final_args = if args.is_empty() {
                            "{}".to_string()
                        } else {
                            args
                        };
                        emitter(AssistantEvent::ToolUse {
                            id,
                            name,
                            input: final_args,
                        });
                    }
                    emitter(AssistantEvent::TextDelta(
                        "\n\n[⚠ 响应因传输中断被截断，以上内容可能不完整]".to_string(),
                    ));
                    if let Some(StreamEvent::Usage {
                        input_tokens,
                        output_tokens,
                        cache_hit_tokens,
                    }) = final_usage.take()
                    {
                        emitter(AssistantEvent::Usage {
                            input_tokens,
                            output_tokens,
                            cache_hit_tokens,
                        });
                    }
                    emitter(AssistantEvent::MessageStop);
                    return Ok(());
                }
                continue;
            }

            // Reasoning already emitted in real-time via delta path above.
            // current_reasoning is kept only for message-path fallback detection.

            for &idx in &tool_id_order {
                if let Some((ref id, ref name, ref args)) = tool_calls_map.get(&idx) {
                    if !name.is_empty() {
                        let final_args = if args.is_empty() {
                            "{}".to_string()
                        } else {
                            args.clone()
                        };
                        emitter(AssistantEvent::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: final_args,
                        });
                    }
                }
            }

            if let Some(StreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_hit_tokens,
            }) = final_usage.take()
            {
                emitter(AssistantEvent::Usage {
                    input_tokens,
                    output_tokens,
                    cache_hit_tokens,
                });
            }

            emitter(AssistantEvent::MessageStop);
            return Ok(());
        }

        Err(crate::NuphusError::LLM(
            crate::LLMError::RetryLoopExhausted { last_error },
        ))
    }

    pub(crate) fn build_request_body(
        &self,
        request: &crate::api::MessageRequest,
    ) -> serde_json::Value {
        let model = if request.model.is_empty() {
            self.config.model.clone()
        } else {
            request.model.clone()
        };
        let mut messages = Vec::new();
        // merged system prompt (L0+L2+L1 as single message) — built once per session
        if let Some(ref merged) = request.merged_system {
            messages.push(serde_json::json!({
                "role": "system",
                "content": merged,
            }));
        } else {
            // fallback: legacy separate system + system_messages
            if let Some(system_content) = &request.system {
                messages.push(serde_json::json!({
                    "role": "system",
                    "content": system_content,
                }));
            }
            for sys_msg in &request.system_messages {
                messages.push(serde_json::json!({
                    "role": "system",
                    "content": sys_msg,
                }));
            }
        }
        // history conversation + current input
        messages.extend(request.messages.clone());
        // ── reasoning_content consistency fix ──
        // DeepSeek / Kimi / MiMo thinking mode requires reasoning_content on
        // EVERY assistant message. If ANY assistant message omits it, the API
        // rejects with 400 "thinking is enabled but reasoning_content is missing".
        //
        // Strategy:
        // ── reasoning_content consistency fix ──
        // Driven by Provider quirks embedded in ChatCompletionsConfig at
        // construction time. No string matching needed.
        let needs_thinking_pad = self.config.quirks.requires_reasoning_echo;
        let mut padded_count = 0u32;
        if needs_thinking_pad {
            for msg in &mut messages {
                if msg.get("role").and_then(|r| r.as_str()) == Some("assistant")
                    && msg.get("reasoning_content").is_none()
                {
                    if let Some(obj) = msg.as_object_mut() {
                        obj.insert("reasoning_content".to_string(), serde_json::json!(" "));
                        padded_count += 1;
                    }
                }
            }
            if padded_count > 0 {
                tracing::info!(
                    "[REQ] Padded {} assistant messages with reasoning_content placeholder for thinking-mode provider",
                    padded_count
                );
            }
        }

        let mut body = if let Some(max_tokens) = request.max_tokens {
            serde_json::json!({
                "model": model,
                "messages": messages,
                "max_tokens": max_tokens,
                "stream": request.stream,
            })
        } else {
            serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": request.stream,
            })
        };
        // Sampling temperature — only when explicitly set (provider default otherwise)
        if let Some(temperature) = request.temperature {
            body["temperature"] = serde_json::json!(temperature);
        }
        // Include usage info in streaming requests (DeepSeek / OpenAI support)
        if request.stream {
            body["stream_options"] = serde_json::json!({"include_usage": true});
        }
        // Include tools — sanitization driven by Provider quirks.
        // Tool names must match ^[a-zA-Z0-9_-]+$ (OpenAI specification),
        // so always apply lightweight name sanitize (:: → _) across all models.
        // Providers that need JSON Schema subset cleanup set `sanitize_tools`
        // in their quirks (e.g. Kimi/Moonshot).
        if let Some(tools) = &request.tools {
            let tools_for_body = if let Some(sanitize) = self.config.quirks.sanitize_tools {
                sanitize(tools)
            } else {
                // Lightweight name sanitize to prevent :: chars causing 400
                tools
                    .iter()
                    .map(|t| {
                        let sanitized_name = sanitize_tool_name(&t.function.name);
                        if sanitized_name != t.function.name {
                            let mut new_tool = t.clone();
                            new_tool.function.name = sanitized_name;
                            new_tool
                        } else {
                            t.clone()
                        }
                    })
                    .collect()
            };
            body["tools"] = serde_json::json!(tools_for_body);
        }

        // Explicitly set thinking mode for providers that support it.
        // Skip when tools are present — thinking mode interferes with function calling.
        if needs_thinking_pad && request.tools.is_none() {
            body["thinking"] = serde_json::json!({"type": "enabled"});
        }

        // Reasoning depth — effort control for providers that accept it.
        // Gated by quirks.supports_reasoning_effort so the other Providers never
        // receive an unknown field; the value comes from `config.toml
        // [[providers]] reasoning_effort`. Providers with effort_excludes_tools
        // (DeepSeek) additionally suppress the field on tool-carrying requests;
        // others (Kimi k3 — verified live) receive it with or without tools.
        if self.config.quirks.supports_reasoning_effort
            && (request.tools.is_none() || !self.config.quirks.effort_excludes_tools)
        {
            if let Some(effort) = &self.config.reasoning_effort {
                body["reasoning_effort"] = serde_json::json!(effort);
            }
        }

        body
    }
}

#[async_trait]
impl super::super::Transport for ChatCompletionsTransport {
    async fn stream(&self, request: crate::api::MessageRequest) -> Result<Vec<StreamEvent>> {
        let is_streaming = request.stream;
        let body = self.build_request_body(&request);
        tracing::info!(
            "[REQ] model={}, {} messages, stream={}",
            self.config.model,
            body["messages"].as_array().map(|a| a.len()).unwrap_or(0),
            is_streaming
        );
        static ALWAYS_FALSE: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        let body_text = self.send_chat_request(body, Some(&ALWAYS_FALSE)).await?;
        if is_streaming {
            Self::parse_sse(&body_text, self.config.quirks.cache_hit_field)
        } else {
            Self::parse_non_streaming(&body_text, self.config.quirks.cache_hit_field)
        }
    }

    async fn stream_with_cancellation(
        &self,
        request: crate::api::MessageRequest,
        cancel_flag: &AtomicBool,
    ) -> Result<Vec<StreamEvent>> {
        let is_streaming = request.stream;
        let body = self.build_request_body(&request);
        tracing::info!(
            "[REQ] model={}, {} messages, stream={}",
            self.config.model,
            body["messages"].as_array().map(|a| a.len()).unwrap_or(0),
            is_streaming
        );
        let body_text = self.send_chat_request(body, Some(cancel_flag)).await?;
        if is_streaming {
            Self::parse_sse_with_cancellation(
                &body_text,
                cancel_flag,
                self.config.quirks.cache_hit_field,
            )
        } else {
            Self::parse_non_streaming(&body_text, self.config.quirks.cache_hit_field)
        }
    }

    async fn stream_with_emitter(
        &self,
        request: crate::api::MessageRequest,
        cancel_flag: &AtomicBool,
        emitter: Box<dyn Fn(AssistantEvent) + Send>,
    ) -> Result<()> {
        let body = self.build_request_body(&request);
        tracing::info!(
            "[REQ-STREAMING] model={}, {} messages",
            self.config.model,
            body["messages"].as_array().map(|a| a.len()).unwrap_or(0)
        );
        self.send_chat_request_streaming(body, Some(cancel_flag), emitter)
            .await
    }

    fn provider_name(&self) -> &str {
        &self.config.name
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn provider_kind(&self) -> Option<crate::api::ProviderKind> {
        self.config.provider_kind
    }
}

impl ChatCompletionsTransport {
    /// Parse SSE-formatted chat.completions response
    pub(crate) fn parse_sse(body: &str, cache_hit_field: &str) -> Result<Vec<StreamEvent>> {
        let mut events = Vec::new();
        let mut current_text = String::new();
        let mut current_reasoning = String::new();
        let mut image_urls: Vec<String> = Vec::new();
        // Accumulate tool call parameters by index, avoid premature emission of intermediate state
        let mut tool_calls_map: std::collections::HashMap<usize, (String, String, String)> =
            std::collections::HashMap::new();
        let mut tool_id_order: Vec<usize> = Vec::new();

        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let data = match line.strip_prefix("data:") {
                Some(d) => d.trim(),
                None => continue,
            };

            if data == "[DONE]" {
                continue;
            }

            let json: serde_json::Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Parse choices[0].delta
            if let Some(delta) = json.pointer("/choices/0/delta") {
                // Reasoning/thinking content (DeepSeek thinking mode, needs to be sent back in next round)
                if let Some(r) = delta.get("reasoning_content").and_then(|c| c.as_str()) {
                    current_reasoning.push_str(r);
                }

                // Text delta (string form or array form)
                if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
                    current_text.push_str(text);
                } else if let Some(parts) = delta.get("content").and_then(|c| c.as_array()) {
                    for part in parts {
                        match part.get("type").and_then(|v| v.as_str()) {
                            Some("text") => {
                                if let Some(txt) = part.get("text").and_then(|v| v.as_str()) {
                                    current_text.push_str(txt);
                                }
                            }
                            Some("image_url") => {
                                if let Some(url) =
                                    part.pointer("/image_url/url").and_then(|v| v.as_str())
                                {
                                    image_urls.push(url.to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                } else if let Some(text) = delta.get("text").and_then(|c| c.as_str()) {
                    current_text.push_str(text);
                }

                // Tool calls — accumulate independently by index, emit only once at end of stream
                if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                    if !tool_calls.is_empty() {
                        tracing::debug!(
                            "[SSE] tool_calls in delta: {}",
                            serde_json::to_string(tool_calls).unwrap_or_default()
                        );
                    }
                    for tool_call in tool_calls {
                        let index =
                            tool_call.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let id = tool_call.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let name = tool_call
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let args = tool_call
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let entry = tool_calls_map.entry(index).or_insert_with(|| {
                            tool_id_order.push(index);
                            (String::new(), String::new(), String::new())
                        });
                        if !id.is_empty() {
                            entry.0 = id.to_string();
                        }
                        if !name.is_empty() {
                            entry.1 = name.to_string();
                        }
                        entry.2.push_str(args);
                    }
                }
            }

            // Parse usage (also pick up reasoning_content from /choices/0/message for some APIs)
            if let Some(msg) = json.pointer("/choices/0/message") {
                if let Some(r) = msg.get("reasoning_content").and_then(|c| c.as_str()) {
                    current_reasoning.push_str(r);
                }
            }
            if let Some(usage) = json.get("usage") {
                let prompt_tokens = usage
                    .get("prompt_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                let cache_hit = read_cache_hit(usage, cache_hit_field);
                let output_tokens = usage
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                events.push(StreamEvent::Usage {
                    input_tokens: prompt_tokens,
                    output_tokens,
                    cache_hit_tokens: cache_hit,
                });
            }
        }

        // Emit reasoning content (DeepSeek thinking mode)
        if !current_reasoning.is_empty() {
            let preview: String = current_reasoning.chars().take(80).collect();
            tracing::info!(
                "[SSE] Reasoning captured ({} chars): {}...",
                current_reasoning.len(),
                preview
            );
            events.push(StreamEvent::Reasoning(std::mem::take(
                &mut current_reasoning,
            )));
        } else {
            tracing::info!("[SSE] No reasoning_content in response");
        }

        // Emit remaining text
        if !current_text.is_empty() {
            let text = current_text.trim().to_string();
            if !text.is_empty() {
                tracing::debug!(
                    "[SSE] TextDelta ({} chars): {}...",
                    text.len(),
                    text.chars().take(80).collect::<String>()
                );
                events.push(StreamEvent::TextDelta(text));
            }
        }

        // Emit all tool calls (parameters fully accumulated, no intermediate state)
        for &idx in &tool_id_order {
            if let Some((ref id, ref name, ref args)) = tool_calls_map.get(&idx) {
                if !name.is_empty() {
                    let final_args = if args.is_empty() {
                        "{}".to_string()
                    } else {
                        args.clone()
                    };
                    tracing::debug!(
                        "[SSE] ToolUse: name={}, args_len={}, args_preview={}",
                        name,
                        final_args.len(),
                        final_args.chars().take(100).collect::<String>()
                    );
                    events.push(StreamEvent::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: final_args,
                    });
                }
            }
        }

        // Emit image URLs collected from multimodal content
        for url in image_urls {
            tracing::info!("[SSE] ImageUrl: {}", url);
            events.push(StreamEvent::ImageUrl(url));
        }

        events.push(StreamEvent::Done);
        let tool_use_count = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::ToolUse { .. }))
            .count();
        tracing::debug!(
            "[SSE] parse_sse done: {} total events, {} ToolUse events",
            events.len(),
            tool_use_count
        );
        Ok(events)
    }

    /// Parse SSE-formatted chat.completions response (with cancellation support)
    pub(crate) fn parse_sse_with_cancellation(
        body: &str,
        cancel_flag: &AtomicBool,
        cache_hit_field: &str,
    ) -> Result<Vec<StreamEvent>> {
        let mut events = Vec::new();
        let mut current_text = String::new();
        let mut current_reasoning = String::new();
        // Accumulate tool call parameters by index, avoid premature emission of intermediate state
        let mut tool_calls_map: std::collections::HashMap<usize, (String, String, String)> =
            std::collections::HashMap::new();
        let mut tool_id_order: Vec<usize> = Vec::new();

        for line in body.lines() {
            // Periodically check cancel flag
            if cancel_flag.load(Ordering::SeqCst) {
                return Err(crate::NuphusError::LLM(crate::LLMError::Cancelled));
            }

            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let data = match line.strip_prefix("data:") {
                Some(d) => d.trim(),
                None => continue,
            };

            if data == "[DONE]" {
                continue;
            }

            let json: serde_json::Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Parse choices[0].delta
            if let Some(delta) = json.pointer("/choices/0/delta") {
                // Reasoning/thinking content (DeepSeek thinking mode, needs to be sent back in next round)
                if let Some(r) = delta.get("reasoning_content").and_then(|c| c.as_str()) {
                    current_reasoning.push_str(r);
                }

                // Text delta
                if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
                    current_text.push_str(text);
                } else if let Some(text) = delta.get("text").and_then(|c| c.as_str()) {
                    current_text.push_str(text);
                }

                // Tool calls — accumulate independently by index, emit only once at end of stream
                if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                    if !tool_calls.is_empty() {
                        tracing::debug!(
                            "[SSE] tool_calls in delta: {}",
                            serde_json::to_string(tool_calls).unwrap_or_default()
                        );
                    }
                    for tool_call in tool_calls {
                        let index =
                            tool_call.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let id = tool_call.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let name = tool_call
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let args = tool_call
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let entry = tool_calls_map.entry(index).or_insert_with(|| {
                            tool_id_order.push(index);
                            (String::new(), String::new(), String::new())
                        });
                        if !id.is_empty() {
                            entry.0 = id.to_string();
                        }
                        if !name.is_empty() {
                            entry.1 = name.to_string();
                        }
                        entry.2.push_str(args);
                    }
                }
            }

            // Parse usage (also pick up reasoning_content from /choices/0/message for some APIs)
            if let Some(msg) = json.pointer("/choices/0/message") {
                if let Some(r) = msg.get("reasoning_content").and_then(|c| c.as_str()) {
                    current_reasoning.push_str(r);
                }
            }
            if let Some(usage) = json.get("usage") {
                let prompt_tokens = usage
                    .get("prompt_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                let cache_hit = read_cache_hit(usage, cache_hit_field);
                let output_tokens = usage
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                events.push(StreamEvent::Usage {
                    input_tokens: prompt_tokens,
                    output_tokens,
                    cache_hit_tokens: cache_hit,
                });
            }
        }

        // Emit reasoning content (DeepSeek thinking mode)
        if !current_reasoning.is_empty() {
            let preview: String = current_reasoning.chars().take(80).collect();
            tracing::info!(
                "[SSE-Cancel] Reasoning captured ({} chars): {}...",
                current_reasoning.len(),
                preview
            );
            events.push(StreamEvent::Reasoning(std::mem::take(
                &mut current_reasoning,
            )));
        } else {
            tracing::info!("[SSE-Cancel] No reasoning_content in response");
        }

        // Emit remaining text
        if !current_text.is_empty() {
            let text = current_text.trim().to_string();
            if !text.is_empty() {
                tracing::debug!(
                    "[SSE] TextDelta ({} chars): {}...",
                    text.len(),
                    text.chars().take(80).collect::<String>()
                );
                events.push(StreamEvent::TextDelta(text));
            }
        }

        // Emit all tool calls (parameters fully accumulated, no intermediate state)
        for &idx in &tool_id_order {
            if let Some((ref id, ref name, ref args)) = tool_calls_map.get(&idx) {
                if !name.is_empty() {
                    let final_args = if args.is_empty() {
                        "{}".to_string()
                    } else {
                        args.clone()
                    };
                    tracing::debug!(
                        "[SSE] ToolUse: name={}, args_len={}, args_preview={}",
                        name,
                        final_args.len(),
                        final_args.chars().take(100).collect::<String>()
                    );
                    events.push(StreamEvent::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: final_args,
                    });
                }
            }
        }

        events.push(StreamEvent::Done);
        let tool_use_count = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::ToolUse { .. }))
            .count();
        tracing::debug!(
            "[SSE] parse_sse done: {} total events, {} ToolUse events",
            events.len(),
            tool_use_count
        );
        Ok(events)
    }

    /// Parse non-streaming JSON response (stream=false)
    ///
    /// Non-streaming response format:
    /// ```json
    /// {
    ///   "choices": [{
    ///     "message": {
    ///       "content": "...",
    ///       "reasoning_content": "...",
    ///       "tool_calls": [{"id": "...", "type": "function", "function": {"name": "...", "arguments": "..."}}]
    ///     }
    ///   }],
    ///   "usage": {"prompt_tokens": 0, "completion_tokens": 0}
    /// }
    /// ```
    pub(crate) fn parse_non_streaming(
        body: &str,
        cache_hit_field: &str,
    ) -> Result<Vec<StreamEvent>> {
        let mut events = Vec::new();
        let json: serde_json::Value = serde_json::from_str(body).map_err(|e| {
            crate::NuphusError::LLM(crate::LLMError::Other(format!(
                "Non-streaming response JSON parse failed: {}",
                e
            )))
        })?;

        // Check for API error
        if let Some(err) = json.get("error") {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(crate::NuphusError::LLM(crate::LLMError::ApiError {
                status: 0,
                body: msg.to_string(),
            }));
        }

        // Extract content from choices[0].message
        if let Some(msg) = json.pointer("/choices/0/message") {
            // Text content
            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                if !content.is_empty() {
                    events.push(StreamEvent::TextDelta(content.to_string()));
                }
            }

            // Reasoning/thinking content (DeepSeek thinking mode)
            if let Some(reasoning) = msg.get("reasoning_content").and_then(|c| c.as_str()) {
                if !reasoning.is_empty() {
                    let preview: String = reasoning.chars().take(80).collect();
                    tracing::info!(
                        "[NonStream] Reasoning captured ({} chars): {}...",
                        reasoning.len(),
                        preview
                    );
                    events.push(StreamEvent::Reasoning(reasoning.to_string()));
                }
            }

            // Tool calls (full tool_calls, no fragmentation issues)
            if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    let id = tc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("call_unknown");
                    let name = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let args = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}");
                    events.push(StreamEvent::ToolUse {
                        id: id.to_string(),
                        name: name.to_string(),
                        arguments: args.to_string(),
                    });
                }
            }
        }

        // usage
        if let Some(usage) = json.get("usage") {
            let prompt_tokens = usage
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let cache_hit = read_cache_hit(usage, cache_hit_field);
            let output_tokens = usage
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            events.push(StreamEvent::Usage {
                input_tokens: prompt_tokens,
                output_tokens,
                cache_hit_tokens: cache_hit,
            });
        }

        events.push(StreamEvent::Done);
        Ok(events)
    }
}

#[cfg(test)]
mod salvage_tests {
    //! 断流 salvage 测试：用真实 TCP 服务器模拟「SSE 流写到一半连接被掐断」
    //! （Content-Length 虚报后提前关连接 → reqwest 报 error decoding response body，
    //! 与生产日志中的错误路径一致）。
    use super::*;
    use crate::api::AssistantEvent;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn test_config(port: u16) -> ChatCompletionsConfig {
        ChatCompletionsConfig {
            name: "test".into(),
            api_key: "sk-test".into(),
            base_url: format!("http://127.0.0.1:{}", port),
            model: "test-model".into(),
            timeout_secs: 30,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: None,
            quirks: crate::config::provider::ProviderQuirks::default(),
            reasoning_effort: None,
        }
    }

    /// 启动一次性假服务器：接收请求后写出 SSE 半截流并立即断开。
    /// 返回 (port, 连接计数器)。
    async fn spawn_truncating_server(sse_payload: String) -> (u16, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits2 = hits.clone();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                hits2.fetch_add(1, AtomicOrdering::SeqCst);
                let payload = sse_payload.clone();
                tokio::spawn(async move {
                    // 读完请求头（不等 body，够真实即可）
                    let mut buf = [0u8; 8192];
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        sock.read(&mut buf),
                    )
                    .await;
                    // 虚报 Content-Length：写到一半直接断连，模拟传输被掐
                    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 999999\r\n\r\n";
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(payload.as_bytes()).await;
                    let _ = sock.flush().await;
                    // drop(sock) → 连接提前关闭，body 不完整
                });
                // 只服务一次（salvage 路径不应触发重试）
            }
        });
        (port, hits)
    }

    fn collect_events() -> (
        Arc<Mutex<Vec<AssistantEvent>>>,
        Box<dyn Fn(AssistantEvent) + Send>,
    ) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let events2 = events.clone();
        let emitter: Box<dyn Fn(AssistantEvent) + Send> =
            Box::new(move |e| events2.lock().unwrap().push(e));
        (events, emitter)
    }

    fn request_body() -> serde_json::Value {
        serde_json::json!({
            "model": "test-model",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}]
        })
    }

    /// 长文本（≥100 字符）半截流 → 应 salvage：Ok + 原文保留 + 截断标记 + MessageStop，不重试
    #[tokio::test]
    async fn test_salvage_partial_text_on_transport_break() {
        let long_text = "这是一段足够长的流式输出文本，用来模拟 ExecAgent 长时间生成后连接被中间链路掐断的场景。这是一段足够长的流式输出文本，用来模拟 ExecAgent 长时间生成后连接被中间链路掐断的场景。补足一百字符。";
        assert!(long_text.chars().count() >= 100);
        let payload = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
            long_text
        );
        let (port, hits) = spawn_truncating_server(payload).await;
        let transport = ChatCompletionsTransport::new(test_config(port));
        let (events, emitter) = collect_events();

        let result = transport
            .send_chat_request_streaming(request_body(), None, emitter)
            .await;

        assert!(
            result.is_ok(),
            "salvage 应返回 Ok，实际: {:?}",
            result.err()
        );
        let events = events.lock().unwrap();
        let full_text: String = events
            .iter()
            .filter_map(|e| match e {
                AssistantEvent::TextDelta(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            full_text.contains(long_text),
            "已收文本应保留: {}",
            full_text
        );
        assert!(
            full_text.contains("传输中断被截断"),
            "应追加截断标记: {}",
            full_text
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AssistantEvent::MessageStop)),
            "应以 MessageStop 正常收尾"
        );
        assert_eq!(
            hits.load(AtomicOrdering::SeqCst),
            1,
            "salvage 不应触发重试（重复生成）"
        );
    }

    /// 完整工具调用 salvage + 截断 JSON 的不完整调用被丢弃
    #[tokio::test]
    async fn test_salvage_complete_tool_call_and_drop_incomplete() {
        // index 0：完整工具调用（args 为合法 JSON，一次性到达）
        // index 1：arguments 字符串写到一半断流（JSON 不完整，不可执行）
        let mut payload = String::new();
        payload.push_str(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"web_search","arguments":"{\"query\":\"rust async\"}"}}]}}]}"#);
        payload.push_str("\n\n");
        payload.push_str(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_2","function":{"name":"web_search","arguments":"{\"query\":\"abc"}}]}}]}"#);
        payload.push_str("\n\n");
        let (port, _) = spawn_truncating_server(payload).await;
        let transport = ChatCompletionsTransport::new(test_config(port));
        let (events, emitter) = collect_events();

        let result = transport
            .send_chat_request_streaming(request_body(), None, emitter)
            .await;

        assert!(
            result.is_ok(),
            "salvage 应返回 Ok，实际: {:?}",
            result.err()
        );
        let events = events.lock().unwrap();
        let tool_uses: Vec<&AssistantEvent> = events
            .iter()
            .filter(|e| matches!(e, AssistantEvent::ToolUse { .. }))
            .collect();
        assert_eq!(tool_uses.len(), 1, "只有完整的工具调用可被 salvage");
        match tool_uses[0] {
            AssistantEvent::ToolUse { id, name, input } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "web_search");
                let parsed: serde_json::Value = serde_json::from_str(input).unwrap();
                assert_eq!(parsed["query"], "rust async");
            }
            _ => unreachable!(),
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AssistantEvent::MessageStop)),
            "应以 MessageStop 正常收尾"
        );
    }
}
