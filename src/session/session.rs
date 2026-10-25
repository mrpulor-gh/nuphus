//! Session 核心结构体与会话生命周期
//!
//! 包含 Session 的定义、创建/恢复、消息推送、状态查询等生命周期管理。
//! API 格式转换和工具对清理在 transform.rs 中。

use crate::session::types::*;
use serde::{Deserialize, Serialize};

/// 提炼摘要 System 消息的元说明前缀（replace_with_distill / accumulate_distill 写入，
/// 会话预览据此剥离元说明只展示摘要正文）
pub const REFINE_SYSTEM_PREFIX: &str =
    "[当前 session 对话内容已触发提炼，以下是提炼的内容，非当前指令]";

/// 进程内提示词前缀快照（session 切换时复用，避免重建 L0+L2+L1）。
///
/// **只活在内存，永不落盘**——这是本结构的核心约束，不是实现细节：
/// - L1 里含每运行随机生成的门铃令牌（`handoff::HandoffState::token`）和
///   `chrono::Local` 当天日期。若随 `Session` 序列化进 SQLite，重启后恢复
///   旧会话会把死令牌/旧日期当成有效系统提示，外部 Agent 上报一律 403。
/// - 升级安装包必然重启进程 → 快照天然失效，提示词按新版本重建，不存在
///   「旧版本宪法滞留」的静默偏差。
///
/// 因此快照不需要 revision / created_at 之类的版本或时效判据：进程边界
/// 就是唯一且完备的失效条件。失效口径 = 「重启即重算」，与项目「重启走
/// 最新数据、不做缓存保障」一致。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSnapshot {
    pub system_prompt: String,
    #[serde(default)]
    pub tools: Vec<crate::api::ToolDefinition>,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub project_path: Option<String>,
    /// Custom 模式激活卡片 id；其它模式 None。防同工具集的两张卡串 L2。
    #[serde(default)]
    pub custom_agent_id: Option<String>,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub vision_model: Option<String>,
    #[serde(default)]
    pub tenets_section: String,
}

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
    /// 上次 API 返回的实际 output_tokens（completion_tokens，含 reasoning 段）
    ///
    /// 与 `api_input_tokens` 同源同时更新。**用途是算上下文占用**：一轮结束时
    /// 会话里最后追加的内容正是这一次调用的产出，故
    /// `context_occupancy = api_input_tokens + api_output_tokens`
    /// 即「下一次请求的提示词规模」的官方近似——不靠字符估算（见 transform.rs）。
    #[serde(default)]
    pub api_output_tokens: u64,
    /// turn 计数器 — 每次用户发送消息时递增。
    /// 用作 memory entry 的 turn_id，用于因果链追踪。
    #[serde(default)]
    pub turn_count: u32,
    /// 进程内提示词前缀快照（session 切换复用）。**永不落盘**：
    /// `skip` 使其不进任何 serde 序列化路径（SQLite 镜像 / session_backup /
    /// retry_session / 手机端回执），进程重启即自然丢弃，按最新数据重建。
    #[serde(skip)]
    pub prompt_snapshot: Option<PromptSnapshot>,
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
            api_output_tokens: 0,
            turn_count: 0,
            prompt_snapshot: None,
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
            api_output_tokens: 0,
            turn_count: parent_depth + 1,
            prompt_snapshot: None,
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
            api_output_tokens: 0,
            turn_count: 0,
            prompt_snapshot: None,
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
        self.prompt_snapshot = None;
    }

    pub fn prompt_snapshot(&self) -> Option<&PromptSnapshot> {
        self.prompt_snapshot.as_ref()
    }

    pub fn set_prompt_snapshot(&mut self, snapshot: PromptSnapshot) {
        self.prompt_snapshot = Some(snapshot);
    }

    pub fn clear_prompt_snapshot(&mut self) {
        self.prompt_snapshot = None;
    }

    pub fn set_prompt_snapshot_tenets(&mut self, tenets_section: String) {
        if let Some(snapshot) = self.prompt_snapshot.as_mut() {
            snapshot.tenets_section = tenets_section;
        }
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
    /// 用于避免同一条追加指令重复注入
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
        // 必须清零：force refine 判据（agent/distill.rs）在 api_input_tokens > 0 时直接取它，
        // 留旧峰值会让下一轮必然再次越过 force_limit（issue #9 附带发现）。清零后
        // estimate_token_usage 回落到提炼后小会话的字符估算，不会误触发。
        // output 一并清零：占用读数（context_occupancy）随会话内容作废，下次调用重建。
        self.api_input_tokens = 0;
        self.api_output_tokens = 0;
        let _ = std::mem::take(&mut self.messages);
        self.messages.push(Message {
            role: MessageRole::System,
            content: vec![ContentBlock::Text {
                text: format!("{}\n\n{}", REFINE_SYSTEM_PREFIX, distill_content),
                reasoning: None,
            }],
            internal: true,
            timestamp: Some(crate::session::types::now_ms()),
        });
    }

    /// 末条 assistant 消息的文本——refine 接力的取数口（模型产出摘要后
    /// 循环自然收敛，末条 assistant 即摘要正文）。无 assistant 消息返回 None。
    pub fn last_assistant_text(&self) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::Assistant)
            .map(|m| m.text_content())
    }

    /// 末条消息是否为「无工具调用的 assistant 纯文本回复」——refine 接力的
    /// 收敛闸门：模型遵守提炼提示词（产出摘要、不调工具）才允许整体替换
    /// 会话；否则（提示词未被遵守、循环走到耗尽/中断）不替换，历史不丢。
    pub fn last_assistant_is_plain_text(&self) -> bool {
        match self.messages.last() {
            Some(m) if m.role == MessageRole::Assistant => !m
                .content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { .. })),
            _ => false,
        }
    }

    /// 再次提炼：只保留当前 refine —— 移除过往 refine System 消息（内容已由
    /// save_refine_entry 归档至记忆系统 refine-{session_id}-{timestamp}），清空其余
    /// 消息，插入新摘要。避免 refine1+refine2 累加撑大小模型上下文，单对话可无限复用。
    /// 非 refine 的 System 消息（安全警告等）保留。
    pub fn accumulate_distill(&mut self, distill_content: &str) {
        self.refined = true;
        // 同 replace_with_distill：不清零则旧峰值残留，下一轮 force refine 必复发
        self.api_input_tokens = 0;
        self.api_output_tokens = 0;
        // 只保留最新 refine：移除过往 refine System 消息，非 refine 的 System 消息保留
        self.messages
            .retain(|m| !Session::is_refine_system_message(m));
        self.messages.push(Message {
            role: MessageRole::System,
            content: vec![ContentBlock::Text {
                text: format!("{}\n\n{}", REFINE_SYSTEM_PREFIX, distill_content),
                reasoning: None,
            }],
            internal: true,
            timestamp: Some(crate::session::types::now_ms()),
        });
    }

    /// 判断消息是否为 refine 摘要 System 消息（content 以 REFINE_SYSTEM_PREFIX 开头）
    pub(crate) fn is_refine_system_message(m: &Message) -> bool {
        m.role == MessageRole::System
            && m.content.iter().any(|b| {
                matches!(b, ContentBlock::Text { text, .. } if text.starts_with(REFINE_SYSTEM_PREFIX))
            })
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
            api_output_tokens: 0,
            turn_count: 0,
            prompt_snapshot: None,
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 核心不变量：提示词快照绝不进序列化产物。
    /// 一旦回归，重启恢复会把旧进程的门铃令牌/日期当成有效系统提示。
    #[test]
    fn prompt_snapshot_never_serialized() {
        let mut session = Session::new();
        session.set_prompt_snapshot(PromptSnapshot {
            system_prompt: "sys".to_string(),
            tools: Vec::new(),
            provider: "p".to_string(),
            model: "m".to_string(),
            mode: "leader".to_string(),
            project_path: None,
            custom_agent_id: None,
            supports_vision: false,
            vision_model: None,
            tenets_section: String::new(),
        });
        assert!(session.prompt_snapshot().is_some());

        let value = serde_json::to_value(&session).unwrap();
        assert!(
            value.get("prompt_snapshot").is_none(),
            "prompt_snapshot 必须被 serde(skip) 排除，否则会落盘"
        );

        // 往返后快照必然丢失 —— 这就是「重启即重算」的实现方式
        let restored: Session = serde_json::from_value(value).unwrap();
        assert!(restored.prompt_snapshot().is_none());
    }

    #[test]
    fn clear_resets_prompt_snapshot() {
        let mut session = Session::new();
        session.set_prompt_snapshot(PromptSnapshot {
            system_prompt: "sys".to_string(),
            tools: Vec::new(),
            provider: "p".to_string(),
            model: "m".to_string(),
            mode: "leader".to_string(),
            project_path: None,
            custom_agent_id: None,
            supports_vision: false,
            vision_model: None,
            tenets_section: String::new(),
        });
        session.set_prompt_snapshot_tenets("tenets".to_string());
        assert_eq!(session.prompt_snapshot().unwrap().tenets_section, "tenets");
        session.clear();
        assert!(session.prompt_snapshot().is_none());
    }
}
