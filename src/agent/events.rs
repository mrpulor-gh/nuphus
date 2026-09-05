//! NuphusEvent — Typed frontend event protocol
//!
//! Replaces the old ExecutionLineType string events.
//! Single `nuphus-event` Tauri event, distinguishing variants via serde tagged enum.
//! Frontend no longer needs to parse colon-separated strings to extract tool names/parameters.

use serde::{Deserialize, Serialize};

/// Unified type for all frontend events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NuphusEvent {
    // ── Execution panel events ──
    /// Execution window opened, start executing a STEP
    ExecutionStarted {
        step_index: usize,
        goal: String,
        tools: Vec<String>,
        /// Message source: "desktop" | "mobile"
        source: String,
        /// Current running mode: "leader" | "workflow"
        #[serde(default)]
        mode: String,
    },

    /// Tool call start (from_task: true when executed by ExecuteAgent, for frontend to distinguish)
    ToolCallStart {
        call_id: String,
        tool_name: String,
        params: serde_json::Value,
        iteration: u32,
        from_task: bool,
    },

    /// Single line output during tool execution (streamed line by line, sent in real-time during execution)
    ToolOutputLine {
        call_id: String,
        line: String,
        is_stderr: bool,
    },

    /// Tool call end (from_task: true when executed by ExecuteAgent, for frontend to distinguish)
    ToolCallEnd {
        call_id: String,
        tool_name: String,
        success: bool,
        duration_ms: u64,
        output_preview: String,
        output_full_size: usize,
        is_truncated: bool,
        error: Option<String>,
        from_task: bool,
    },

    /// LLM text delta (typewriter effect)
    LlmTextDelta {
        text: String,
        is_thinking: bool,
        /// true when emitted by ExecAgent (subtask); frontend skips chat bubble render
        from_task: bool,
    },

    /// Model-native image generation output URL
    ImageGenerated { url: String },

    /// Execution progress update
    ExecutionProgress {
        iteration: u32,
        max_iterations: u32,
        tool_calls_so_far: usize,
    },

    /// STEP execution completed
    ExecutionCompleted {
        step_index: usize,
        output: StepOutput,
        /// Total execution duration (ms)
        total_duration_ms: u64,
        /// Total actual calls
        total_calls: usize,
    },

    /// STEP execution error
    ExecutionError { step_index: usize, error: String },

    // ── Task decomposition events ──
    /// A decomposed task has started execution
    TaskStarted {
        task_id: usize,
        total_tasks: usize,
        description: String,
    },

    /// A decomposed task has completed execution
    TaskCompleted {
        task_id: usize,
        total_tasks: usize,
        success: bool,
        description: String,
        summary: String,
    },

    /// Task list (pushed after planning, for TaskBubble display)
    TaskList {
        plan_path: String,
        tasks: Vec<TaskItem>,
    },

    // ── Evolution events (received by both views) ──
    /// New evolution seed generated
    SeedGenerated {
        seed_id: String,
        seed_type: String,
        summary: String,
    },

    // ── Safety events ──
    /// User paused execution (waiting for continue/append/terminate decision)
    ExecutionPaused { action_id: String },

    /// Dangerous operation requiring user confirmation
    SecurityCheck {
        action_id: String,
        tool: String,
        params: String,
        risk: RiskLevel,
        reason: String,
    },

    /// Agent requests user input (e.g. API key, password, choice, screenshot, coordinates)
    UserInputRequest {
        action_id: String,
        title: String,
        prompt: String,
        sensitive: bool,
        input_type: String,
        // ── icon_confirm 专用字段 ──
        #[serde(skip_serializing_if = "Option::is_none")]
        icon_path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        default_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        default_shortcut: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rel_x: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rel_y: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        default_note: Option<String>,
        // ── step_form 专用字段（全 optional，不破坏既有 text/icon_confirm/capture 路径）──
        #[serde(skip_serializing_if = "Option::is_none")]
        default_stage: Option<String>,
    },

    /// 安全确认/输入请求弹窗已超时（或等待被取消），前端应清除对应 action_id 的弹窗
    PromptTimeout { action_id: String },

    // ── Reminder events ──
    /// Persistent reminder injected (frontend displays reminder status)
    AgentReminder {
        kind: String,
        count: u32,
        max_count: u32,
        text: String,
    },

    // ── General ──
    /// User message received (from desktop or mobile)
    /// images: 图片 data URL 列表（已冻结 PNG），供前端 user 消息渲染
    UserMessageReceived {
        content: String,
        source: String,
        #[serde(default)]
        images: Vec<String>,
    },

    /// Plain text message (main view conversation)
    DirectResponse { message: String },

    /// System warning
    Warning { code: String, message: String },

    /// System error
    Error {
        code: String,
        message: String,
        retryable: bool,
        /// From subtask (dispatch Exec), frontend should not reset isProcessing
        #[serde(default)]
        from_subtask: bool,
    },

    /// Goal type identification result (sent at execution start)
    GoalTypeIdentified {
        goal_type: String,
        label: String,
        confidence: f32,
        max_iterations: usize,
    },

    /// "Understand first" phase complete (sent after goal type identification, before execution)
    UnderstandingComplete {
        summary: String,
        critiques: Vec<String>,
        needs_clarification: bool,
        confidence: f32,
    },

    /// Session lifecycle
    SessionInfo {
        session_id: String,
        model: String,
        timestamp: u64,
    },

    /// 运行模式切换（空闲态 set_mode 时广播，手机端同步「当前模式」）
    ModeChanged { mode: String },

    /// LLM token usage (sent after each LLM call)
    /// source: "main" = main window session / "exec" = execution window
    TokenUsage {
        input_tokens: u32,
        output_tokens: u32,
        cache_hit_tokens: u32,
        source: String,
    },

    // ── Context refinement events ──
    /// Context usage exceeds threshold, ask user whether to refine.
    /// If forced=true, backend has already decided to refine (auto-mode).
    RefinePrompt {
        current_tokens: u32,
        refine_limit: u32,
        force_limit: u32,
        threshold: f64,
        context_window: u32,
        /// true = forced refine (backend decided, frontend should auto-execute)
        forced: bool,
    },

    /// Context refinement in progress (user confirmed or forced refinement)
    RefineExecuting,

    /// 用户跳过提炼（一端跳过，广播双端同步关闭弹窗，防状态残留）
    RefineSkipped,

    /// Session refined (continues in same session after refinement)
    SessionRefined {
        summary: String,
        message_count: usize,
        /// Session ID for lookup / cross-referencing
        session_id: String,
    },

    /// 提炼失败（LLM 调用失败/超时/空摘要）：与 RefineExecuting 配对的结束事件。
    /// 缺失它会让双端"提炼中"UI（桌面弹窗 spinner / 手机提炼卡片）永久卡死。
    RefineFailed { message: String },

    // ── HUD Overlay events ──
    /// Update HUD overlay text and phase
    HudUpdate {
        text: String,
        phase: String, // "running" | "done" | "error" | "workflow" | "workflow_wait"
        #[serde(skip_serializing_if = "Option::is_none")]
        step_kind: Option<String>, // "tool" | "wait" | "chat_agent" | "call" | "script" | "seq" | "loop" | "if"
    },

    /// Leader execution round completed — trigger main window focus.
    /// Emitted ONLY by react_loop when the Leader finishes, NOT per-tool.
    LeaderDone { message: String },

    // ── 会话镜像（手机跟随桌面当前视图）──
    /// 当前会话已切换（桌面 rail 或手机遥控任一路径触发）。
    /// 手机��收到后重拉 /history 即呈现桌面当前会话——手机不维护独立会话状态。
    SessionChanged {
        /// 切换后的当前会话 id
        session_id: String,
    },

    // ── 展示台列表变化（手机跟随桌面会话清单）──
    /// 会话清单结构变化（手动归档 / 重命名）——当前会话未变，手机端只需刷新
    /// 会话清单，不重拉历史、不弹跟随提示。
    ShelfUpdated,

    // ── 新建对话意图（手机遥控 → 双端回 welcome）──
    /// 新建对话纯视图意图广播：后端**不创建/不切换任何 session**（无空会话逻辑，
    /// 会话只在 welcome 直发消息时 force_new 创建）。手机 /new-chat 触发；
    /// 桌面端收到后执行本地 handleNewChat（清聊天区回欢迎页，执行中自守卫）；
    /// 手机端本机发送前已先行清视图（幂等）。
    NewChatBroadcast,
}

