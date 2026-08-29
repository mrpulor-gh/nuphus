//! session — 聊天会话生命周期命令

use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn get_session_info() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "name": "Nuphus",
        "description": "协同共生桌面助手"
    }))
}

/// 获取当前 Leader Agent 的完整对话历史
/// 前端刷新后调用此命令恢复主对话，弥补实时事件丢失
/// 当 agent 不存在或 session 为空时，从 session_backup 恢复最新记录
#[tauri::command]
pub fn get_chat_history(
    state: State<'_, AppState>,
) -> Result<Vec<crate::state::HistoryMessage>, String> {
    chat_history(state.inner())
}

/// 从 session 消息过滤出 UI 可见历史。
/// 追加指令段 [APPEND]：LLM 上下文保留完整指令语义，但 UI 历史需还原为用户原文
/// ——否则执行中发送的消息在刷新/重连后消失（用户看不到自己发过的内容）。
fn extract_history(session: &nuphus::session::Session) -> Vec<crate::state::HistoryMessage> {
    use nuphus::session::ContentBlock;
    let messages = session.messages();
    // 第一遍：收集 ToolResult（call_id → is_error），供 assistant 消息的 ToolUse 补完成状态。
    // Session 完整存储工具调用与结果；历史拉取时据此还原「执行过程」供手机端显示完成状态。
    let mut tool_results: std::collections::HashMap<String, bool> =
        std::collections::HashMap::new();
    for m in messages.iter() {
        for block in &m.content {
            if let ContentBlock::ToolResult {
                tool_use_id,
                is_error,
                ..
            } = block
            {
                tool_results.insert(tool_use_id.clone(), *is_error);
            }
        }
    }

    // 提炼配对：REFINE_PROMPT（user）过滤自身；紧随其后的 assistant（提炼输出
    // 摘要）降级为独立 refine 角色输出——摘要内容完整保留（Leader 自我提炼的决策
    // 记忆，只有 Leader 知道发生过什么、如何决策），但不冒充普通 assistant 最后一轮
    // （否则手机端折叠连续 assistant 会把提炼前一轮回复覆盖成摘要）。
    let mut prev_was_refine_prompt = false;

    messages
        .iter()
        .filter_map(|m| {
            let role = match m.role {
                nuphus::session::MessageRole::User => "user",
                nuphus::session::MessageRole::Assistant => "assistant",
                nuphus::session::MessageRole::System => "system",
                nuphus::session::MessageRole::Tool => "tool",
            };
            let content = m.text_content();
            let images = m.image_urls();
            let audio = m.audio_urls();

            // 提炼提示词（user）→ 过滤自身 + 标记紧随的 assistant 为提炼输出。
            // 前缀匹配固定模板，不模糊——真实用户消息不会以「开始进行上下文提炼」开头。
            if role == "user" && content.starts_with("开始进行上下文提炼") {
                prev_was_refine_prompt = true;
                return None;
            }
            // 提炼输出摘要：紧跟 REFINE_PROMPT 的第一条消息若是 assistant → 以独立
            // refine 角色输出（内容保留）。flag 只在紧随消息上消费、不悬空（提炼失败后
            // 用户继续对话时，REFINE_PROMPT 后跟新 user 消息先消费 flag，后续正常
            // 回复不受影响）。
            if prev_was_refine_prompt {
                prev_was_refine_prompt = false;
                if role == "assistant" {
                    if content.trim().is_empty() {
                        return None;
                    }
                    return Some(crate::state::HistoryMessage {
                        role: "refine".to_string(),
                        content,
                        images,
                        audio,
                        timestamp: m.timestamp,
                        trace_items: Vec::new(),
                    });
                }
            }

            // 追加指令段（[APPEND]）：前端禁止显示——执行中发送的追加指令只进
            // LLM 上下文（internal=true），不还原为用户气泡。若还原为 user 会在
            // 刷新/重连后带出追加消息，分割 agent 回复气泡（用户实测：电脑端
            // agent 消息被分割、接收不到完整回复）。旧数据 internal=false 的
            // [APPEND] 段也在此统一过滤（不泄漏系统说明格式）。
            if role == "user" && nuphus::mobile_append::is_append_section(&content) {
                return None;
            }

            // 提炼摘要（replace_with_distill / accumulate_distill 写入基础层）：
            // 内容以固定前缀开头、internal=true 的 System 消息 → 降级为独立
            // refine 角色输出（内容保留）。refine 本质是一轮正常对话，其最终回复
            // 就是摘要——提取出来正常显示（前端以独立气泡展现），不被 internal
            // 过滤吞掉。前缀精确匹配固定模板，不模糊——真实 System 消息不会以
            // 「[当前 session 对话内容已触发提炼」开头。
            if role == "system" && content.starts_with("[当前 session 对话内容已触发提炼")
            {
                let body = content
                    .strip_prefix("[当前 session 对话内容已触发提炼，以下是提炼的内容，非当前指令]")
                    .unwrap_or(&content)
                    .trim_start_matches('\n')
                    .trim()
                    .to_string();
                if body.is_empty() {
                    return None;
                }
                return Some(crate::state::HistoryMessage {
                    role: "refine".to_string(),
                    content: body,
                    images,
                    audio,
                    timestamp: m.timestamp,
                    trace_items: Vec::new(),
                });
            }

            // 跳过空内容、系统内部消息（internal 标记——reminders/门铃/安全检查警告等
            // 只进 LLM 上下文，不显示在前端历史）、以及 system 前缀 [ 的内部提示。
            if (content.trim().is_empty() && images.is_empty() && audio.is_empty())
                || m.internal
                || (role == "system" && content.starts_with('['))
            {
                return None;
            }
            // 系统收尾提示（后端行为，用户不可见）：旧数据 internal=false 兜底过滤。
            // 精确匹配固定模板，不模糊——真实 agent 输出不会被误伤。
            if role == "assistant" {
                let t = content.trim();
                if t == "达到最大迭代次数"
                    || (t.starts_with("任务完成（共执行 ") && t.ends_with(" 步）"))
                {
                    return None;
                }
            }
            // 提炼系统提示词（distill::REFINE_PROMPT）：旧数据 internal=false 兜底过滤。
            // 提炼是内部流程，提示词不应以「用户消息」形式显示在桌面/手机历史。
            // 精确前缀匹配固定模板，不模糊——真实用户消息不会被误伤。
            if role == "user" && content.starts_with("开始进行上下文提炼") {
                return None;
            }
            // 上下文用量系统提示（react_loop 每 ~100K tokens 注入的 leader_memory_update
            // 提醒）：旧数据 internal=false 兜底过滤。内部行为提示，不应显示在前端历史。
            // 前缀匹配固定模板，不模糊——真实用户消息不会以「[系统提示词]」开头。
            if role == "user" && content.starts_with("[系统提示词]") {
                return None;
            }
            // 组装执行过程 trace_items（思考/流式文本/工具调用，按实际顺序）。
            // Session 完整存储 ToolUse/ToolResult——历史拉取还原真实执行过程，
            // 手机端显示完成状态（tool 状态由 ToolResult 补 ok/fail）。
            let mut trace_items: Vec<crate::state::HistoryTraceItem> = Vec::new();
            for block in &m.content {
                match block {
                    ContentBlock::Text { text, reasoning } => {
                        if let Some(r) = reasoning {
                            let r = r.trim();
                            if !r.is_empty() {
                                trace_items.push(crate::state::HistoryTraceItem {
                                    kind: "thinking".into(),
                                    call_id: None,
                                    name: None,
                                    status: None,
                                    params: None,
                                    text: Some(r.to_string()),
                                });
                            }
                        }
                        let t = text.trim();
                        if !t.is_empty() {
                            trace_items.push(crate::state::HistoryTraceItem {
                                kind: "text".into(),
                                call_id: None,
                                name: None,
                                status: None,
                                params: None,
                                text: Some(t.to_string()),
                            });
                        }
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        let status = tool_results
                            .get(id)
                            .map(|is_err| if *is_err { "fail" } else { "ok" })
                            .unwrap_or("running")
                            .to_string();
                        trace_items.push(crate::state::HistoryTraceItem {
                            kind: "tool".into(),
                            call_id: Some(id.clone()),
                            name: Some(name.clone()),
                            status: Some(status),
                            params: Some(input.to_string()),
                            text: None,
                        });
                    }
                    _ => {}
                }
            }
            Some(crate::state::HistoryMessage {
                role: role.to_string(),
                content,
                images,
                audio,
                timestamp: m.timestamp,
                trace_items,
            })
        })
        .collect()
}

