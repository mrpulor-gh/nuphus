//! Session 核心结构体与会话生命周期
//!
//! 包含 Session 的定义、创建/恢复、消息推送、状态查询等生命周期管理。
//! API 格式转换和工具对清理在 transform.rs 中。

use crate::session::types::*;
use serde::{Deserialize, Serialize};

/// 会话
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// 会话唯一 ID
    pub id: String,
    /// 父会话 ID（通过提炼/分裂产生时设置）
    pub parent_id: Option<String>,
    /// 子会话 ID 列表（后续分裂出去的会话）
    pub child_ids: Vec<String>,
    /// 会话链深度（0=根，每分裂+1，≥10 拒绝继续分裂）
    pub depth: u32,
    pub(crate) messages: Vec<Message>,
    /// 是否已提炼
    refined: bool,
    /// 上次提炼移除的消息（供 Leader LLM 摘要用）
    /// 提炼后被清空，Leader 读取后被清空
    pub pending_refine_msgs: Vec<Message>,
    /// 当前提炼策略（供后续记忆写入决策用）
    pub last_refine_strategy: Option<RefineStrategy>,
    /// 上次 API 返回的实际 input_tokens (prompt_tokens + prompt_cache_hit_tokens)
    /// 保持 session 内峰值——status-bar 和 refine 共用，不受 KV cache 波动影响
    pub api_input_tokens: u64,
    /// turn 计数器 — 每次用户发送消息时递增。
    /// 用作 memory entry 的 turn_id，用于因果链追踪。
    #[serde(default)]
    pub turn_count: u32,
}

