//! types.rs — Nuphus Workflow data model
//!
//! Streamlined type system: removed multi-turn dialogue, multi-step state machines,
//! self-healing, and simulation types. Adopted "single-pass planning → deterministic execution."

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Workflow definition ──

/// Complete workflow definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    /// 创建时间；旧数据缺失时为 None，store 层 load 后补填
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    /// 更新时间；旧数据缺失时为 None，store 层 load 后补填
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    pub status: WorkflowStatus,
    pub steps: Vec<Step>,
    pub doc: Option<String>, // Plan document (embedded markdown)
    pub schedule: Option<ScheduleConfig>,
    /// 运行历史（最近在前，最多保留 10 条）
    #[serde(default)]
    pub run_history: Vec<RunRecord>,
    /// 工作流级超时（秒），None 无限制。超时后工作流整体失败
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// dry-run：仅编译校验，不执行步骤
    #[serde(default)]
    pub dry_run: bool,
}

impl Workflow {
    pub fn new(name: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
            status: WorkflowStatus::Draft,
            steps: Vec::new(),
            doc: None,
            schedule: None,
            run_history: Vec::new(),
            timeout_secs: None,
            dry_run: false,
        }
    }

    /// 获取最近一次运行记录
    pub fn last_run(&self) -> Option<&RunRecord> {
        self.run_history.first()
    }

    /// 获取最近一次运行记录（可变）
    pub fn last_run_mut(&mut self) -> Option<&mut RunRecord> {
        self.run_history.first_mut()
    }

    /// 推入新运行记录（保留最近 3 条）
    pub fn push_run(&mut self, run: RunRecord) {
        const MAX_HISTORY: usize = 3;
        self.run_history.insert(0, run);
        self.run_history.truncate(MAX_HISTORY);
    }
}

/// Workflow lifecycle state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkflowStatus {
    Draft,     // Plan generated
    Ready,     // Ready to execute (all steps prepared)
    Running,   // Executing
    Completed, // Execution complete
    Error,     // Error
}

// ── Visual anchors ──

/// Screenshot annotation anchor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualAnchor {
    pub id: String,
    pub screenshot_path: String,
    pub region: Rect,
    pub label: String,
    pub ocr_result: Option<String>,
    pub dict_config: Option<OcrDictConfig>,
}

/// Rectangle region (screenshot pixel coordinates)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Word-level dictionary for multi-match region recognition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrDictConfig {
    pub words: Vec<String>,
    #[serde(default = "default_true")]
    pub case_sensitive: bool,
    #[serde(default = "default_ocr_max_error")]
    pub max_error_rate: f64,
}

fn default_true() -> bool {
    true
}
fn default_ocr_max_error() -> f64 {
    0.1
}

// ── Scheduling ──

/// Cron-style schedule configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    /// Cron expression: "minute hour day month weekday"
    pub cron: String,
    /// Timezone (default "Asia/Shanghai")
    #[serde(default = "default_cron_tz")]
    pub timezone: String,
    /// Whether schedule is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Human-readable description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

fn default_cron_tz() -> String {
    "Asia/Shanghai".to_string()
}

// ── Run history ──

/// Single execution record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub status: RunStatus,
    /// Per-step records; empty until executor starts populating
    #[serde(default)]
    pub steps: Vec<StepRunRecord>,
    /// Top-level error message when workflow fails
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Snapshot of variables at end of run
    #[serde(default)]
    pub variables_snapshot: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RunStatus {
    Running,
    Success,
    Error(String),
    Cancelled,
    Paused,
}

/// Single step execution record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRunRecord {
    pub step_id: String,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub status: StepRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StepRunStatus {
    Running,
    Success,
    Skipped,
    Error(String),
}

// ── OnError ──

/// Per-step error handling strategy
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnError {
    /// 立即终止整个工作流（默认）
    #[default]
    Abort,
    /// 跳过该步骤继续执行
    Skip,
    /// 重试：max 次，间隔 backoff_ms 毫秒
    Retry {
        max: u32,
        #[serde(default = "default_backoff_ms")]
        backoff_ms: u64,
        #[serde(default)]
        backoff_multiplier: f64,
    },
    /// 仅允许特定退出码通过（非白名单码视为失败）
    AllowCodes {
        #[serde(default)]
        codes: Vec<i32>,
    },
}

fn default_backoff_ms() -> u64 {
    500
}

impl OnError {
    pub fn abort_default() -> Self {
        OnError::Abort
    }

    pub fn is_abort(&self) -> bool {
        matches!(self, OnError::Abort)
    }

    pub fn is_skip(&self) -> bool {
        matches!(self, OnError::Skip)
    }
}

// ── ConditionOp ──
// 仅数值比较操作（eval_condition 的 gt/lt/gte/lte 使用）；
// V1 的字符串比较操作已由 Condition untagged 12 变体承接，此处收敛为数值专用。

#[derive(Debug, Clone, Serialize, Deserialize, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOp {
    /// 数值比较：var > value（value 须可解析为 f64）
    Gt,
    Lt,
    Gte,
    Lte,
}

// ═══════════════════════════════════════════════════════════════
// ── V2 Schema-First Types (the canonical Step system) ──
// ═══════════════════════════════════════════════════════════════

// ── VarRef: variable reference or literal ──

/// Variable reference or literal value
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VarRef {
    /// {{var}}
    Var { var: String },
    /// Plain string literal (without {{}})
    Lit(String),
}