/// 把「当前轮 user 消息」补进历史末尾（仅 agent 被 take 期间走 session_backup 时调用）。
/// session_backup 是执行前快照，不含当前轮 user；而 SessionState.last_message 在
/// 非 busy 受理时更新（append 追加指令不更新、系统提示不更新）——用它补回，
/// 保证执行中刷新/息屏重开历史可见「最后的 turn 的 user 消息」。
/// 安全边界：只补正常 user 消息；追加消息（前端禁止显示）与系统提示不会进入。
fn append_last_turn_user(
    mut msgs: Vec<crate::state::HistoryMessage>,
    last_message: &str,
    last_images: &[String],
) -> Vec<crate::state::HistoryMessage> {
    let text = last_message.trim();
    // 空内容且无图 → 无可补回（纯图消息 future 场景除外：有 images 就补）
    if text.is_empty() && last_images.is_empty() {
        return msgs;
    }
    // 已含同 content（刚放回 agent / 执行完成间隙）则不重复附加
    if msgs.iter().any(|m| m.role == "user" && m.content == text) {
        return msgs;
    }
    msgs.push(crate::state::HistoryMessage {
        role: "user".to_string(),
        content: text.to_string(),
        images: last_images.to_vec(),
        audio: Vec::new(),
        timestamp: None,
        trace_items: Vec::new(),
    });
    msgs
}