// ── Helper types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskItem {
    pub id: usize,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepInfo {
    pub index: usize,
    pub goal: String,
    pub tools: Vec<String>,
    pub success_criteria: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepOutput {
    pub step_index: usize,
    pub result_message: String,
    pub artifacts: Vec<String>, // Output file paths
    pub tool_calls_count: usize,
}

/// Risk level — re-exported from api::types for backward compatibility
pub use crate::api::types::RiskLevel;

// ── EventEmitter trait ──

/// Event emitter trait.
/// Implemented by the Tauri bridge layer, Agent sends typed events to frontend through this trait.
pub trait EventEmitter: Send + Sync + 'static {
    fn emit(&self, event: NuphusEvent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_step_info_serialization() {
        let info = StepInfo {
            index: 0,
            goal: "读取 Cargo.toml".to_string(),
            tools: vec!["Read".to_string()],
            success_criteria: "获取项目依赖列表".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: StepInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.index, 0);
        assert_eq!(parsed.goal, "读取 Cargo.toml");
        assert_eq!(parsed.tools, vec!["Read"]);
        assert_eq!(parsed.success_criteria, "获取项目依赖列表");
    }

    #[test]
    fn test_step_status_serialization() {
        assert_eq!(
            serde_json::to_string(&WorkflowStepStatus::Pending).unwrap(),
            r#""pending""#
        );
        assert_eq!(
            serde_json::to_string(&WorkflowStepStatus::Running).unwrap(),
            r#""running""#
        );
        let parsed: WorkflowStepStatus = serde_json::from_str(r#""completed""#).unwrap();
        assert_eq!(parsed, WorkflowStepStatus::Completed);
    }

    #[test]
    fn test_risk_level_serialization() {
        assert_eq!(
            serde_json::to_string(&RiskLevel::High).unwrap(),
            r#""high""#
        );
        let parsed: RiskLevel = serde_json::from_str(r#""critical""#).unwrap();
        assert_eq!(parsed, RiskLevel::Critical);
    }

    #[test]
    fn test_direct_response_event_roundtrip() {
        let event = NuphusEvent::DirectResponse {
            message: "你好！".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: NuphusEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            NuphusEvent::DirectResponse { message } => assert_eq!(message, "你好！"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_error_event_roundtrip() {
        let event = NuphusEvent::Error {
            code: "plan_failed".to_string(),
            message: "LLM 超时".to_string(),
            retryable: true,
            from_subtask: false,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: NuphusEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            NuphusEvent::Error {
                code,
                message,
                retryable,
                from_subtask,
            } => {
                assert_eq!(code, "plan_failed");
                assert_eq!(message, "LLM 超时");
                assert!(retryable);
                assert!(!from_subtask);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_security_check_event_roundtrip() {
        let event = NuphusEvent::SecurityCheck {
            action_id: "a1".to_string(),
            tool: "system_shell".to_string(),
            params: r#"{"command":"rm -rf C:\\"}"#.to_string(),
            risk: RiskLevel::Critical,
            reason: "危险操作".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: NuphusEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            NuphusEvent::SecurityCheck {
                action_id, risk, ..
            } => {
                assert_eq!(action_id, "a1");
                assert_eq!(risk, RiskLevel::Critical);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_user_input_request_roundtrip() {
        let event = NuphusEvent::UserInputRequest {
            action_id: "a1".to_string(),
            title: "API Key".to_string(),
            prompt: "请输入你的 DeepSeek API Key".to_string(),
            sensitive: true,
            input_type: "text".to_string(),
            icon_path: None,
            default_name: None,
            default_shortcut: None,
            rel_x: None,
            rel_y: None,
            default_note: None,
            default_stage: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: NuphusEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            NuphusEvent::UserInputRequest {
                action_id,
                title,
                prompt: _,
                sensitive,
                ..
            } => {
                assert_eq!(action_id, "a1");
                assert_eq!(title, "API Key");
                assert!(sensitive);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_all_variants_have_type_tag() {
        let variants = vec![
            serde_json::to_value(NuphusEvent::DirectResponse {
                message: "x".into(),
            })
            .unwrap(),
            serde_json::to_value(NuphusEvent::SessionChanged {
                session_id: "s1".into(),
            })
            .unwrap(),
            serde_json::to_value(NuphusEvent::ExecutionStarted {
                step_index: 0,
                goal: "x".into(),
                tools: vec![],
                source: "desktop".into(),
                mode: "leader".into(),
            })
            .unwrap(),
            serde_json::to_value(NuphusEvent::ExecutionCompleted {
                step_index: 0,
                output: StepOutput {
                    step_index: 0,
                    result_message: "x".into(),
                    artifacts: vec![],
                    tool_calls_count: 0,
                },
                total_duration_ms: 0,
                total_calls: 0,
            })
            .unwrap(),
            serde_json::to_value(NuphusEvent::SessionRefined {
                summary: "x".into(),
                message_count: 5,
                session_id: "s1".into(),
            })
            .unwrap(),
            serde_json::to_value(NuphusEvent::HudUpdate {
                text: "x".into(),
                phase: "running".into(),
                step_kind: None,
            })
            .unwrap(),
        ];
        for v in &variants {
            assert!(v.get("type").is_some(), "missing type tag in {:?}", v);
        }
    }
}
