//! Session API 格式转换与工具对清理
//!
//! 包含 Session 与 LLM API 之间的消息格式转换（to_api_messages）、
//! 不完整工具调用的清理（strip_incomplete_tools）、
//! 提炼标记插入（insert_refine_marker/insert_divide_anchor）、
//! Token 用量估算等功能。

use crate::session::session::Session;
use crate::session::types::*;

impl Session {
    /// 清理不完整的工具调用对，确保 API 格式合法
    ///
    /// DeepSeek 等严格 API 要求每个 assistant(tool_calls) 消息
    /// 必须紧接一条带有对应 tool_call_id 的 tool 消息。
    /// 提炼或异常中断可能打断配对；此方法修复这些情况。
    pub fn strip_incomplete_tools(&mut self) {
        use std::collections::HashSet;

        // 收集所有 ToolResult 引用的 tool_call_id
        let mut result_ids: HashSet<String> = HashSet::new();
        for msg in &self.messages {
            for block in &msg.content {
                if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                    result_ids.insert(tool_use_id.clone());
                }
            }
        }

        // 移除没有对应 ToolResult 的 ToolUse block
        for msg in &mut self.messages {
            if msg.role == MessageRole::Assistant {
                msg.content.retain(|b| match b {
                    ContentBlock::ToolUse { id, .. } => result_ids.contains(id.as_str()),
                    _ => true,
                });
            }
        }

        // 收集所有 ToolUse id（清理后）
        let mut use_ids: HashSet<String> = HashSet::new();
        for msg in &self.messages {
            for block in &msg.content {
                if let ContentBlock::ToolUse { id, .. } = block {
                    use_ids.insert(id.clone());
                }
            }
        }

        // 移除没有对应 ToolUse 的 ToolResult block
        for msg in &mut self.messages {
            msg.content.retain(|b| match b {
                ContentBlock::ToolResult { tool_use_id, .. } => {
                    use_ids.contains(tool_use_id.as_str())
                }
                _ => true,
            });
        }