/// 对话历史读取（桌面 get_chat_history 命令与 mobile_server GET /history 共用）
///
/// 选择策略（后端权威 current_mode 驱动，替换旧「最近活跃 timestamp 猜测」）：
/// 1. 主模式 session（current_mode 对应 agent）非空 → 返回主模式历史
/// 2. 空闲（busy=false）且主模式 session 空/不可用 → 回退另一侧 agent（不丢历史）
/// 3. session_backup（AppState 持久化 JSON）—— 执行中 agent 被 take() 移出
///    （process.rs 执行开始前备份，执行完成放回），busy=true 时跳过另一侧回退，
///    直接走 backup：手机端在电脑端执行中重进页面仍能拉到执行前历史；agent
///    从未创建（无备份）→ 空列表欢迎页
///
/// ⚠️ 注意：不要在此附加 append_queue 的 pending 消息为 user——追加消息只应以
/// [APPEND] 段经 extract_history 提取（extract_append_user_text）进入历史，
/// 直接附加原始队列内容会把系统提示词带出到用户可见历史。
pub(crate) fn chat_history(state: &AppState) -> Result<Vec<crate::state::HistoryMessage>, String> {
    let guard = state.runtime.lock().map_err(|e| e.to_string())?;

    // ── current_mode 驱动：主模式 session 优先。custom 模式走 leader 主循环
    // （session 在 leader_agent），按 leader 处理。
    let current_mode = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    let is_workflow = current_mode == "workflow";

    let primary = if is_workflow {
        guard
            .workflow_agent
            .as_ref()
            .map(|a| extract_history(a.session()))
            .unwrap_or_default()
    } else {
        guard
            .leader_agent
            .as_ref()
            .map(|a| extract_history(a.session()))
            .unwrap_or_default()
    };
    if !primary.is_empty() {
        tracing::info!(
            "[CHAT] get_chat_history returned {} messages from {} agent (current_mode={})",
            primary.len(),
            if is_workflow { "workflow" } else { "leader" },
            current_mode
        );
        return Ok(primary);
    }

    // 空闲且主模式 session 空/不可用 → 先检查切换中转 backup，再回退另一侧（不丢历史）。
    // busy=true（执行中 agent 被 take）→ 不回退另一侧，直接走 session_backup（当前模式快照）
    let busy = state.busy.load(std::sync::atomic::Ordering::SeqCst);
    if !busy {
        // 切换中转目标优先：跨 mode 点击会话、目标 agent 槽为 None 时，目标会话经
        // switch_session 降级路径写入 session_backup。此时 primary 为空，必须先返回
        // backup 中的目标会话；否则回退另一侧 agent 会显示「当前对话」而非目标会话
        // （回归 2026-08-30：点击其它 mode 会话聊天区不变）。
        if let Ok(sb) = state.session.lock() {
            if let Some(ref json) = sb.session_backup {
                if let Ok(sess) = serde_json::from_str::<nuphus::session::Session>(json) {
                    let messages = extract_history(&sess);
                    if !messages.is_empty() {
                        let messages = append_last_turn_user(
                            messages,
                            &sb.last_message,
                            &sb.last_message_images,
                        );
                        tracing::info!(
                            "[CHAT] get_chat_history returned {} messages from switch backup (current_mode={})",
                            messages.len(),
                            current_mode
                        );
                        return Ok(messages);
                    }
                }
            }
        }
        let secondary = if is_workflow {
            guard
                .leader_agent
                .as_ref()
                .map(|a| extract_history(a.session()))
                .unwrap_or_default()
        } else {
            guard
                .workflow_agent
                .as_ref()
                .map(|a| extract_history(a.session()))
                .unwrap_or_default()
        };
        if !secondary.is_empty() {
            tracing::info!(
                "[CHAT] get_chat_history returned {} messages from {} agent (fallback, current_mode={})",
                secondary.len(),
                if is_workflow { "leader" } else { "workflow" },
                current_mode
            );
            return Ok(secondary);
        }
    }

    // 主模式执行中（busy）或两侧都空 → 走 session_backup / last_message / 空列表逻辑。
    // agent 不存在或 session 为空 → 回退从 session_backup 恢复（执行中 agent 被
    // take 移出时仍能读到执行前历史；retry.rs 用同一 JSON 恢复 session 的先例）
    drop(guard);
    if let Ok(sb) = state.session.lock() {
        if let Some(ref json) = sb.session_backup {
            if let Ok(sess) = serde_json::from_str::<nuphus::session::Session>(json) {
                let messages = extract_history(&sess);
                if !messages.is_empty() {
                    // 执行中 agent take：backup 是执行前快照，补当前轮 user 消息（含图）
                    let messages =
                        append_last_turn_user(messages, &sb.last_message, &sb.last_message_images);
                    tracing::info!(
                        "[CHAT] get_chat_history returned {} messages from session_backup",
                        messages.len()
                    );
                    return Ok(messages);
                }
            }
        }
        // backup 为空（首次执行 / 新 session）但有当前轮 user → 直接返回它，
        // 否则执行中刷新/息屏重开连第一条消息都看不到。
        if !sb.last_message.trim().is_empty() {
            return Ok(vec![crate::state::HistoryMessage {
                role: "user".to_string(),
                content: sb.last_message.trim().to_string(),
                images: Vec::new(),
                audio: Vec::new(),
                timestamp: None,
                trace_items: Vec::new(),
            }]);
        }
    }

    // agent session 为空且无备份时不自动恢复，显示欢迎界面
    tracing::info!("[CHAT] Agent session empty, returning empty for welcome screen");
    Ok(vec![])
}
#[cfg(test)]
mod tests {
    use super::*;
    use nuphus::session::{ContentBlock, Session};

