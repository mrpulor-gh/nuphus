//! Nuphus - 协同共生桌面助手
//!
//! 核心架构：Agent + Memory(进化支点) + Tools(可插拔)
//!
//! 设计原则：
//! 1. 核心类型定义在 lib.rs
//! 2. 业务逻辑在各子模块
//! 3. 不用 dyn Trait，用具体类型

pub mod agent;
pub mod annotation;
pub mod api;
pub mod browser;
pub mod cache;
pub mod config;
pub mod cookies;
pub mod custom_agents;
pub mod desktop;
pub mod embed;
pub mod filter;
pub mod handoff;
pub mod hooks;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod mobile_append;
pub mod permissions;
pub mod render_bridge;
pub mod runtime;
pub mod security;
pub mod segmenter;
pub mod session;
pub mod skill;
pub mod state;
pub mod store;
pub mod tools;
pub mod transports;
pub mod utils;
pub mod video_bridge;
pub mod workflow;

#[cfg(test)]
mod test_helpers;

// ============================================================================
// 核心数据类型
// ============================================================================

/// 工具调用
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub tool: String,
    pub params: serde_json::Value,
}

impl ToolCall {
    pub fn new(tool: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            tool: tool.into(),
            params,
        }
    }
}

/// 工具执行结果 - 统一格式
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    /// 进程退出码（system_shell 等工具设置，供 AllowCodes 处理用）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

impl ToolResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: Some(output.into()),
            error: None,
            exit_code: Some(0),
        }
    }

    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: None,
            error: Some(error.into()),
            exit_code: None,
        }
    }

    /// 将 ToolResult 转为工具回调所需的 Result<String, String>。
    /// 失败时若含 exit_code，编码到错误字符串中以供下游 AllowCodes 解析。
    pub fn into_exec_result(self) -> std::result::Result<String, String> {
        if self.success {
            Ok(self.output.unwrap_or_default())
        } else {
            let msg = self.error.unwrap_or_else(|| "Unknown error".to_string());
            Err(match self.exit_code {
                Some(code) => format!("__EXIT_CODE:{}__ {}", code, msg),
                None => msg,
            })
        }
    }
}

/// Agent 输出
#[derive(Debug)]
pub struct AgentOutput {
    pub message: String,
    pub success: bool,
    pub steps: Vec<ExecutionStep>,
    /// LLM 请求失败时的可重试会话（session JSON）
    pub retry_session: Option<String>,
}

/// 执行步骤
#[derive(Debug, Clone)]
pub struct ExecutionStep {
    pub tool: String,
    pub params: serde_json::Value,
    pub result: Option<ToolResult>,
    pub status: StepStatus,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// GoalType 分类标签(dispatch 时写入,用于记忆分类检索)
    pub goal_type: Option<String>,
    /// Leader 调用此工具前的推理链
    pub reasoning: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepStatus {
    Pending,
    Running,
    Success,
    Error,
    Retry,
}

// ============================================================================
// 错误类型
// ============================================================================

/// Agent-level structured errors.
///
/// Replaces the opaque `Agent(String)` variant with typed errors
/// that can be matched on (e.g. NotFound vs Cancelled).
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Not found: {what} '{id}'")]
    NotFound { what: String, id: String },

    #[error("Missing dependency: {dep}")]
    MissingDependency { dep: String },

    #[error("Workflow validation failed: {errors}")]
    ValidationFailed { errors: String },

    #[error("Workflow '{name}' failed: {error}")]
    WorkflowFailed { name: String, error: String },

    #[error("Safety halt after {consecutive_failures} consecutive failures")]
    SafetyHalt { consecutive_failures: u32 },

    #[error("Ambiguous id '{id}' matches {count} items")]
    AmbiguousId { id: String, count: usize },

    #[error("Cancelled")]
    Cancelled,

    #[error("Model switch failed: {error}")]
    ModelSwitchFailed { error: String },

    #[error("Scheduling conflict: {reason}")]
    SchedulingConflict { reason: String },

    #[error("{0}")]
    Other(String),
}

impl From<String> for AgentError {
    fn from(s: String) -> Self {
        AgentError::Other(s)
    }
}

/// LLM transport-level structured errors.
///
/// Distinguishes user cancellation from API errors from network failures.
#[derive(Debug, thiserror::Error)]
pub enum LLMError {
    #[error("Request cancelled by user")]
    Cancelled,

    #[error("API error {status}: {body}")]
    ApiError { status: u16, body: String },

    #[error("Request failed: {error}")]
    RequestFailed { error: String },

    #[error("Model not found: '{model_id}'")]
    ModelNotFound { model_id: String },

    #[error("Retry loop exhausted: {last_error}")]
    RetryLoopExhausted { last_error: String },

    #[error("Stream error: {error}")]
    StreamError { error: String },

    #[error("Failed to build HTTP client: {error}")]
    HttpBuildFailed { error: String },

    #[error("Failed to read response body: {error}")]
    ReadResponseFailed { error: String },

    #[error("{0}")]
    Other(String),
}

impl From<String> for LLMError {
    fn from(s: String) -> Self {
        LLMError::Other(s)
    }
}

/// Store/database-level structured errors.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Database connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Query failed: {0}")]
    QueryFailed(String),

    #[error("Migration failed: {0}")]
    MigrationFailed(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("{0}")]
    Other(String),
}

impl From<String> for StoreError {
    fn from(s: String) -> Self {
        StoreError::Other(s)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NuphusError {
    #[error("Desktop error: {0}")]
    Desktop(#[from] desktop_api::DesktopError),

    #[error("Agent error: {0}")]
    Agent(#[from] AgentError),

    #[error("LLM error: {0}")]
    LLM(#[from] LLMError),

    #[error("Memory error: {0}")]
    Memory(String),

    #[error("Tool error: {0}")]
    Tool(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Store error: {0}")]
    Store(#[from] StoreError),
}

impl NuphusError {
    /// Convenience: create Agent error from a string (goes into `AgentError::Other`).
    /// Prefer using `AgentError::NotFound` etc. for structured errors.
    pub fn agent(s: impl Into<String>) -> Self {
        NuphusError::Agent(AgentError::Other(s.into()))
    }
    /// Convenience: create LLM error from a string.
    pub fn llm(s: impl Into<String>) -> Self {
        NuphusError::LLM(LLMError::Other(s.into()))
    }
    /// Convenience: create Store error from a string.
    pub fn store(s: impl Into<String>) -> Self {
        NuphusError::Store(StoreError::Other(s.into()))
    }
}

/// 允许 `?` 直接传播 String 错误（映射到 Tool 变体）——Linux 窗口管理等多处使用。
impl From<String> for NuphusError {
    fn from(s: String) -> Self {
        NuphusError::Tool(s)
    }
}

impl serde::Serialize for NuphusError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

// ============================================================================
// Result 别名
// ============================================================================

pub type Result<T> = std::result::Result<T, NuphusError>;

// ============================================================================
// 导出子模块的类型
// ============================================================================

pub use agent::goal_types::GoalType;
pub use agent::{AgentConfig, ReactAgent};
pub use api::{ApiClient, ProviderKind};
pub use permissions::{PermissionOutcome, PermissionPolicy, ToolCategory, ToolPermissions};
pub use runtime::Mode;
pub use session::{ContentBlock, Message, MessageRole, Session, TokenUsage};
pub use tools::ToolRegistry;

pub use llm::LlmClient;
