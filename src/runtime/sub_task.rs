//! SubTaskRunner — Free ReACT executor
//!
//! Migrated from the dual-Agent architecture's ExecuteAgent, serving as Runtime's internal execution engine.
//!
//! Key differences from ReactAgent:
//! - Does not check wants_file_output, does not force_output (architecture itself prevents talk-only)
//! - memory::search / memory::query as tools callable by LLM
//!
//! ## File split
//!
//! - `sub_task.rs` — Struct definitions, construction, setters, safety checks, context queries, tests
//! - `sub_task_loop.rs` — `run_free()` main loop + private helper methods
//! - `sub_task_shell.rs` — Shell streaming execution + tool execution (free functions)

use crate::agent::events::{EventEmitter, NuphusEvent};
use crate::agent::goal_types::GoalType;
use crate::agent::pause::PauseDecision;
use crate::agent::reminders::ReminderQueue;
use crate::runtime::protection::ProtectionGuard;
use crate::{
    permissions::ToolPermissions,
    session::{ContentBlock, Session},
    tools::ToolRegistry,
    ToolCall, ToolResult,
};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// SubTaskRunner configuration
pub struct SubTaskConfig {
    pub max_iterations: usize,
    /// Model context window size (for byte estimation), default 128000
    pub context_window: usize,
}

impl Default for SubTaskConfig {
    fn default() -> Self {
        Self {
            max_iterations: GoalType::MAX_ITERATIONS,
            context_window: 128000,
        }
    }
}

/// Free ReACT executor (for subtasks)
pub struct SubTaskRunner {
    pub(crate) llm: Arc<dyn ApiClient>,
    pub(crate) tools: ToolRegistry,
    pub(crate) tool_permissions: ToolPermissions,
    pub(crate) config: SubTaskConfig,
    pub(crate) session: Session,
    pub(crate) event_emitter: Option<Arc<dyn EventEmitter>>,
    pub(crate) reminders: ReminderQueue,
    pub(crate) step_prompt: String,
    /// Task title (user's original input, displayed in execution window)
    pub(crate) goal: String,
    /// Suppress Error events in silent mode (for retry use)
    pub(crate) suppress_error_events: bool,
    /// Suppress ExecutionStarted/Completed/Error lifecycle events in silent mode
    /// (used by task dispatch, dispatch layer manages lifecycle uniformly)
    pub(crate) suppress_lifecycle_events: bool,
    // Execution statistics
    /// Total execution duration timer
    pub(crate) execution_started_at: std::time::Instant,
    /// Actual total tool calls (for progress events)
    pub(crate) tool_call_total_count: u32,
    // Pause flag (set externally, polled between iterations)
    pub(crate) pause_flag: Option<Arc<AtomicBool>>,
    // Protection detection state
    pub(crate) protection: ProtectionGuard,
    // Marks whether this SubTaskRunner is a subtask of task::dispatch
    /// When true, frontend shows "task:" prefix to distinguish from Leader direct operation
    pub(crate) is_task: bool,
    // Iteration progressive warning level (0=none, 1=remind, 2=emphasis, 3=redline, 4=forbid)
    pub(crate) max_warning_injected: usize,
    // Circuit breaker (safety hard breaker)
    /// Consecutive safety check failure count (permission/Tenet/SecurityGuard), >= 3 triggers hard breaker
    pub(crate) safety_consecutive_failures: u32,
    /// User message (for keyword extraction in deviation summary)
    pub(crate) user_message: String,
    // Goal type (for tool whitelist filtering)
    pub(crate) goal_type: Option<GoalType>,
    // Pending system message buffer to flush
    pub(crate) pending_warnings: Vec<String>,
    // Stable prompt prefix during dispatch (without reminders), improves API cache hit rate
    pub(crate) cached_base_prompt: Option<String>,
    // Stable tool schema during dispatch (avoids cache mismatch from regeneration)
    pub(crate) cached_tools: Option<Vec<crate::api::ToolDefinition>>,
    // Flags: user terminated via pause→terminate, signal to Leader to stop
    pub(crate) user_terminated: bool,
    /// 主模型是否原生支持视觉（来自 ModelDef.supports_vision）。
    /// 决定 to_api_messages 是否以 image_url 直发主模型；supports_vision=false 时
    /// 图片保存为临时 BMP + 路径注入（Agent 按需调 desktop_vision 查看），禁止把 image_url 发给不支持视觉的主模型。
    pub(crate) supports_vision: bool,
    /// 同 turn 内已完成任务的最终回复（用于上下文压缩）
    pub(crate) turn_replies: Vec<String>,
    /// 下次 dispatch 前是否需要压缩上下文
    pub(crate) needs_compress: bool,
    /// Leader session ID (for causal chain linking in memory entries)
    pub(crate) leader_session_id: Option<String>,
    /// Leader turn ID (for causal chain linking in memory entries)
    pub(crate) leader_turn_id: Option<String>,
}