        // 移除变空的消息
        self.messages.retain(|msg| !msg.content.is_empty());
    }

    /// 转换为 API 格式消息列表（完全扁平结构）
    pub fn to_api_messages(&self, supports_vision: bool) -> Vec<serde_json::Value> {
        // 防御性：收集所有有对应 ToolResult 的 tool_call_id
        // 防止 strip_incomplete_tools 遗漏或 session 被异常修改
        let valid_result_ids: std::collections::HashSet<String> = self
            .messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.clone()),
                _ => None,
            })
            .collect();

        self.messages
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                    MessageRole::System => "system",
                };

                // 分离文本块和工具调用块
                let texts: Vec<&str> = msg.content.iter()
                    .filter_map(|b| match b { ContentBlock::Text { text, .. } => Some(text.as_str()), _ => None })
                    .collect();
                let reasoning: Option<&str> = msg.content.iter()
                    .find_map(|b| match b {
                        ContentBlock::Text { reasoning, .. } => reasoning.as_deref(),
                        _ => None,
                    });
                let tool_uses: Vec<&ContentBlock> = msg.content.iter()
                    .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
                    .collect();
                let tool_results: Vec<&ContentBlock> = msg.content.iter()
                    .filter(|b| matches!(b, ContentBlock::ToolResult { .. }))
                    .collect();

                // Tool result 消息 → 扁平：{"role":"tool","tool_call_id":"...","content":"..."}
                if let Some(ContentBlock::ToolResult { tool_use_id, content, .. }) = tool_results.first() {
                    return serde_json::json!({
                        "role": role,
                        "tool_call_id": tool_use_id,
                        "content": content,
                    });
                }

                // Assistant with tool calls → 只保留有对应 ToolResult 的 tool_calls
                let valid_tool_uses: Vec<&ContentBlock> = tool_uses.iter()
                    .filter(|b| match b {
                        ContentBlock::ToolUse { id, .. } => valid_result_ids.contains(id.as_str()),
                        _ => false,
                    })
                    .copied()
                    .collect();

                if !valid_tool_uses.is_empty() {
                    let tool_calls: Vec<serde_json::Value> = valid_tool_uses.iter().map(|b| {
                        if let ContentBlock::ToolUse { id, name, input } = b {
                            let normalized_name = name.replace("::", "_");
                            serde_json::json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": normalized_name,
                                    "arguments": serde_json::to_string(input).unwrap_or_default(),
                                }
                            })
                        } else {
                            serde_json::json!({})
                        }
                    }).collect();
                    let content_text: String = texts.iter()
                        .map(|s| s.trim()).filter(|s| !s.is_empty())
                        .collect::<Vec<_>>().join("\n");
                    let mut msg_val = serde_json::json!({
                        "role": role,
                    });
                    // reasoning_content BEFORE content — field order signals to the model
                    // that thinking precedes text output
                    if role == "assistant" {
                        if let Some(r) = reasoning.filter(|s| !s.is_empty()) {
                            msg_val["reasoning_content"] = serde_json::json!(r);
                        }
                    }
                    msg_val["content"] = serde_json::json!(if content_text.is_empty() { serde_json::Value::Null } else { serde_json::json!(content_text) });
                    msg_val["tool_calls"] = serde_json::json!(tool_calls);
                    return msg_val;
                }

                // 检查是否有 Image / Audio block
                let has_media = msg.content.iter().any(|b| {
                    matches!(b, ContentBlock::Image { .. } | ContentBlock::Audio { .. })
                });

                if has_media {
                    let mut content_array = Vec::new();
                    for text in &texts {
                        if !text.is_empty() {
                            content_array.push(serde_json::json!({
                                "type": "text",
                                "text": text,
                            }));
                        }
                    }
                    for block in &msg.content {
                        if let ContentBlock::Image { url } = block {
                            if supports_vision {
                                // 保持现有逻辑：BMP→PNG 转换 + image_url
                                let final_url = if url.starts_with("data:image/bmp") {
                                    crate::utils::convert_bmp_data_url_to_png(url)
                                        .unwrap_or_else(|e| {
                                            tracing::warn!("[transform] BMP→PNG conversion failed: {e}. Sending original.");
                                            url.clone()
                                        })
                                } else {
                                    url.clone()
                                };
                                content_array.push(serde_json::json!({
                                    "type": "image_url",
                                    "image_url": { "url": final_url },
                                }));
                            } else {
                                // 主模型不支持视觉：图片统一转码为 PNG 保存临时文件，注入路径。
                                // Agent 收到路径后按需调用 desktop_vision(image_path=<路径>, prompt=<精准问题>)
                                // 定向查看图片内容——不自动调用视觉模型（泛化描述浪费且不精准）。
                                let saved_path = save_base64_to_temp_png(url);
                                match saved_path {
                                    Ok(path) => {
                                        let path_str = path.to_string_lossy().to_string();
                                        content_array.push(serde_json::json!({
                                            "type": "text",
                                            "text": format!("[📷 用户附带图片，已保存至: {}]", path_str),
                                        }));
                                        tracing::info!("[transform] Image saved to temp file: {}", path_str);
                                    }
                                    Err(e) => {
                                        tracing::warn!("[transform] Failed to save image to temp: {e}");
                                        content_array.push(serde_json::json!({
                                            "type": "text",
                                            "text": "[📷 用户附带图片，但保存失败]",
                                        }));
                                    }
                                }
                            }
                        } else if let ContentBlock::Audio { url } = block {
                            // Audio 附件——暂不支持直接传给 API，保存为临时文件 + 文本占位符
                            // 如需支持 audio 输入（如 OpenAI GPT-4o-audio），在此添加 content type 分支
                            let saved_path = save_base64_to_temp_raw(url, "audio");
                            match saved_path {
                                Ok(path) => {
                                    let path_str = path.to_string_lossy().to_string();
                                    content_array.push(serde_json::json!({
                                        "type": "text",
                                        "text": format!("[🎤 用户附带音频，已保存至: {}]", path_str),
                                    }));
                                    tracing::info!("[transform] Audio saved to temp file: {}", path_str);
                                }
                                Err(e) => {
                                    tracing::warn!("[transform] Failed to save audio to temp: {e}");
                                    content_array.push(serde_json::json!({
                                        "type": "text",
                                        "text": "[🎤 用户附带音频，但保存失败]",
                                    }));
                                }
                            }
                        }
                    }
                    let mut msg_val = serde_json::json!({
                        "role": role,
                    });
                    // reasoning_content BEFORE content — field order signals to the model
                    // that thinking precedes text output
                    if role == "assistant" {
                        if let Some(r) = reasoning.filter(|s| !s.is_empty()) {
                            msg_val["reasoning_content"] = serde_json::json!(r);
                        }
                    }
                    msg_val["content"] = serde_json::json!(content_array);
                    msg_val
                } else {
                    let mut msg_val = serde_json::json!({
                        "role": role,
                    });
                    // reasoning_content BEFORE content — field order signals to the model
                    // that thinking precedes text output
                    if role == "assistant" {
                        if let Some(r) = reasoning.filter(|s| !s.is_empty()) {
                            msg_val["reasoning_content"] = serde_json::json!(r);
                        }
                    }
                    msg_val["content"] = serde_json::json!(texts.join("\n"));
                    msg_val
                }
            })
            .collect()
    }

    /// 估算当前消息列表 token 用量
    /// 优先使用 API 返回的实际 input_tokens，退化到字符估算
    pub fn estimate_token_usage(&self) -> usize {
        let json_str = serde_json::to_string(&self.messages).unwrap_or_default();
        let char_estimate = (json_str.len() / 4) + 100;
        std::cmp::max(self.api_input_tokens as usize, char_estimate)
    }

    /// 更新 API 返回的 input_tokens
    pub fn update_api_input_tokens(&mut self, count: u64) {
        self.api_input_tokens = count;
    }

    /// 插入提炼/分裂标记消息到会话中
    ///
    /// - `summary` — 提炼摘要文本或标记文本
    /// - `child_id` — 分裂时子会话 ID（触发详情格式）
    /// - `include_detail` — 是否使用详情格式（分隔线+摘要+子会话ID）
    ///
    /// 当 `include_detail=false` 时，`summary` 原样插入。
    pub fn insert_refine_marker(
        &mut self,
        summary: &str,
        child_id: Option<&str>,
        include_detail: bool,
    ) {
        let marker = if include_detail {
            format!(
                "━━━ 会话提炼 ━━━\n📌 摘要：{}\n子会话 ID：{}\n━━━━━━━━━━━━━━━",
                summary.lines().next().unwrap_or(summary),
                child_id.unwrap_or(""),
            )
        } else {
            summary.to_string()
        };
        self.messages.push(Message {
            role: MessageRole::System,
            content: vec![ContentBlock::Text {
                text: marker,
                reasoning: None,
            }],
            internal: true,
            timestamp: Some(crate::session::types::now_ms()),
        });
    }

    /// 插入分裂锚点（refine/divide 时标记旧会话的结束位置）
    /// 委托给 insert_refine_marker 实现
    pub fn insert_divide_anchor(&mut self, summary: &str, child_id: &str) {
        self.insert_refine_marker(summary, Some(child_id), true);
    }
}

