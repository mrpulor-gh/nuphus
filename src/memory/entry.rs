//! entry.rs — unified memory entry structure
//!
//! Replaces RawTimelineEntry + RawExecutionRecord as the single event anchor for the Timeline memory system.
//!
//! v2（消费方驱动）：一维 `kind` 分类（conversation / task_trace / distill / pattern / snapshot）
//! 替代 agent_type × source × goal_type 三维自由组合。`agent_type` 降级为普通元数据（谁产生的），
//! `source` / `verified` / `understanding_meta` 已删除（无消费方）。

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ============================================================================
// Enums
// ============================================================================

/// Agent type（普通元数据：谁产生的这条记忆，不再承担分类职责）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub enum AgentType {
    Leader,
    Exec,
    WorkAgent,
}

impl AgentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentType::Leader => "leader",
            AgentType::Exec => "exec",
            AgentType::WorkAgent => "workagent",
        }
    }
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for AgentType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "leader" => Ok(AgentType::Leader),
            "exec" => Ok(AgentType::Exec),
            "workagent" => Ok(AgentType::WorkAgent),
            _ => Err(format!("unknown AgentType: {}", s)),
        }
    }
}

/// Memory kind — 一维分类，回答"这条记忆是什么"。
/// 决定保留策略、索引策略、UI 归属。serde 为 snake_case 字符串，与 DB CHECK 约束一致。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Hash, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// 会话对话对（user + assistant 全文），Leader 每轮写入，永久保留
    Conversation,
    /// 任务执行轨迹（紧凑步骤），Exec/Workflow 写入，90 天衰减
    TaskTrace,
    /// 会话提炼（LLM 语义压缩），refine 流程写入，永久保留
    Distill,
    /// 实战模式（评分产出，人工验证），评分链路写入，永久保留
    Pattern,
    /// 历史遗留：工作记忆已迁出 SQLite（纯 md 文件注入），不再产生新记录，
    /// 仅为历史数据查询兼容保留
    Snapshot,
}

impl MemoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryKind::Conversation => "conversation",
            MemoryKind::TaskTrace => "task_trace",
            MemoryKind::Distill => "distill",
            MemoryKind::Pattern => "pattern",
            MemoryKind::Snapshot => "snapshot",
        }
    }
}

impl std::fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MemoryKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "conversation" => Ok(MemoryKind::Conversation),
            "task_trace" => Ok(MemoryKind::TaskTrace),
            "distill" => Ok(MemoryKind::Distill),
            "pattern" => Ok(MemoryKind::Pattern),
            "snapshot" => Ok(MemoryKind::Snapshot),
            _ => Err(format!("unknown MemoryKind: {}", s)),
        }
    }
}

/// Time window
#[derive(Debug, Clone, Copy)]
pub enum TimeWindow {
    LastHour,
    LastDay,
    Last7Days,
    Last30Days,
    All,
}

impl TimeWindow {
    /// Return the window start wall_clock_ms (current time minus window length)
    pub fn start_ms(&self, now_ms: u64) -> u64 {
        match self {
            TimeWindow::LastHour => now_ms.saturating_sub(3_600_000),
            TimeWindow::LastDay => now_ms.saturating_sub(86_400_000),
            TimeWindow::Last7Days => now_ms.saturating_sub(604_800_000),
            TimeWindow::Last30Days => now_ms.saturating_sub(2_592_000_000),
            TimeWindow::All => 0,
        }
    }
}

// ============================================================================
// Execution steps — 紧凑轨迹（摘要化，替代全量 params/result 存储）
// ============================================================================

/// 紧凑持久化步骤：体积从平均 123KB/条降到 <8KB/条的关键。
/// params/result 只存截断摘要，构造时即截断，不存在"先存全量再截"的路径。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedStep {
    pub tool: String,
    /// 参数摘要（≤120 字符，构造时截断）
    pub params_summary: String,
    /// 结果摘要（≤300 字符，构造时截断）
    pub result_summary: String,
    pub success: bool,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

impl PersistedStep {
    /// 从运行时步骤数据构造紧凑步骤（转换时即截断）
    pub fn new(
        tool: String,
        params: &serde_json::Value,
        result: Option<&str>,
        success: bool,
        duration_ms: Option<u64>,
    ) -> Self {
        let params_raw = serde_json::to_string(params).unwrap_or_default();
        Self {
            tool,
            params_summary: truncate(&params_raw, 120),
            result_summary: truncate(result.unwrap_or(""), 300),
            success,
            duration_ms,
        }
    }
}