use crate::api::ApiClient;

impl SubTaskRunner {
    /// Create free ReACT executor (no step constraints)
    /// `goal` is the user's original input, displayed as task title in execution window
    pub fn new_free(
        llm: Arc<dyn ApiClient>,
        tools: ToolRegistry,
        system_prompt: String,
        goal: String,
    ) -> Self {
        let supports_vision = Self::resolve_supports_vision(llm.model_name());
        Self {
            llm,
            tools,
            tool_permissions: ToolPermissions::none(),
            config: SubTaskConfig::default(),
            session: Session::new(),
            event_emitter: None,
            reminders: ReminderQueue::new(),
            step_prompt: system_prompt,
            goal: goal.clone(),
            suppress_error_events: false,
            suppress_lifecycle_events: false,
            execution_started_at: std::time::Instant::now(),
            tool_call_total_count: 0,
            pause_flag: None,
            protection: ProtectionGuard::new(),
            is_task: false,
            max_warning_injected: 0,
            safety_consecutive_failures: 0,
            goal_type: None,
            pending_warnings: Vec::new(),
            cached_base_prompt: None,
            cached_tools: None,
            user_message: goal,
            user_terminated: false,
            supports_vision,
            turn_replies: Vec::new(),
            needs_compress: false,
            leader_session_id: None,
            leader_turn_id: None,
        }
    }

    /// 从 model registry 解析主模型是否原生支持视觉
    fn resolve_supports_vision(model_name: &str) -> bool {
        crate::config::load_registry()
            .ok()
            .and_then(|r| r.find_model(model_name).map(|(_, m)| m.supports_vision))
            .unwrap_or(false)
    }

    /// Set tool permissions
    pub fn set_tool_permissions(&mut self, perms: ToolPermissions) {
        self.tool_permissions = perms;
    }

    /// Set whether to suppress error events (suppress intermediate failure Error events on retry)
    pub fn set_suppress_error_events(&mut self, suppress: bool) {
        self.suppress_error_events = suppress;
    }

    /// Set whether to suppress lifecycle events (ExecutionStarted/Completed/Error).
    /// Used by task dispatch layer, dispatch layer manages lifecycle events itself.
    pub fn set_suppress_lifecycle_events(&mut self, suppress: bool) {
        self.suppress_lifecycle_events = suppress;
    }

    /// Set max iterations
    pub fn set_max_iterations(&mut self, max: usize) {
        self.config.max_iterations = max;
    }

    /// Set model context window size (for 60% distillation threshold)
    pub fn set_context_window(&mut self, window: usize) {
        self.config.context_window = window;
    }

    /// Set pause flag (set externally after user clicks pause button)
    pub fn set_pause_flag(&mut self, flag: Arc<AtomicBool>) {
        self.pause_flag = Some(flag);
    }

    pub fn set_event_emitter(&mut self, emitter: Arc<dyn EventEmitter>) {
        self.event_emitter = Some(emitter);
    }

    /// Mark this SubTaskRunner as a subtask of task::dispatch (frontend shows "task:" prefix)
    pub fn set_task_mode(&mut self, is_task: bool) {
        self.is_task = is_task;
    }

    /// Set session context (multi-turn conversation history)
    pub fn set_session(&mut self, session: Session) {
        self.session = session;
    }

    /// Set goal type (for tool whitelist filtering)
    pub fn set_goal_type(&mut self, goal_type: GoalType) {
        self.goal_type = Some(goal_type);
    }

    /// Prepare to reuse this Runner for a new task
    pub fn prepare_for_next_task(&mut self, new_goal: String) {
        self.goal = new_goal.clone();
        self.user_message = new_goal;
        self.execution_started_at = std::time::Instant::now();
        self.safety_consecutive_failures = 0;
        self.max_warning_injected = 0;
        self.pending_warnings.clear();
        self.user_terminated = false;
    }

    /// Inject structured bridge data into predecessor task (enters session as user message)
    pub fn inject_bridge_data(&mut self, data: &str) {
        self.session.push_user(data.to_string());
    }

    /// Get actual total tool calls
    pub fn total_tool_calls(&self) -> u32 {
        self.tool_call_total_count
    }

    /// Export current session JSON (for retry recovery)
    pub fn save_session_json(&self) -> Option<String> {
        serde_json::to_string(&self.session).ok()
    }