impl Session {
    /// 创建新会话
    pub fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id: None,
            child_ids: Vec::new(),
            depth: 0,
            messages: Vec::new(),
            refined: false,
            pending_refine_msgs: Vec::new(),
            last_refine_strategy: None,
            api_input_tokens: 0,
                    turn_count: 0,
        }
    }

    /// 当前 turn ID（如 "t1", "t2"）— 用作 memory entry 的 turn_id
    pub fn current_turn_id(&self) -> String {
        format!("t{}", self.turn_count)
    }

    /// 递增 turn 计数器并返回新的 turn ID
    pub fn advance_turn(&mut self) -> String {
        self.turn_count += 1;
        format!("t{}", self.turn_count)
    }

    /// 创建带父会话引用的子会话（由 refine/divide 产生）
    /// `parent_depth`: 父会话的 depth 值，子会话 depth = parent_depth + 1
    pub fn new_with_parent(parent_id: &str, parent_depth: u32) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id: Some(parent_id.to_string()),
            child_ids: Vec::new(),
            depth: parent_depth + 1,
            messages: Vec::new(),
            refined: false,
            pending_refine_msgs: Vec::new(),
            last_refine_strategy: None,
            api_input_tokens: 0,
                    turn_count: parent_depth + 1,
        }
    }

    /// 从历史消息构建会话（用于多轮对话上下文恢复）
    ///
    /// history: Vec<(role, content)>, role 为 "user" | "assistant" | "system"
    ///
    /// 警告：这仅创建 ContentBlock::Text —— 所有 ToolUse/ToolResult block
    /// 都会被丢弃。 优先使用完整的 Session clone（backup_session）。
    /// 此函数是没有结构化 session 数据时的最终回退路径。
    pub fn from_history(history: Vec<(String, String)>) -> Self {
        tracing::warn!(
            "[SESSION] from_history called with {} messages - text-only session \
             (ToolUse/ToolResult discarded). This is a degraded recovery path; \
             prefer backup_session clone.",
            history.len()
        );
        let messages: Vec<Message> = history
            .into_iter()
            .map(|(role, content)| {
                let role_enum = match role.as_str() {
                    "user" => MessageRole::User,
                    "assistant" => MessageRole::Assistant,
                    "system" => MessageRole::System,
                    _ => MessageRole::User,
                };
                Message {
                    role: role_enum,
                    content: vec![ContentBlock::Text {
                        text: content,
                        reasoning: None,
                    }],
                    internal: false,
                    timestamp: None,
                }
            })
            .collect();

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id: None,
            child_ids: Vec::new(),
            depth: 0,
            messages,
            refined: false,
            pending_refine_msgs: Vec::new(),
            last_refine_strategy: None,
            api_input_tokens: 0,
                    turn_count: 0,
        }
    }

    // ════════════════════════════════════════════════════════════════
    // 消息推送
    // ════════════════════════════════════════════════════════════════

    /// 添加用户消息
    pub fn push_user(&mut self, text: String) {
        self.messages.push(Message::user_text(text));
    }

    /// 添加内部注入用户消息（reminders/门铃/追加指令/系统提示等）
    /// 只进 LLM 上下文，前端历史拉取时被过滤，不显示。
    pub fn push_user_internal(&mut self, text: String) {
        self.messages.push(Message::user_text_internal(text));
    }

    /// 添加系统消息（安全检查警告等内部提示，不显示在前端）
    pub fn push_system(&mut self, text: String) {
        self.messages.push(Message {
            role: MessageRole::System,
            content: vec![ContentBlock::Text {
                text,
                reasoning: None,
            }],
            internal: true,
            timestamp: Some(crate::session::types::now_ms()),
        });
    }

    /// 添加助手消息
    pub fn push_assistant(&mut self, blocks: Vec<ContentBlock>) {
        self.messages.push(Message::assistant(blocks));
    }

    /// 添加内部助手消息（系统收尾提示等——「任务完成」「达到最大迭代次数」）
    /// 只进 LLM 上下文，前端历史拉取时被过滤，不显示。
    /// 用户不需要也不能看到系统提示（系统行为对用户不可见）。
    pub fn push_assistant_internal(&mut self, blocks: Vec<ContentBlock>) {
        let mut msg = Message::assistant(blocks);
        msg.internal = true;
        self.messages.push(msg);
    }

    /// 添加工具结果消息
    pub fn push_tool_result(&mut self, tool_use_id: String, content: String, is_error: bool) {
        self.messages
            .push(Message::tool_result(tool_use_id, content, is_error));
    }

    /// 添加用户消息（支持多 content block，含 Image）
    pub fn push_user_blocks(&mut self, content: Vec<ContentBlock>) {
        self.messages.push(Message {
            role: MessageRole::User,
            content,
            internal: false,
            timestamp: Some(crate::session::types::now_ms()),
        });
    }

    // ════════════════════════════════════════════════════════════════
    // 状态查询
    // ════════════════════════════════════════════════════════════════

    /// 获取所有消息
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// 获取消息数量
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// 是否已提炼
    pub fn is_refined(&self) -> bool {
        self.refined
    }

    /// 清空消息
    pub fn clear(&mut self) {
        self.messages.clear();
        self.refined = false;
    }

    // ════════════════════════════════════════════════════════════════
    // 消息管理
    // ════════════════════════════════════════════════════════════════

    /// 移除最后一条用户消息（用于中断/取消后清理孤儿消息）
    /// 如果最后一条是 user message 则删除，否则不操作
    pub fn pop_last_user_if_orphan(&mut self) {
        if let Some(last) = self.messages.last() {
            if last.role == MessageRole::User {
                self.messages.pop();
            }
        }
    }

    /// 检查最后一条消息是否为 user 且文本内容匹配
    /// 用于 drain_pending_append 避免同一条指令重复注入
    pub fn last_message_is_user_with(&self, content: &str) -> bool {
        if let Some(last) = self.messages.last() {
            if last.role != MessageRole::User {
                return false;
            }
            let text_content: String = last
                .content
                .iter()
                .filter_map(|block| {
                    if let ContentBlock::Text { text, .. } = block {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            text_content == content
        } else {
            false
        }
    }

    /// 替换所有消息（用于 ReAct 提炼等需要重建会话的场景）
    pub fn replace_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        self.refined = false;
    }

    /// 方案 1: 提炼后原始对话存入 memory，用提炼内容替换当前会话历史
    /// 保留 session ID、depth 不变，清空 messages，插入引导消息
    pub fn replace_with_distill(&mut self, distill_content: &str) {
        self.refined = true;
        let _ = std::mem::take(&mut self.messages);
        self.messages.push(Message {
            role: MessageRole::System,
            content: vec![ContentBlock::Text {
                text: format!(
                    "[当前 session 对话内容已触发提炼，以下是提炼的内容，非当前指令]\n\n{}",
                    distill_content
                ),
                reasoning: None,
            }],
            internal: true,
            timestamp: Some(crate::session::types::now_ms()),
        });
    }

    /// 累积提炼：保留已有的提炼 System 消息，清空其余消息，追加新摘要
    /// 用于同一 session 多次 refine —— 先前摘要不动，前缀缓存不失效
    pub fn accumulate_distill(&mut self, distill_content: &str) {
        self.refined = true;
        self.messages.retain(|m| m.role == MessageRole::System);
        self.messages.push(Message {
            role: MessageRole::System,
            content: vec![ContentBlock::Text {
                text: format!(
                    "[当前 session 对话内容已触发提炼，以下是提炼的内容，非当前指令]\n\n{}",
                    distill_content
                ),
                reasoning: None,
            }],
            internal: true,
            timestamp: Some(crate::session::types::now_ms()),
        });
    }

    /// 移除并返回最后一条 User 消息（用于 refine 失败后清理残留 prompt）
    pub fn pop_last_user_message(&mut self) -> Option<Message> {
        if let Some(pos) = self
            .messages
            .iter()
            .rposition(|m| m.role == MessageRole::User)
        {
            Some(self.messages.remove(pos))
        } else {
            None
        }
    }

    /// 获取父会话 ID
    pub fn parent_id(&self) -> Option<&str> {
        self.parent_id.as_deref()
    }

    /// 获取子会话 ID 列表
    pub fn child_ids(&self) -> &[String] {
        &self.child_ids
    }

    /// 获取最近 N 条消息
    pub fn recent_messages(&self, n: usize) -> Vec<&Message> {
        self.messages
            .iter()
            .rev()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// 消费上次提炼移除的消息（Leader LLM 摘要后调用）
    pub fn consume_pending_refine_msgs(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.pending_refine_msgs)
    }

    /// 归档：返回所有消息并清空当前会话（保存旧会话数据用）
    pub fn archive(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.messages)
    }

    /// 创建子会话：创建新的空会话，生成新 ID
    pub fn create_child(&self) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id: Some(self.id.clone()),
            child_ids: Vec::new(),
            depth: self.depth + 1,
            messages: Vec::new(),
            refined: false,
            pending_refine_msgs: Vec::new(),
            last_refine_strategy: None,
            api_input_tokens: 0,
                    turn_count: 0,
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