    fn assistant(text: &str) -> Vec<ContentBlock> {
        vec![ContentBlock::Text {
            text: text.to_string(),
            reasoning: None,
        }]
    }

    /// 旧数据（internal=false）残留的 REFINE_PROMPT user 消息必须被过滤（前缀兜底），
    /// 紧随其后的 assistant（提炼输出摘要）降级为独立 refine 角色——内容保留、不冒充 assistant。
    #[test]
    fn extract_history_filters_refine_prompt_legacy() {
        let mut session = Session::new();
        session.push_user(nuphus::agent::distill::REFINE_PROMPT.to_string());
        session.push_assistant(assistant("提炼摘要内容"));
        let hist = extract_history(&session);
        assert_eq!(
            hist.len(),
            1,
            "REFINE_PROMPT 旧数据应被过滤，提炼摘要降级为 refine"
        );
        assert_eq!(hist[0].role, "refine");
        assert_eq!(hist[0].content, "提炼摘要内容");
    }

    /// 新数据（internal=true）的 REFINE_PROMPT user 消息必须被过滤（internal 标记），
    /// 紧随其后的 assistant（提炼输出摘要）降级为独立 refine 角色。
    #[test]
    fn extract_history_filters_refine_prompt_internal() {
        let mut session = Session::new();
        session.push_user_internal(nuphus::agent::distill::REFINE_PROMPT.to_string());
        session.push_assistant(assistant("提炼摘要内容"));
        let hist = extract_history(&session);
        assert_eq!(
            hist.len(),
            1,
            "internal REFINE_PROMPT 应被过滤，提炼摘要降级为 refine"
        );
        assert_eq!(hist[0].role, "refine");
        assert_eq!(hist[0].content, "提炼摘要内容");
    }