    pub(crate) fn emit(&self, event: NuphusEvent) {
        if let Some(ref emitter) = self.event_emitter {
            emitter.emit(event);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Pause + Safety check
// ═══════════════════════════════════════════════════════════════════

impl SubTaskRunner {
    /// Pause polling: wait for user to make Continue/Append/Terminate decision via frontend dialog
    pub(crate) async fn wait_for_pause_decision(
        &self,
        action_id: &str,
        cancel_flag: &AtomicBool,
    ) -> PauseDecision {
        crate::agent::pause::wait_for_pause_decision_global(
            self.tools.signals(),
            action_id,
            cancel_flag,
        )
        .await
    }

    /// Safety check: session authorization → permissions → SecurityGuard
    /// Returns None for safe to execute, Some(ToolResult) for intercepted/denied
    pub(crate) async fn check_tool_safety(
        &mut self,
        call: &ToolCall,
        cancel_flag: &AtomicBool,
    ) -> Option<ToolResult> {
        let mut policy = crate::permissions::PermissionPolicy::new(self.tool_permissions);
        policy = policy.with_categories(self.tools.all_tool_categories());

        let emitter: Option<&dyn crate::agent::events::EventEmitter> =
            self.event_emitter.as_deref();

        crate::agent::exec_tool::check_tool_security(
            self.tools.signals(),
            &call.tool,
            &call.params,
            &policy,
            emitter,
            cancel_flag,
            Some(&self.tool_permissions),
            &mut self.safety_consecutive_failures,
            &mut self.pending_warnings,
        )
        .await
    }
}

// ═══════════════════════════════════════════════════════════════════
// Context queries
// ═══════════════════════════════════════════════════════════════════

impl SubTaskRunner {
    /// Get execution context: (execution_session_id, steps_summary)
    pub fn get_execution_context(&self) -> (String, Vec<String>) {
        let mut result_map: std::collections::HashMap<String, (String, bool)> =
            std::collections::HashMap::new();
        for msg in self.session.messages() {
            for block in &msg.content {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } = block
                {
                    result_map.insert(tool_use_id.clone(), (content.clone(), *is_error));
                }
            }
        }
        let mut steps: Vec<String> = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();
        for msg in self.session.messages() {
            for block in &msg.content {
                if let ContentBlock::ToolUse {
                    id, name, input, ..
                } = block
                {
                    if seen_ids.contains(id) {
                        continue;
                    }
                    seen_ids.insert(id.clone());
                    let params = crate::agent::common::summarize_tool_params(input);
                    let (status, result_preview) =
                        if let Some((content, is_error)) = result_map.get(id) {
                            let preview: String = content.chars().take(200).collect();
                            let status = if *is_error { "✗" } else { "✓" };
                            if preview.is_empty() {
                                (status.to_string(), String::new())
                            } else {
                                (status.to_string(), format!(" → {}", preview))
                            }
                        } else {
                            ("✗".to_string(), String::new())
                        };
                    if params.is_empty() {
                        steps.push(format!("{}{} {}", name, result_preview, status));
                    } else {
                        steps.push(format!("{}({}){} {}", name, params, result_preview, status));
                    }
                }
            }
        }
        (self.session.id.clone(), steps)
    }

    /// Extract full tool call history (with params and results)
    pub fn format_tool_history(&self) -> String {
        let mut result_map: std::collections::HashMap<String, (String, bool)> =
            std::collections::HashMap::new();
        for msg in self.session.messages() {
            for block in &msg.content {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } = block
                {
                    result_map.insert(tool_use_id.clone(), (content.clone(), *is_error));
                }
            }
        }
        let mut seen_ids = std::collections::HashSet::new();
        let mut entries: Vec<String> = Vec::new();
        for msg in self.session.messages() {
            for block in &msg.content {
                if let ContentBlock::ToolUse { id, name, input } = block {
                    if seen_ids.contains(id) {
                        continue;
                    }
                    seen_ids.insert(id.clone());
                    let params_str = serde_json::to_string_pretty(input).unwrap_or_default();
                    let mut entry = format!("[{}]\n参数:\n{}", name, params_str);
                    if let Some((content, is_error)) = result_map.get(id) {
                        let preview: String = content.chars().take(3000).collect();
                        if *is_error {
                            entry.push_str(&format!("\n结果: [错误] {}", preview));
                        } else {
                            entry.push_str(&format!("\n结果: {}", preview));
                        }
                    } else {
                        entry.push_str("\n结果: [无]");
                    }
                    entries.push(entry);
                }
            }
        }
        entries.join("\n\n")
    }

    /// Find ToolResult content corresponding to ToolUse id in session
    pub(crate) fn find_tool_result(&self, tool_use_id: &str) -> Option<String> {
        for msg in self.session.messages() {
            for block in &msg.content {
                if let ContentBlock::ToolResult {
                    tool_use_id: tid,
                    content,
                    ..
                } = block
                {
                    if tid == tool_use_id {
                        return Some(content.clone());
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ProviderKind;

    /// Minimal mock ApiClient for unit tests
    struct MockClient;
    #[async_trait::async_trait]
    impl ApiClient for MockClient {
        async fn stream(
            &self,
            _request: crate::api::MessageRequest,
        ) -> crate::Result<Vec<crate::api::AssistantEvent>> {
            Ok(vec![])
        }
        fn model_name(&self) -> &str {
            "mock"
        }
        fn provider_kind(&self) -> ProviderKind {
            ProviderKind::MiniMax
        }
    }

    #[test]
    fn test_build_tool_schemas_includes_memory_tools() {
        let agent = SubTaskRunner::new_free(
            Arc::new(MockClient),
            ToolRegistry::builtin(),
            "test prompt".to_string(),
            "test".to_string(),
        );
        let schemas = agent.tools.get_schemas();
        let has_search = schemas.iter().any(|s| s.function.name == "memory_search");
        assert!(has_search, "missing memory_search tool");
    }

    #[test]
    fn test_extract_tool_calls_normal() {
        let _agent = SubTaskRunner::new_free(
            Arc::new(MockClient),
            ToolRegistry::builtin(),
            "test".to_string(),
            "test".to_string(),
        );
        let blocks = vec![ContentBlock::ToolUse {
            id: "1".to_string(),
            name: "Read".to_string(),
            input: serde_json::json!({"path": "Cargo.toml"}),
        }];
        let calls = crate::agent::common::extract_tool_calls(&blocks);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool, "Read");
    }

    #[test]
    fn test_extract_tool_calls_skips_text() {
        let _agent = SubTaskRunner::new_free(
            Arc::new(MockClient),
            ToolRegistry::builtin(),
            "test".to_string(),
            "test".to_string(),
        );
        let blocks = vec![
            ContentBlock::Text {
                text: "hello".to_string(),
                reasoning: None,
            },
            ContentBlock::ToolUse {
                id: "1".to_string(),
                name: "Write".to_string(),
                input: serde_json::json!({"path": "out.txt"}),
            },
        ];
        let calls = crate::agent::common::extract_tool_calls(&blocks);
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn test_extract_tool_calls_dedup() {
        let _agent = SubTaskRunner::new_free(
            Arc::new(MockClient),
            ToolRegistry::builtin(),
            "test".to_string(),
            "test".to_string(),
        );
        let blocks = vec![
            ContentBlock::ToolUse {
                id: "1".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({"path": "a.txt"}),
            },
            ContentBlock::ToolUse {
                id: "2".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({"path": "a.txt"}),
            },
        ];
        let calls = crate::agent::common::extract_tool_calls(&blocks);
        assert_eq!(
            calls.len(),
            1,
            "duplicate tool calls should be deduplicated"
        );
    }

    #[test]
    fn test_extract_tool_calls_skips_null_input() {
        let _agent = SubTaskRunner::new_free(
            Arc::new(MockClient),
            ToolRegistry::builtin(),
            "test".to_string(),
            "test".to_string(),
        );
        let blocks = vec![ContentBlock::ToolUse {
            id: "1".to_string(),
            name: "Read".to_string(),
            input: serde_json::Value::Null,
        }];
        let calls = crate::agent::common::extract_tool_calls(&blocks);
        assert!(calls.is_empty(), "null input should be skipped");
    }

    #[test]
    fn test_extract_tool_calls_skips_empty_object() {
        let _agent = SubTaskRunner::new_free(
            Arc::new(MockClient),
            ToolRegistry::builtin(),
            "test".to_string(),
            "test".to_string(),
        );
        let blocks = vec![ContentBlock::ToolUse {
            id: "1".to_string(),
            name: "Read".to_string(),
            input: serde_json::json!({}),
        }];
        let calls = crate::agent::common::extract_tool_calls(&blocks);
        assert_eq!(
            calls.len(),
            1,
            "empty object input should be preserved as valid tool call"
        );
    }

    #[test]
    fn test_build_tool_schemas_includes_standard_tools() {
        let agent = SubTaskRunner::new_free(
            Arc::new(MockClient),
            ToolRegistry::builtin(),
            "test".to_string(),
            "test".to_string(),
        );
        let schemas = agent.tools.get_schemas();
        let names: Vec<&str> = schemas.iter().map(|s| s.function.name.as_str()).collect();
        assert!(names.contains(&"Read"));
        assert!(names.contains(&"Write"));
        assert!(names.contains(&"system_shell"));
    }
}