// ============================================================================
// MemoryEntry — full memory entry
// ============================================================================

/// Unified memory entry, replacing RawTimelineEntry + RawExecutionRecord
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String, // "{agent_type}-{turn_id}-{seq:03}" 或写入方自定义
    pub session_id: String,
    pub turn_id: String,
    pub sequence: u32,
    pub created_at: String, // ISO 8601
    pub wall_clock_ms: u64, // Unix timestamp in milliseconds
    pub agent_type: AgentType,
    /// 一维分类：这条记忆是什么（保留/索引/UI 策略的锚点）
    pub kind: MemoryKind,
    pub task_chain_id: Option<String>,
    pub chain_step: Option<u32>,
    pub goal_type: Option<String>, // GoalType id()，受 DB CHECK 白名单约束
    pub tags: Vec<String>,         // snake_case normalized + deduplicated
    pub intent: String,            // <= 200 chars (truncated, for indexing/retrieval)
    pub summary: String,           // <= 300 chars (truncated, for indexing/retrieval)
    /// Full user message (for history display, not truncated)
    #[serde(default)]
    pub user_message: String,
    /// Full assistant reply (for history display, not truncated)
    #[serde(default)]
    pub assistant_message: String,
    pub tools_used: Vec<String>,
    pub success: bool,
    /// 任务结果输出（写入前统一截断 ≤2000 字符，防止污染 FTS/BM25）
    pub output: Option<String>,
    pub artifacts: Vec<String>, // produced file paths etc.
    /// User flag (note/bookmark)
    #[serde(default)]
    pub is_marked: bool,
    /// 紧凑执行步骤（摘要化），最多保留最后 20 步
    #[serde(default)]
    pub execution_steps: Vec<PersistedStep>,
    pub parent_id: Option<String>,
    pub children_ids: Vec<String>,
    /// Checker-refined battle-tested pattern (scenario → steps → verification → counterexamples)
    #[serde(default)]
    pub pattern: Option<String>,
    /// Custom 会话归属卡片 id（None = 非 Custom 会话）。
    /// insert_entry 写入时自动从全局状态填充；检索时按此做双向隔离。
    #[serde(default)]
    pub custom_agent_id: Option<String>,
}

impl MemoryEntry {
    /// Create a minimal working entry (remaining fields filled by caller)
    pub fn new(
        id: String,
        session_id: String,
        turn_id: String,
        agent_type: AgentType,
        kind: MemoryKind,
    ) -> Self {
        let now = chrono::Utc::now();
        Self {
            id,
            session_id,
            turn_id,
            sequence: 0,
            created_at: now.to_rfc3339(),
            wall_clock_ms: now.timestamp_millis() as u64,
            agent_type,
            kind,
            task_chain_id: None,
            chain_step: None,
            goal_type: None,
            tags: vec![],
            intent: String::new(),
            summary: String::new(),
            user_message: String::new(),
            assistant_message: String::new(),
            tools_used: vec![],
            success: false,
            output: None,
            artifacts: vec![],
            is_marked: false,
            execution_steps: vec![],
            parent_id: None,
            children_ids: vec![],
            pattern: None,
            custom_agent_id: None,
        }
    }

    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.tools_used = tools;
        self
    }

    pub fn with_success(mut self, success: bool) -> Self {
        self.success = success;
        self
    }

    pub fn with_summary(mut self, summary: String) -> Self {
        self.summary = summary;
        self
    }
}

// ============================================================================
// Utility functions
// ============================================================================

/// Convert from lib.rs ExecutionStep to compact PersistedStep（截断在转换时完成）
pub fn from_lib_step(s: &crate::ExecutionStep) -> PersistedStep {
    let result_text = s
        .result
        .as_ref()
        .and_then(|r| r.output.clone().or_else(|| r.error.clone()));
    PersistedStep::new(
        s.tool.clone(),
        &s.params,
        result_text.as_deref(),
        matches!(s.status, crate::StepStatus::Success),
        None,
    )
}

/// Normalize tags: snake_case + deduplicate + filter empty strings
pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for tag in tags {
        let normalized = tag
            .trim()
            .to_lowercase()
            .replace([' ', '-'], "_")
            .replace("__", "_");
        if normalized.is_empty() || seen.contains(&normalized) {
            continue;
        }
        seen.insert(normalized.clone());
        result.push(normalized);
    }
    result
}

