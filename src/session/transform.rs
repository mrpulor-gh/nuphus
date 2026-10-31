//! Session API 格式转换与工具对清理
//!
//! 包含 Session 与 LLM API 之间的消息格式转换（to_api_messages）、
//! 不完整工具调用的清理（strip_incomplete_tools）、
//! 提炼标记插入（insert_refine_marker/insert_divide_anchor）、
//! Token 用量估算等功能。

use crate::session::session::Session;
use crate::session::types::*;

/// 用户强制中断时，为未配对的 ToolUse 补写的 ToolResult 内容。
/// 明确三件事：被叫停的是用户、该调用没有产出、不要假设它成功。
const INTERRUPTED_TOOL_NOTICE: &str = "[中断] 用户强制中断了本次执行，该工具调用未产生任何结果。请等待用户的下一步指令，不要假设此调用已成功。";

/// 序列化层：把 tool 消息归位到发出它的 assistant 之后（协议邻接要求）。
///
/// 运行时会在 assistant(tool_use) 与对应 tool_result 之间插入上下文消息
/// （`push_system` 的安全检查告警、ACTIVE REMINDERS 提醒等）。序列化后形态是
/// `assistant(tool_calls) → user(提醒) → tool(result)`，两种协议都判非法：
/// - Anthropic Messages：`tool_result` 必须紧跟发起 `tool_use` 的 assistant，
///   否则 400 `tool call result does not follow tool call`；
/// - 严格 OpenAI 兼容端点（DeepSeek 等）：`tool` 消息必须紧接带对应
///   `tool_call_id` 的 assistant(tool_calls)。
///
/// 只在**序列化输出**上归位，不改会话存储（与 System→user 同一条纪律）：
/// 每个 tool 消息移到其就近前驱 assistant 之后，同组保持相对顺序；其它消息
/// （提醒 / 摘要 / 用户输入）原序保留。模型读到的顺序变成「先工具结果、后提醒」，
/// 这正是协议允许、也是提醒本来的生效时机（下一轮开始前）。
/// 找不到对应 tool_call 的孤儿 tool 消息丢弃并 warn —— 两种协议都拒收。
fn normalize_tool_adjacency(messages: &mut Vec<serde_json::Value>) {
    use std::collections::HashMap;

    if !messages.iter().any(|m| m["role"].as_str() == Some("tool")) {
        return;
    }

    // tool 下标 → 归属的 assistant 下标（就近前驱：同 id 跨轮复用时取最近的一轮）
    let mut owner: HashMap<usize, usize> = HashMap::new();
    let mut orphans = 0usize;
    for i in 0..messages.len() {
        if messages[i]["role"].as_str() != Some("tool") {
            continue;
        }
        let found = messages[i]["tool_call_id"].as_str().and_then(|id| {
            (0..i).rev().find(|&j| {
                messages[j]["role"].as_str() == Some("assistant")
                    && messages[j]["tool_calls"]
                        .as_array()
                        .is_some_and(|arr| arr.iter().any(|c| c["id"].as_str() == Some(id)))
            })
        });
        match found {
            Some(j) => {
                owner.insert(i, j);
            }
            None => orphans += 1,
        }
    }
    if orphans > 0 {
        tracing::warn!(
            "[transform] dropped {orphans} orphan tool message(s) without a matching tool_call"
        );
    }

    // 已经合法（无孤儿，且每个 tool 都排在它那组 assistant 结果的位置）→ 不动
    let already_adjacent = orphans == 0
        && owner
            .iter()
            .all(|(&t, &a)| (a + 1..t).all(|k| owner.get(&k) == Some(&a)));
    if already_adjacent {
        return;
    }

    let mut by_owner: HashMap<usize, Vec<usize>> = HashMap::new();
    for (&t, &a) in &owner {
        by_owner.entry(a).or_default().push(t);
    }
    for list in by_owner.values_mut() {
        list.sort_unstable();
    }

    // 拆成「保序的非 tool 消息」+「按归属的 tool 消息」，孤儿在拆分时丢弃
    let mut kept: Vec<(usize, serde_json::Value)> = Vec::with_capacity(messages.len());
    let mut tools: HashMap<usize, serde_json::Value> = HashMap::new();
    for (i, m) in messages.drain(..).enumerate() {
        if m["role"].as_str() == Some("tool") {
            if owner.contains_key(&i) {
                tools.insert(i, m);
            }
        } else {
            kept.push((i, m));
        }
    }

    let mut out: Vec<serde_json::Value> = Vec::with_capacity(kept.len() + tools.len());
    for (i, m) in kept {
        out.push(m);
        if let Some(list) = by_owner.remove(&i) {
            for t in list {
                if let Some(v) = tools.remove(&t) {
                    out.push(v);
                }
            }
        }
    }
    *messages = out;
}

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

    /// 用户强制中断后的工具调用封口：为每个未配对的 ToolUse 补一条 ToolResult，
    /// 把「被强制中断、该调用无结果」写进会话历史。
    ///
    /// 与 `strip_incomplete_tools` 的分工（有意不同，勿合并）：
    /// - `strip_incomplete_tools` **删除**痕迹，用于 LLM 报错 / 异常场景——那里的
    ///   未配对 ToolUse 是脏数据，写任何具体原因都是编造；
    /// - 本方法**保留痕迹并注入真实原因**，专用于用户主动中断。
    ///
    /// 为什么中断场景必须保留：ToolUse 一旦发出，对应的副作用（文件已写、命令已跑）
    /// 不会因为中断而消失。若把 ToolUse 从历史里抹掉，下一轮模型会遇到
    /// 「我发起过这个调用」与「历史上没有这个调用」的认知错位——它不知道自己
    /// 已经动过手，会在失真的世界状态上继续推理。补一条明确的 ToolResult 让
    /// 模型知道走到哪一步被叫停、且那一步没有产出。
    ///
    /// 幂等：已配对的 ToolUse 不重复补；无未配对 ToolUse 时为 no-op。
    pub fn seal_interrupted_tools(&mut self) {
        use std::collections::HashSet;
        let mut result_ids: HashSet<&str> = HashSet::new();
        for msg in &self.messages {
            for block in &msg.content {
                if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                    result_ids.insert(tool_use_id.as_str());
                }
            }
        }
        // 先只读地收集「哪条 assistant 消息之后要补哪个 id」，再统一插入。
        // 不可在同一轮里既借 self.messages 又 insert（E0502）。
        let mut pending: Vec<(usize, String)> = Vec::new();
        for (i, msg) in self.messages.iter().enumerate() {
            if msg.role != MessageRole::Assistant {
                continue;
            }
            for block in &msg.content {
                if let ContentBlock::ToolUse { id, .. } = block {
                    if !result_ids.contains(id.as_str()) {
                        pending.push((i, id.clone()));
                    }
                }
            }
        }
        // 逆序插入，避免 insert 造成 index 漂移
        for (i, id) in pending.into_iter().rev() {
            self.messages.insert(
                i + 1,
                Message::tool_result(id, INTERRUPTED_TOOL_NOTICE.to_string(), true),
            );
        }
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

        let mut out: Vec<serde_json::Value> = self
            .messages
            .iter()
            .map(|msg| {
                // 会话内的 System 消息（提炼摘要 / push_system 安全警告 / insert_refine_marker
                // 分裂锚点 / ExecPool 压缩）是**注进上下文的内容**，不是系统指令——真正的系统
                // 提示词由传输层单独下发（chat_completions 前置为 messages[0]、Anthropic 用顶层
                // `system` 字段）。若这里原样下发 system，报文会变成
                // [system(主提示), system(摘要), user…]：llama.cpp 的 Jinja 模板硬检查
                // 「System message must be at the beginning」→ 确定性 HTTP 500（云端模板静默合并，
                // 所以只在本地后端暴露）；Anthropic 适配器还会把这段内容替换成占位串而丢失摘要。
                // 以 user 角色下发对任何后端都合法，模型看到的也是内容本身。
                // 注意：仅改**序列化层**——存储仍保留 MessageRole::System + internal 标记，
                // 依赖该标记的内部逻辑（extract_history 过滤、shelf 快照等）不受影响。
                let role = match msg.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                    MessageRole::System => "user",
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
            .collect();
        // 协议邻接：tool 结果必须紧跟其 assistant（详见 normalize_tool_adjacency）
        normalize_tool_adjacency(&mut out);
        out
    }

    /// 估算当前消息列表 token 用量
    /// 优先使用 API 返回的实际 input_tokens，退化到字符估算
    ///
    /// ⚠️ 仅用于**内部水位启发式**（提炼触发判据、子任务/工作流的上下文告警阈值、
    /// dispatch 记录的会话规模）——那些场景要的是「含尚未过 API 的内容」的保守值，
    /// 官方读数覆盖不到刚追加的工具结果。
    /// **界面上的 ctx 占用不得用它**：`max()` 会让字符估算压过官方读数（已导致的
    /// 缺陷：轮次结束后 ctx 已用值不等于 API 真实 input_tokens）。界面一律走
    /// [`Self::context_occupancy`]。
    pub fn estimate_token_usage(&self) -> usize {
        let json_str = serde_json::to_string(&self.messages).unwrap_or_default();
        let char_estimate = (json_str.len() / 4) + 100;
        std::cmp::max(self.api_input_tokens as usize, char_estimate)
    }

    /// 当前上下文占用（**官方口径**）：最后一次 API 调用的 input + output。
    ///
    /// 为什么这样就够：一轮正常结束时，会话里最后追加的内容正是那一次调用的产出
    /// （末轮无工具调用，只有 text / thinking / response），所以
    /// `input + output` 就是「下一次请求的提示词规模」。三个数全部来自 API 的
    /// `usage`，没有任何本地估算；下一次调用到达时会被该次真实的 `input_tokens`
    /// 原地校正（自愈），误差窗口仅限两轮之间。
    ///
    /// 返回 0 = 尚无官方读数（新会话 / 提炼后已清零）→ 界面显示 "--"，
    /// 或由调用方决定是否退回 `estimate_token_usage`。
    pub fn context_occupancy(&self) -> u64 {
        self.api_input_tokens.saturating_add(self.api_output_tokens)
    }

    /// 阈值判据用的「当前上下文占用」——**与界面 ctx 完全同口径**。
    ///
    /// `refine` 分档线（prompt / force / 85% 硬线）与切换校验一律走这里，
    /// 禁止各写一套：曾经判据只读 `api_input_tokens`，而界面显示
    /// `context_occupancy`（input + output），导致「阈值提醒比轮末 ctx 少一截」
    /// （差额恒为最后一次调用的 output）。
    ///
    /// 无官方读数（新会话 / 提炼后清零）才回落 [`Self::estimate_token_usage`]：
    /// 那时会话内容只在本地手里，估算比直接不判更有用。
    pub fn context_watermark(&self) -> usize {
        let occupancy = self.context_occupancy();
        if occupancy > 0 {
            occupancy as usize
        } else {
            self.estimate_token_usage()
        }
    }

    /// 更新 API 返回的 input_tokens
    pub fn update_api_input_tokens(&mut self, count: u64) {
        self.api_input_tokens = count;
    }

    /// 更新 API 返回的 output_tokens（与 input 成对调用，见 `context_occupancy`）
    pub fn update_api_output_tokens(&mut self, count: u64) {
        self.api_output_tokens = count;
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

    // PNG 已是目标格式：magic 命中免解码重编码，直接落盘（screenshot 产物即 PNG）
    let png_bytes = if bytes.len() >= 8 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        bytes
    } else {
        // 其他格式（jpeg/bmp/webp）统一转 PNG，保证幂等性与格式一致
        let img =
            image::load_from_memory(&bytes).map_err(|e| format!("Image decode failed: {e}"))?;
        let mut png_buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png_buf, image::ImageFormat::Png)
            .map_err(|e| format!("PNG encode failed: {e}"))?;
        png_buf.into_inner()
    };

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