/// Unified condition expression.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Condition {
    Equals { equals: Vec<VarRef> },
    NotEquals { not_equals: Vec<VarRef> },
    Contains { contains: Vec<VarRef> },
    StartsWith { starts_with: Vec<VarRef> },
    Regex { regex: Vec<VarRef> },
    NotEmpty { not_empty: VarRef },
    Empty { empty: VarRef },
    Gt { gt: Vec<VarRef> },
    Lt { lt: Vec<VarRef> },
    Gte { gte: Vec<VarRef> },
    Lte { lte: Vec<VarRef> },
    Always { always: bool },
}

// ── LoopDef (new, from LoopV2) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub for_each: Option<ForEachDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<Condition>,
    #[serde(default = "default_max_loop")]
    pub max: u32,
    #[serde(rename = "do")]
    pub steps: Vec<Step>,
}

fn default_max_loop() -> u32 {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForEachDef {
    pub items: VarRef,
    #[serde(rename = "as")]
    pub item_var: String,
}

// ── IfDef (new, from IfV2) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IfDef {
    pub condition: Condition,
    #[serde(rename = "then")]
    pub then: Vec<Step>,
    #[serde(default, rename = "else")]
    pub else_branch: Vec<Step>,
}

// ── ChatOpts (new, from ChatWithV2) ──

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatOpts {
    /// 引用的 ChatAgentConfig ID（加载对应配置文件）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub screenshot: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<Vec<String>>,
    /// 内联模型：优先按 registry 模型 ID 路由专属 provider client；
    /// registry 无此 ID 时回退为裸模型名沿用主模型客户端（向后兼容）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 内联模型显示名称
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_display: Option<String>,
    /// 内联 temperature
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// 内联 max_tokens
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// 内联 system prompt 覆盖
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// 内联 persona（用户自定义人设）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    /// 内联 goal（任务目标覆盖）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// 内联 constraints（约束条件覆盖）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<Vec<String>>,
    /// 内联 requirements（操作规范覆盖）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirements: Option<Vec<String>>,
    /// 内联 max_iterations（ReAct 最大轮数）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<u32>,
}

// ── ScriptDef (new, from ScriptV2) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptDef {
    pub runtime: String,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

// ── AssertDef (new, from AssertV2) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssertDef {
    pub condition: Condition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// ── McpDef (new, from McpV2) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpDef {
    pub server: String,
    pub tool: String,
    #[serde(default)]
    pub with: serde_json::Value,
}

// ── Action: the "do" payload ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Action {
    Tool {
        tool: String,
        #[serde(default)]
        with: serde_json::Value,
    },
    Seq {
        seq: Vec<Step>,
    },
    Loop {
        #[serde(rename = "loop")]
        def: LoopDef,
    },
    If {
        #[serde(rename = "if")]
        def: IfDef,
    },
    Call {
        call: String,
        #[serde(default)]
        with: serde_json::Value,
    },
    Wait {
        wait: String,
        #[serde(default)]
        auto: Vec<Step>,
    },
    Chat {
        chat: String,
        #[serde(default)]
        with: ChatOpts,
    },
    Script {
        script: ScriptDef,
    },
    Assert {
        assert: AssertDef,
    },
    Mcp {
        mcp: McpDef,
    },
    Sleep {
        sleep: f64,
    },
    Break {
        #[serde(rename = "break")]
        _break: bool,
    },
    Continue {
        #[serde(rename = "continue")]
        _continue: bool,
    },
    Custom(serde_json::Value),
}

// ── Step: the unified step type ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "OnError::abort_default")]
    pub on_error: OnError,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(rename = "do")]
    pub action: Action,
}

impl Step {
    pub fn id(&self) -> String {
        self.id.clone()
    }

    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn kind_str(&self) -> &'static str {
        match &self.action {
            Action::Tool { .. } => "tool",
            Action::Seq { .. } => "seq",
            Action::Loop { .. } => "loop",
            Action::If { .. } => "if",
            Action::Call { .. } => "call",
            Action::Wait { .. } => "wait",
            Action::Chat { .. } => "chat_agent",
            Action::Script { .. } => "script",
            Action::Assert { .. } => "assert",
            Action::Mcp { .. } => "mcp",
            Action::Sleep { .. } => "tool",
            Action::Break { .. } => "break",
            Action::Continue { .. } => "continue",
            Action::Custom(_) => "tool",
        }
    }

    pub fn on_error_strategy(&self) -> &OnError {
        &self.on_error
    }

    /// Convenience: create a Seq container step
    pub fn new_seq(id: &str, name: &str, children: Vec<Step>) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            on_error: OnError::Abort,
            capture: None,
            timeout_secs: None,
            action: Action::Seq { seq: children },
        }
    }
}

impl Default for Step {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            on_error: OnError::Abort,
            capture: None,
            timeout_secs: None,
            action: Action::Custom(serde_json::Value::Null),
        }
    }
}

// ── WorkflowSummary type ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSummary {
    pub id: String,
    pub name: String,
    pub status: WorkflowStatus,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub step_count: usize,
    pub last_run: Option<DateTime<Utc>>,
    pub last_status: Option<RunStatus>,
}

impl From<&Workflow> for WorkflowSummary {
    fn from(wf: &Workflow) -> Self {
        let last = wf.run_history.first();
        Self {
            id: wf.id.clone(),
            name: wf.name.clone(),
            status: wf.status.clone(),
            created_at: wf.created_at,
            updated_at: wf.updated_at,
            step_count: wf.steps.len(),
            last_run: last.and_then(|r| r.finished_at),
            last_status: last.map(|r| r.status.clone()),
        }
    }
}