/// 将 base64 data URL 解码原样保存为临时文件（用于非图片附件，如音频），返回路径
///
/// 不做格式转换（音频字节不能经 image crate 解码）。文件名基于内容 hash，
/// 保证相同附件 → 相同路径；首次写入，后续命中已存在文件直接返回。
fn save_base64_to_temp_raw(data_url: &str, kind: &str) -> Result<std::path::PathBuf, String> {
    let base64_data = if let Some(comma_pos) = data_url.find(',') {
        &data_url[comma_pos + 1..]
    } else {
        return Err("Invalid data URL: no comma found".to_string());
    };

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| format!("Base64 decode failed: {e}"))?;

    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let hash = hasher.finish();

    let temp_dir = std::env::temp_dir();
    let filename = format!("nuphus_{}_{:016x}.bin", kind, hash);
    let filepath = temp_dir.join(filename);

    if filepath.exists() {
        return Ok(filepath);
    }

    std::fs::write(&filepath, &bytes).map_err(|e| format!("Failed to write temp file: {e}"))?;

    Ok(filepath)
}

/// 将 base64 data URL 解码并统一转码为 PNG，保存为临时文件，返回路径
///
/// 统一转 PNG 的原因：
/// - 大模型视觉 API（MiniMax 等）拒绝 `image/bmp`，PNG 原生支持
/// - PNG 有压缩，体积远小于未压缩 BMP（省磁盘、省 IO）
///
/// 文件名基于内容 hash，确保相同图片始终写入相同路径，不破坏 prompt cache 前缀。
/// 首次调用写入磁盘，后续命中已存在文件直接返回。
fn save_base64_to_temp_png(data_url: &str) -> Result<std::path::PathBuf, String> {
    // 解析 data:image/<mime>;base64,xxxx（bmp/png/jpeg 等，统一转 PNG）
    let base64_data = if let Some(comma_pos) = data_url.find(',') {
        &data_url[comma_pos + 1..]
    } else {
        return Err("Invalid data URL: no comma found".to_string());
    };

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| format!("Base64 decode failed: {e}"))?;

    // 统一转码 PNG（若已是 PNG 会轻微重编码，保证幂等性与格式一致）
    let img = image::load_from_memory(&bytes).map_err(|e| format!("Image decode failed: {e}"))?;
    let mut png_buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png_buf, image::ImageFormat::Png)
        .map_err(|e| format!("PNG encode failed: {e}"))?;
    let png_bytes = png_buf.into_inner();

    // 用内容 hash 替代时间戳，保证相同图片 → 相同路径
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    png_bytes.hash(&mut hasher);
    let hash = hasher.finish();

    let temp_dir = std::env::temp_dir();
    let filename = format!("nuphus_img_{:016x}.png", hash);
    let filepath = temp_dir.join(filename);

    // 文件已存在则直接返回（幂等写入）
    if filepath.exists() {
        return Ok(filepath);
    }

    std::fs::write(&filepath, &png_bytes).map_err(|e| format!("Failed to write temp PNG: {e}"))?;

    Ok(filepath)
}