    /// 核心场景：提炼失败/中断残留 [旧对话…, REFINE_PROMPT, 摘要] → 历史输出
    /// [旧对话…, refine(摘要)]——提炼前最后一轮 assistant 回复完整保留不被覆盖，
    /// 摘要以独立 refine 角色展示（内容不丢）。
    #[test]
    fn extract_history_refine_output_role_and_content_kept() {
        let mut session = Session::new();
        session.push_user("帮我整理这个项目".to_string());
        session.push_assistant(assistant("已整理完成，改动如下…"));
        session.push_user_internal(nuphus::agent::distill::REFINE_PROMPT.to_string());
        session.push_assistant(assistant("标题\n\n决策链与当前状态…"));
        let hist = extract_history(&session);
        assert_eq!(hist.len(), 3);
        assert_eq!(hist[0].role, "user");
        assert_eq!(hist[0].content, "帮我整理这个项目");
        // 提炼前最后一轮回复保留、角色不变
        assert_eq!(hist[1].role, "assistant");
        assert_eq!(hist[1].content, "已整理完成，改动如下…");
        // 提炼摘要降级为独立 refine 角色、内容完整保留
        assert_eq!(hist[2].role, "refine");
        assert_eq!(hist[2].content, "标题\n\n决策链与当前状态…");
    }

    /// 提炼输出配对不误伤后续对话：REFINE_PROMPT 后是新的 user 消息（提炼失败后
    /// 用户继续对话）时，flag 在 user 上消费，正常回复必须保留为 assistant。
    #[test]
    fn extract_history_refine_pair_keeps_followup_conversation() {
        let mut session = Session::new();
        session.push_user("任务一".to_string());
        session.push_assistant(assistant("完成一"));
        session.push_user_internal(nuphus::agent::distill::REFINE_PROMPT.to_string());
        session.push_user("任务二".to_string());
        session.push_assistant(assistant("完成二"));
        let hist = extract_history(&session);
        assert_eq!(hist.len(), 4, "REFINE_PROMPT 被过滤，其余 4 条正常对话保留");
        assert_eq!(hist[0].role, "user");
        assert_eq!(hist[0].content, "任务一");
        assert_eq!(hist[1].role, "assistant");
        assert_eq!(hist[1].content, "完成一");
        assert_eq!(hist[2].role, "user");
        assert_eq!(hist[2].content, "任务二");
        assert_eq!(hist[3].role, "assistant");
        assert_eq!(hist[3].content, "完成二");
    }

    /// 【决定性验证】真实 refine 成功路径：replace_with_distill 清空 session 后
    /// 只剩一条 internal System 摘要。extract_history 必须把摘要降级为独立
    /// refine 角色输出（内容保留），而不是因 internal 被过滤 → 历史为空。
    /// 方案 B 的配对逻辑只覆盖「提炼失败残留」（REFINE_PROMPT + assistant 摘要），
    /// 提炼成功后 session 结构完全不同——此测试验证成功路径。
    #[test]
    fn extract_history_after_replace_with_distill_keeps_summary() {
        let mut session = Session::new();
        session.push_user("帮我整理这个项目".to_string());
        session.push_assistant(assistant("已整理完成，改动如下…"));
        // 模拟 refine 成功路径：resume 产出摘要后 replace_with_distill 清空 session
        session.replace_with_distill("标题\n\n决策链与当前状态…");
        assert_eq!(
            session.messages().len(),
            1,
            "提炼成功后 session 只剩一条 System 摘要"
        );
        let hist = extract_history(&session);
        assert_eq!(
            hist.len(),
            1,
            "提炼摘要必须以 refine 角色输出，而非被 internal 过滤为空：实际 {:?}",
            hist.iter()
                .map(|h| (h.role.as_str(), &h.content))
                .collect::<Vec<_>>()
        );
        assert_eq!(hist[0].role, "refine");
        assert!(
            hist[0].content.contains("决策链与当前状态"),
            "摘要内容完整保留：{}",
            hist[0].content
        );
        assert!(
            !hist[0].content.starts_with("[当前 session"),
            "前缀包装应被剥离，只留摘要正文：{}",
            hist[0].content
        );
    }