/// Build entry ID — 必须包含 session 维度，否则跨会话同 turn_id 会主键冲突
///（旧格式 `{agent}-{turn}-{seq}` 被新会话同 turn REPLACE 覆盖，历史对话静默丢失）。
/// session_id 取前 8 字符（uuid 前 8 位碰撞概率可忽略），保持 id 可读。
pub fn build_entry_id(agent_type: AgentType, session_id: &str, turn_id: &str, seq: u32) -> String {
    let sid: String = session_id.chars().take(8).collect();
    format!("{}-{}-{}-{:03}", agent_type.as_str(), sid, turn_id, seq)
}

/// 紧凑轨迹最多保留的步数（优先保留最后的步骤——结果通常在最后）
const MAX_PERSISTED_STEPS: usize = 20;

/// Build MemoryEntry (kind=task_trace) from Exec session compact steps
pub fn entry_from_exec_steps(
    session_id: &str,
    turn_id: &str,
    seq: u32,
    steps: &[PersistedStep],
    tools_used: Vec<String>,
    success: bool,
    error: Option<String>,
    goal_type: Option<String>,
    task_chain_id: Option<String>,
) -> MemoryEntry {
    // Extract meaningful content from step results (not listing tool names)
    let step_summary = steps
        .iter()
        .filter_map(|s| {
            let snippet: String = s.result_summary.chars().take(120).collect();
            if snippet.is_empty() {
                None
            } else {
                Some(format!("{}: {}", s.tool, snippet))
            }
        })
        .take(3)
        .collect::<Vec<_>>()
        .join("; ");
    let summary = if step_summary.is_empty() {
        if success { "completed" } else { "failed" }.to_string()
    } else {
        truncate(&step_summary, 300)
    };

    let mut tags = vec![
        goal_type.clone().unwrap_or_default(),
        if success { "success" } else { "failure" }.to_string(),
    ];
    tags.extend(tool_category_tags(&tools_used));
    let tags = normalize_tags(&tags);

    let mut entry = MemoryEntry::new(
        build_entry_id(AgentType::Exec, session_id, turn_id, seq),
        session_id.to_string(),
        turn_id.to_string(),
        AgentType::Exec,
        MemoryKind::TaskTrace,
    );
    entry.sequence = seq;
    entry.tools_used = tools_used;
    entry.success = success;
    entry.summary = summary;
    // 任务结果统一截断 ≤2000 字符（全量 output 是 FTS/BM25 污染源）
    entry.output = error
        .or_else(|| {
            steps
                .last()
                .map(|s| s.result_summary.clone())
                .filter(|r| !r.is_empty())
        })
        .map(|o| truncate(&o, 2000));
    entry.tags = tags;
    entry.goal_type = goal_type;
    entry.task_chain_id = task_chain_id;
    // 最多保留最后 20 步（结果/收敛信息通常集中在尾部）
    let keep_from = steps.len().saturating_sub(MAX_PERSISTED_STEPS);
    entry.execution_steps = steps[keep_from..].to_vec();
    entry
}

/// Extract category tags from tool name list (file_ops / system / desktop / memory / planner)
pub(crate) fn tool_category_tags(tools: &[String]) -> Vec<String> {
    let mut cats = Vec::new();
    for t in tools {
        if t.starts_with("file_") || t.starts_with("search_") {
            if !cats.contains(&"file_ops".to_string()) {
                cats.push("file_ops".to_string());
            }
        } else if t.starts_with("system_") || t.starts_with("process_") {
            if !cats.contains(&"system".to_string()) {
                cats.push("system".to_string());
            }
        } else if t.starts_with("desktop_") {
            if !cats.contains(&"desktop".to_string()) {
                cats.push("desktop".to_string());
            }
        } else if t.starts_with("memory_")
            || t.starts_with("timeline_")
            || t.starts_with("session_")
        {
            if !cats.contains(&"memory".to_string()) {
                cats.push("memory".to_string());
            }
        } else if t.starts_with("planner_") {
            if !cats.contains(&"planner".to_string()) {
                cats.push("planner".to_string());
            }
        } else if t.starts_with("browser_") && !cats.contains(&"browser".to_string()) {
            cats.push("browser".to_string());
        }
    }
    cats
}

pub fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