    /// 真实用户消息（不以提炼前缀开头）不受影响
    #[test]
    fn extract_history_keeps_real_user_message() {
        let mut session = Session::new();
        session.push_user("帮我看下这个文件".to_string());
        session.push_assistant(assistant("好的"));
        let hist = extract_history(&session);
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].content, "帮我看下这个文件");
    }

    /// 执行中发送的追加指令（[APPEND] 段）：前端禁止显示——过滤掉，不还原为 user。
    /// 否则刷新/重连后带出追加消息，分割 agent 回复气泡（用户实测）。
    /// LLM 上下文保留 [APPEND] 包装不变（本函数只过滤展示层）。
    #[test]
    fn extract_history_filters_append_sections() {
        let mut session = Session::new();
        // 模拟 react_loop 轮次边界注入的 [APPEND] 段（internal=true）
        session.push_user_internal(nuphus::mobile_append::format_mobile_append_section(&[
            "追加第一条".to_string(),
            "追加第二条".to_string(),
        ]));
        session.push_assistant(assistant("正在处理"));
        let hist = extract_history(&session);
        assert_eq!(hist.len(), 1, "[APPEND] 段应被过滤，只剩 assistant 回复");
        assert_eq!(hist[0].role, "assistant");
        // 旧数据（internal=false 的 [APPEND] 段）同样过滤，不泄漏系统说明格式
        let mut legacy = Session::new();
        legacy.push_user(nuphus::mobile_append::format_mobile_append_section(&[
            "旧追加".to_string(),
        ]));
        let hist_legacy = extract_history(&legacy);
        assert!(hist_legacy.is_empty(), "旧 [APPEND] 段也应被过滤");
    }

    /// 历史消息时间戳透传：session Message 的 timestamp 应原样进入 HistoryMessage
    #[test]
    fn extract_history_passes_timestamp() {
        let mut session = Session::new();
        session.push_user("带时间的消息".to_string());
        session.push_assistant(assistant("回复"));
        let hist = extract_history(&session);
        assert_eq!(hist.len(), 2);
        // push_user/push_assistant 构造器自动填充 timestamp
        assert!(
            hist[0].timestamp.is_some(),
            "user 消息应带 timestamp，实际: {:?}",
            hist[0].timestamp
        );
        assert!(hist[1].timestamp.is_some(), "assistant 消息应带 timestamp");
        // timestamp 是合理的历史时间（不早于 2026-01-01，不晚于现在+1min）
        let ts = hist[0].timestamp.unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let early_2026 = 1767225600000u64; // 2026-01-01
        assert!(
            ts >= early_2026 && ts <= now + 60_000,
            "时间戳应在合理区间: {ts}"
        );
    }

    // ── chat_history 选择策略（current_mode 驱动） ──
    // 根因 v1：workflow 模式消息存于 workflow_agent 独立 session，chat_history 只读
    // leader_agent.session()（workflow 模式下不增长）→ workflow 历史为空。
    // 回归 v2：workflow 非空永远优先 → 切回 leader 后新发消息刷新丢失（返回 workflow
    // 旧历史）。现按后端权威 current_mode 选择主模式 agent；空闲时主模式为空回退另一侧；
    // 执行中（busy）走 session_backup（当前模式快照）。

    /// Minimal mock ApiClient for constructing WorkflowAgent / Runtime in unit tests
    struct MockApiClient;
    #[async_trait::async_trait]
    impl nuphus::api::ApiClient for MockApiClient {
        async fn stream(
            &self,
            _request: nuphus::api::MessageRequest,
        ) -> nuphus::Result<Vec<nuphus::api::AssistantEvent>> {
            Ok(vec![])
        }
        fn model_name(&self) -> &str {
            "mock"
        }
        fn provider_kind(&self) -> nuphus::api::ProviderKind {
            nuphus::api::ProviderKind::MiniMax
        }
    }

    /// 构造带受控 timestamp 的 Session（JSON 往返覆写）：
    /// - Some(ts) → 最后一条消息 timestamp 置为 ts
    /// - None → 全部消息 timestamp 置空
    ///
    /// 注：chat_history 已按 current_mode 选择（timestamp 不参与），此辅助仍用于
    /// 构造不同内容的 session 以便断言选择结果。
    fn session_with_ts(msgs: &[(&str, &str)], last_ts: Option<u64>) -> nuphus::session::Session {
        let mut s = nuphus::session::Session::new();
        for (u, a) in msgs {
            s.push_user(u.to_string());
            s.push_assistant(assistant(a));
        }
        let mut v = serde_json::to_value(&s).expect("session 序列化");
        if let Some(msgs_arr) = v["messages"].as_array_mut() {
            match last_ts {
                Some(ts) => {
                    if let Some(last) = msgs_arr.last_mut() {
                        last["timestamp"] = serde_json::json!(ts);
                    }
                }
                None => {
                    for m in msgs_arr.iter_mut() {
                        m["timestamp"] = serde_json::Value::Null;
                    }
                }
            }
        }
        serde_json::from_value(v).expect("session 反序列化")
    }

    fn workflow_agent_with(sess: nuphus::session::Session) -> nuphus::runtime::WorkflowAgent {
        let mut wa = nuphus::runtime::WorkflowAgent::new(
            std::sync::Arc::new(MockApiClient),
            nuphus::ToolRegistry::work_agent(),
            None,
            None,
            "mock".to_string(),
            "user".to_string(),
            "Nuphus".to_string(),
            nuphus::permissions::ToolPermissions::default(),
            0.5,
        );
        wa.session_mut().replace_messages(sess.messages().to_vec());
        wa
    }

    fn leader_runtime_with(sess: nuphus::session::Session) -> nuphus::runtime::Runtime {
        let mut rt = nuphus::runtime::RuntimeBuilder::new()
            .llm(std::sync::Arc::new(MockApiClient))
            .tools(nuphus::ToolRegistry::builtin())
            .build()
            .expect("RuntimeBuilder::build 应成功");
        rt.session_mut().replace_messages(sess.messages().to_vec());
        rt
    }

    /// 设置后端权威 current_mode（测试环境直接写 AppState.current_mode）
    fn set_current_mode(state: &AppState, mode: &str) {
        let mut cm = state.current_mode.write().unwrap();
        *cm = mode.to_string();
    }

    /// current_mode=workflow + workflow session 非空 → workflow 历史
    /// （即使 leader 侧有数据，也按权威 mode 返回 workflow）
    #[test]
    fn chat_history_workflow_mode_returns_workflow_session() {
        let state = AppState::default();
        set_current_mode(&state, "workflow");
        {
            let mut guard = state.runtime.lock().unwrap();
            guard.leader_agent = Some(leader_runtime_with(session_with_ts(
                &[("旧 leader 问题", "旧 leader 回复")],
                Some(1000),
            )));
            guard.workflow_agent = Some(workflow_agent_with(session_with_ts(
                &[("工作流问题", "工作流回复")],
                Some(2000),
            )));
        }
        let hist = chat_history(&state).unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].content, "工作流问题");
        assert_eq!(hist[1].content, "工作流回复");
    }

    /// current_mode=leader + leader session 非空 → leader 历史
    /// （即使 workflow 也有数据，切回 leader 后新发消息刷新不丢）
    #[test]
    fn chat_history_leader_mode_returns_leader_session() {
        let state = AppState::default();
        set_current_mode(&state, "leader");
        {
            let mut guard = state.runtime.lock().unwrap();
            guard.workflow_agent = Some(workflow_agent_with(session_with_ts(
                &[("工作流问题", "工作流回复")],
                Some(1000),
            )));
            guard.leader_agent = Some(leader_runtime_with(session_with_ts(
                &[("leader 新问题", "leader 新回复")],
                Some(2000),
            )));
        }
        let hist = chat_history(&state).unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].content, "leader 新问题");
        assert_eq!(hist[1].content, "leader 新回复");
    }

    /// current_mode=workflow + workflow session 空 + 空闲（busy=false）+ leader 非空
    /// → 回退 leader（不丢历史）
    #[test]
    fn chat_history_workflow_mode_falls_back_to_leader_when_idle() {
        let state = AppState::default();
        set_current_mode(&state, "workflow");
        {
            let mut guard = state.runtime.lock().unwrap();
            guard.workflow_agent = Some(workflow_agent_with(session_with_ts(&[], None))); // workflow session 空
            guard.leader_agent = Some(leader_runtime_with(session_with_ts(
                &[("leader 问题", "leader 回复")],
                None,
            )));
        }
        // busy 默认 false（空闲）
        let hist = chat_history(&state).unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].content, "leader 问题");
    }

    /// current_mode=workflow + workflow agent 被 take（None）+ 执行中（busy=true）
    /// + leader 非空 + session_backup 有 workflow 快照 → 返回 backup（不回退 leader）
    #[test]
    fn chat_history_busy_skips_fallback_uses_backup() {
        let state = AppState::default();
        set_current_mode(&state, "workflow");
        state.busy.store(true, std::sync::atomic::Ordering::SeqCst);
        {
            let mut guard = state.runtime.lock().unwrap();
            guard.workflow_agent = None; // 执行中 agent 被 take
            guard.leader_agent = Some(leader_runtime_with(session_with_ts(
                &[("leader 问题", "leader 回复")],
                None,
            )));
        }
        // session_backup 是执行前 workflow 快照（与 leader 内容不同）
        let backup_sess = {
            let mut s = Session::new();
            s.push_user("备份消息".to_string());
            s.push_assistant(assistant("备份回复"));
            serde_json::to_string(&s).unwrap()
        };
        {
            let mut sb = state.session.lock().unwrap();
            sb.session_backup = Some(backup_sess);
        }
        let hist = chat_history(&state).unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].content, "备份消息");
    }

    /// current_mode=leader + leader session 空 + 空闲（busy=false）+ workflow 非空
    /// → 回退 workflow（不丢历史）
    #[test]
    fn chat_history_leader_mode_falls_back_to_workflow() {
        let state = AppState::default();
        set_current_mode(&state, "leader");
        {
            let mut guard = state.runtime.lock().unwrap();
            guard.leader_agent = Some(leader_runtime_with(session_with_ts(&[], None))); // leader session 空
            guard.workflow_agent = Some(workflow_agent_with(session_with_ts(
                &[("工作流问题", "工作流回复")],
                None,
            )));
        }
        let hist = chat_history(&state).unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].content, "工作流问题");
    }

    /// 跨 mode 切换降级中转：current_mode=leader + leader agent 槽为 None（重启后
    /// agent 未创建）+ 空闲 + workflow 非空 + session_backup 为目标会话
    /// → 必须返回 backup 目标会话，而非回退 workflow（点击其它 mode 会话聊天区不变，回归 2026-08-30）
    #[test]
    fn chat_history_switch_backup_priority_over_secondary() {
        let state = AppState::default();
        set_current_mode(&state, "leader"); // 已切到目标 mode
        {
            let mut guard = state.runtime.lock().unwrap();
            guard.leader_agent = None; // 目标 agent 槽为 None → 降级 backup 中转
            guard.workflow_agent = Some(workflow_agent_with(session_with_ts(
                &[("当前对话", "当前回复")],
                None,
            )));
        }
        // switch_session 降级路径写入的目标会话（与 workflow 内容不同）
        let target_sess = {
            let mut s = Session::new();
            s.push_user("目标会话消息".to_string());
            s.push_assistant(assistant("目标会话回复"));
            serde_json::to_string(&s).unwrap()
        };
        {
            let mut sb = state.session.lock().unwrap();
            sb.session_backup = Some(target_sess);
        }
        let hist = chat_history(&state).unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].content, "目标会话消息");
    }

    /// 无 agent、无备份 → 空列表（欢迎页，不回归）
    #[test]
    fn chat_history_returns_empty_for_fresh_start() {
        let state = AppState::default();
        let hist = chat_history(&state).unwrap();
        assert!(hist.is_empty());
    }
}
