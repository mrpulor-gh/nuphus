//! workflow_agent.rs — WorkflowAgent: 独立的工作流设计执行器
//!
//! mode=Workflow 时 Runtime 系统层直接路由到 WorkflowAgent，不经过 Leader。
//! WorkflowAgent 有独立身份提示词、独立 session（跨轮次持久）、复用 Runtime 全部基础设施。
//!
//! 与 SubTaskRunner 的关键区别：
//! - 多轮次持久（不是一次性 exec），session 在多次 run() 调用间存在
//! - 事件 source 用 "workflow" 而非 "exec"
//! - 工具集：排除 Leader 专属工具（planner_*/task_dispatch/leader_memory_update 等）
//! - 包含 desktop_*/browser_* 自动化工具（workflow 探索阶段需要）

use crate::agent::distill;
use crate::agent::events::{EventEmitter, NuphusEvent, StepOutput};
use crate::agent::exec_tool;
use crate::agent::pause::PauseDecision;
use crate::agent::reminders::{ReminderCategory, ReminderPriority, ReminderQueue};
use crate::runtime::protection::ProtectionGuard;
use crate::{
    api::{ApiClient, AssistantEvent, MessageRequest},
    session::{ContentBlock, Session},
    tools::ToolRegistry,
    AgentOutput, ToolCall, ToolResult,
};
use serde_json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// WorkflowAgent 配置
pub struct WorkflowAgentConfig {
    pub max_iterations: usize,
}

impl Default for WorkflowAgentConfig {
    fn default() -> Self {
        Self {
            max_iterations: 1000, // 与 ExecAgent 保持一致（GoalType::MAX_ITERATIONS）
        }
    }
}

/// WorkflowAgent — 独立的工作流设计执行器
///
/// 生命周期由 Runtime 管理：创建后存于 Runtime::workflow_agent，
/// mode=Workflow 时反复使用（跨轮次保持 session），mode 切走时冻结。
pub struct WorkflowAgent {
    pub(crate) llm: Arc<dyn ApiClient>,
    pub(crate) tools: ToolRegistry,
    pub(crate) config: WorkflowAgentConfig,
    /// 独立 session，跨轮次持久
    pub(crate) session: Session,
    pub(crate) emitter: Option<Arc<dyn EventEmitter>>,
    pub(crate) pause_flag: Option<Arc<AtomicBool>>,
    /// L0 prompt cache (stable for API prefix match)
    pub(crate) cached_prompt: Option<String>,
    /// Tool schemas cache (stable across iterations)
    pub(crate) cached_tools: Option<Vec<crate::api::ToolDefinition>>,
    /// Last round output text
    pub(crate) last_output: Option<String>,
    /// Sequential tool call counter (per round)
    pub(crate) tool_call_count: usize,
    /// Tool names used in current turn (for automatic memory storage)
    pub(crate) tools_used_this_turn: Vec<String>,
    /// Execution steps collected in current turn (for automatic memory storage)
    pub(crate) execution_steps: Vec<crate::memory::entry::PersistedStep>,
    /// Safety consecutive failure breaker
    pub(crate) safety_failures: u32,
    /// Reminder queue (persistent deviation reminders)
    pub(crate) reminders: ReminderQueue,
    /// Progressive warning level
    pub(crate) max_warning_injected: usize,
    /// Protection detection state
    pub(crate) protection: ProtectionGuard,
    /// Pending warnings buffer
    pub(crate) pending_warnings: Vec<String>,
    /// User requested stop via pause→terminate
    pub(crate) user_terminated: bool,
    /// Execution start time (per round)
    pub(crate) execution_started_at: std::time::Instant,
    /// Session refine counter (max 2 auto-refines)
    pub(crate) refine_count: u32,
    /// Refine threshold (inherited from Runtime config, same as Leader)
    pub(crate) refine_threshold: f64,
    /// Model label for prompt building
    pub(crate) model_label: String,
    /// 主模型是否原生支持视觉（来自 ModelDef.supports_vision）。
    /// 决定 to_api_messages 是否以 image_url 直发主模型（supports_vision=false 时
    /// 图片保存为临时 BMP + 路径注入，Agent 按需调 desktop_vision，禁止把 image_url 发给不支持视觉的主模型）。
    pub(crate) supports_vision: bool,
    /// Custom user label for address rule (e.g. "大王")
    pub(crate) user_label: String,
    /// Custom assistant name (e.g. "丞相")
    pub(crate) assistant_name: String,
    /// Current tool permissions (inherited from Runtime, synced before each run)
    pub(crate) tool_permissions: crate::permissions::ToolPermissions,
    /// Workflow engine reference (injected by Runtime for workflow_run tool)
    pub(crate) workflow_engine: Option<Arc<tokio::sync::RwLock<crate::workflow::WorkflowEngine>>>,
    /// Message source marker ("desktop" | "mobile"), stamped on ExecutionStarted
    pub(crate) source: String,
    /// 内部流程标记（refine 用）：本轮 input 以 internal user 消息入 session（前端不显示），
    /// 且不重复 push（调用方已手动 push_user_internal）。
    pub(crate) internal_input: bool,
}

impl WorkflowAgent {
    /// Create a new WorkflowAgent
    pub fn new(
        llm: Arc<dyn ApiClient>,
        tools: ToolRegistry,
        emitter: Option<Arc<dyn EventEmitter>>,
        pause_flag: Option<Arc<AtomicBool>>,
        model_label: String,
        user_label: String,
        assistant_name: String,
        tool_permissions: crate::permissions::ToolPermissions,
        refine_threshold: f64,
    ) -> Self {
        Self {
            llm,
            tools,
            config: WorkflowAgentConfig::default(),
            session: Session::new(),
            emitter,
            pause_flag,
            cached_prompt: None,
            cached_tools: None,
            last_output: None,
            tool_call_count: 0,
            tools_used_this_turn: Vec::new(),
            execution_steps: Vec::new(),
            safety_failures: 0,
            reminders: ReminderQueue::new(),
            max_warning_injected: 0,
            protection: ProtectionGuard::new(),
            pending_warnings: Vec::new(),
            user_terminated: false,
            execution_started_at: std::time::Instant::now(),
            refine_count: 0,
            refine_threshold,
            model_label,
            supports_vision: false,
            user_label,
            assistant_name,
            tool_permissions,
            workflow_engine: None,
            source: "desktop".to_string(),
            internal_input: false,
        }
        .apply_supports_vision()
    }

    /// 构造后用 model_label 初始化 supports_vision（与 set_model_label 同源）
    fn apply_supports_vision(mut self) -> Self {
        self.supports_vision = Self::resolve_supports_vision(&self.model_label);
        self
    }

    /// Set message source marker ("desktop" | "mobile"), called before each message round.
    /// Default "desktop"; the mobile HTTP entry sets "mobile" so events carry the origin.
    pub fn set_source(&mut self, source: &str) {
        self.source = source.to_string();
    }

    /// Emit event through shared emitter
    pub(crate) fn emit(&self, event: NuphusEvent) {
        if let Some(ref emitter) = self.emitter {
            emitter.emit(event);
        }
    }

    /// Set pause flag (shared from Runtime)
    pub fn set_pause_flag(&mut self, flag: Option<Arc<AtomicBool>>) {
        self.pause_flag = flag;
    }

    /// Set event emitter
    pub fn set_emitter(&mut self, emitter: Option<Arc<dyn EventEmitter>>) {
        self.emitter = emitter;
    }

    /// Take event emitter (refine 等内部流程临时静默用，run 后 restore)
    pub fn take_emitter(&mut self) -> Option<Arc<dyn EventEmitter>> {
        self.emitter.take()
    }

    /// 标记本轮 input 为内部流程（refine）：input 以 internal user 消息入 session（前端不显示），
    /// 且 run 内不再重复 push（调用方已手动 push_user_internal）。
    pub fn set_internal_input(&mut self, internal: bool) {
        self.internal_input = internal;
    }

    /// Set model label (for prompt building)
    pub fn set_model_label(&mut self, label: String) {
        self.supports_vision = Self::resolve_supports_vision(&label);
        self.model_label = label;
    }

    /// Current model label (for change detection across turns)
    pub fn model_label(&self) -> &str {
        &self.model_label
    }

    /// Replace the underlying LLM client + model label when agent-level model changes.
    /// Keeps session (cross-turn context), invalidates prompt/tool caches
    /// (prompt content is model-dependent).
    pub fn set_llm(&mut self, llm: Arc<dyn ApiClient>, model_label: String) {
        self.llm = llm;
        self.supports_vision = Self::resolve_supports_vision(&model_label);
        self.model_label = model_label;
        self.cached_prompt = None;
        self.cached_tools = None;
    }

    /// 从 model registry 解析主模型是否原生支持视觉（与 RuntimeBuilder 同源逻辑）
    fn resolve_supports_vision(model_label: &str) -> bool {
        crate::config::load_registry()
            .ok()
            .and_then(|r| r.find_model(model_label).map(|(_, m)| m.supports_vision))
            .unwrap_or(false)
    }

    /// Update user/assistant names and invalidate prompt cache
    pub fn set_names(&mut self, user_label: String, assistant_name: String) {
        if self.user_label != user_label || self.assistant_name != assistant_name {
            self.cached_prompt = None; // invalidate cache
        }
        self.user_label = user_label;
        self.assistant_name = assistant_name;
    }

    /// Sync tool permissions from Runtime (called before each run)
    pub fn set_tool_permissions(&mut self, perms: crate::permissions::ToolPermissions) {
        self.tool_permissions = perms;
    }

    /// Inject workflow engine for workflow_run tool support
    pub fn set_workflow_engine(
        &mut self,
        engine: Arc<tokio::sync::RwLock<crate::workflow::WorkflowEngine>>,
    ) {
        self.workflow_engine = Some(engine);
    }

    /// Get a reference to the WorkflowAgent's session
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Get a mutable reference to the WorkflowAgent's session
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Get the refine count
    pub fn refine_count(&self) -> u32 {
        self.refine_count
    }

    /// Increment the refine count
    pub fn inc_refine_count(&mut self) {
        self.refine_count += 1;
    }

    /// Sync emitter, names, tool permissions, and workflow engine before run
    pub fn sync_before_run(
        &mut self,
        emitter: Option<Arc<dyn EventEmitter>>,
        user_label: &str,
        assistant_name: &str,
        tool_permissions: crate::permissions::ToolPermissions,
        workflow_engine: Option<Arc<tokio::sync::RwLock<crate::workflow::WorkflowEngine>>>,
    ) {
        self.set_emitter(emitter);
        self.set_names(user_label.to_string(), assistant_name.to_string());
        self.set_tool_permissions(tool_permissions);
        if let Some(engine) = workflow_engine {
            self.set_workflow_engine(engine);
        }
    }

    /// Inject workflow memory tail from workflow-memory.md（append 日志，最新条目）
    pub fn inject_memory_snapshot(&mut self) {
        if self.session.is_empty() {
            let wmem_path = crate::utils::nuphus_data_dir().join("workflow-memory.md");
            if wmem_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&wmem_path) {
                    // append 日志：只注入最新条目 tail（≤2000 字符），与 Leader memory journal 注入一致
                    let tail = crate::utils::memory_journal_tail(&content, 2000);
                    let trimmed = tail.trim();
                    if !trimmed.is_empty() {
                        self.session.push_user(format!(
                            "=== 历史工作流记忆（来自 workflow-memory.md，追加式日志，最新即所读）===\n{trimmed}\n\n当前 session id：{}\n记忆快照文件：read {}（追加式日志，tail 即最新）\n> 更新用 workflow_memory_update；跨会话检索用 memory_search / memory_recent",
                            self.session.id,
                            wmem_path.display()
                        ));
                        tracing::info!("[WORKFLOW-MEMORY] Injected workflow-memory.md snapshot");
                    }
                }
            }
        }
    }

    /// Session refinement check — called from spawned-task post-processing
    /// after the agent has been put back to state.runtime.
    pub async fn maybe_refine_session(
        &mut self,
        context_window: usize,
        refine_threshold: f64,
        emitter: Option<&dyn EventEmitter>,
    ) {
        distill::maybe_refine_session(
            &mut self.session,
            context_window,
            refine_threshold,
            emitter,
            &mut self.refine_count,
        )
        .await;
    }

    /// Build tool schemas — WorkflowAgent 的工具集已在构造时由 ToolRegistry::work_agent() 过滤，
    /// 此处直接取全量，无需二次过滤（LEADER_ONLY 工具本就不在其中）。
    fn get_filtered_schemas(&self) -> Vec<crate::api::ToolDefinition> {
        self.tools.get_schemas()
    }

    /// Build system prompt (cached)
    fn build_system_prompt(&mut self) -> String {
        if self.cached_prompt.is_none() {
            let tool_schemas = {
                let schemas = self.get_filtered_schemas();
                serde_json::to_string(&schemas).unwrap_or_default()
            };
            // Check vision availability: capabilities.vision > main model supports_vision > none
            let vision_model = match crate::config::resolve_vision_strategy() {
                crate::config::VisionStrategy::Capability(name) => Some(name),
                crate::config::VisionStrategy::Main => Some(self.model_label.clone()),
                crate::config::VisionStrategy::None => None,
            };
            // 主模型 supports_vision：从配置/builtin 读取
            let main_supports_vision = crate::config::load_registry()
                .ok()
                .and_then(|r| {
                    r.find_model(&self.model_label)
                        .map(|(_, m)| m.supports_vision)
                })
                .unwrap_or(false);
            self.cached_prompt = Some(crate::agent::prompt::build_workagent_prompt(
                &self.model_label,
                main_supports_vision,
                &tool_schemas,
                &self.user_label,
                &self.assistant_name,
                vision_model.as_deref(),
            ));
        }
        self.cached_prompt
            .as_ref()
            .expect("cached_prompt set to Some just above in this function")
            .clone()
    }
}

// ── Execution loop ──

impl WorkflowAgent {
    /// Run WorkflowAgent on user input (multi-round: appends to existing session)
    ///
    /// Returns AgentOutput with success status and result message.
    pub async fn run(
        &mut self,
        input: &str,
        images: &Option<Vec<String>>,
        cancel_flag: &AtomicBool,
    ) -> crate::Result<AgentOutput> {
        self.execution_started_at = std::time::Instant::now();
        self.tool_call_count = 0;
        self.tools_used_this_turn.clear();
        self.execution_steps.clear();
        // 新任务开始时清空上一任务残留的追加指令队列（防跨任务泄漏，与 react_loop 入口一致）
        crate::mobile_append::clear();
        // 首轮注入 workflow 记忆 tail（session 为空时生效，同会话仅一次）
        self.inject_memory_snapshot();
        // Advance turn counter for memory tracking (consistent with Leader)
        self.session.advance_turn();
        self.max_warning_injected = 0;
        self.user_terminated = false;
        self.pending_warnings.clear();

        // 1. Push user input (with optional images) to our session
        // internal_input=true（refine 内部流程）：调用方已手动 push_user_internal，此处跳过
        if !self.internal_input {
            if let Some(imgs) = images {
                if !imgs.is_empty() {
                    let mut blocks: Vec<ContentBlock> = Vec::new();
                    if !input.is_empty() {
                        blocks.push(ContentBlock::Text {
                            text: input.to_string(),
                            reasoning: None,
                        });
                    }
                    for url in imgs {
                        let final_url = if url.starts_with("data:image/bmp") {
                            crate::utils::convert_bmp_data_url_to_png(url).unwrap_or_else(|e| {
                                tracing::warn!("[WA] BMP→PNG 转换失败: {e}，使用原始 URL");
                                url.clone()
                            })
                        } else {
                            url.clone()
                        };
                        blocks.push(ContentBlock::Image { url: final_url });
                    }
                    self.session.push_user_blocks(blocks);
                } else {
                    self.session.push_user(input.to_string());
                }
            } else {
                self.session.push_user(input.to_string());
            }
            // 对齐 Runtime.run()（loop.rs:668）：user 消息广播供手机端 pending 确认 + 桌面端气泡渲染。
            // 置于 if !self.internal_input 块内：refine 内部流程（internal_input=true）不发——
            // refine 消息不显示在前端历史，与 extract_history 过滤 internal 一致
            self.emit(NuphusEvent::UserMessageReceived {
                content: input.to_string(),
                source: self.source.clone(),
                images: images.clone().unwrap_or_default(),
            });
        }

        // 2. Emit lifecycle events
        self.emit(NuphusEvent::ExecutionStarted {
            step_index: 0,
            goal: input.chars().take(120).collect(),
            tools: self.tools.tool_names(),
            source: self.source.clone(),
            mode: "workflow".to_string(),
        });

        let mut user_requested_stop = false;

        for iteration in 0..self.config.max_iterations {
            if cancel_flag.load(Ordering::SeqCst) {
                self.emit(NuphusEvent::Error {
                    code: "cancelled".to_string(),
                    message: "任务已被用户中断".to_string(),
                    retryable: false,
                    from_subtask: false,
                });
                self.store_turn_memory(input, "任务已被用户中断", false);
                return Ok(AgentOutput {
                    success: false,
                    message: "任务已被用户中断".to_string(),
                    steps: vec![],
                    retry_session: None,
                });
            }

            // ── Pause check ──
            if let Some(ref pause_flag) = self.pause_flag {
                if pause_flag.load(Ordering::SeqCst) {
                    let action_id = crate::agent::pause::get_pause_action_id(self.tools.signals())
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    let skip_emit =
                        crate::agent::pause::peek_pause_decision(self.tools.signals(), &action_id)
                            .is_some();
                    if !skip_emit {
                        self.emit(NuphusEvent::ExecutionPaused {
                            action_id: action_id.clone(),
                        });
                    }
                    let decision = crate::agent::pause::wait_for_pause_decision_global(
                        self.tools.signals(),
                        &action_id,
                        cancel_flag,
                    )
                    .await;
                    pause_flag.store(false, Ordering::SeqCst);
                    match decision {
                        PauseDecision::Continue => {
                            tracing::info!("[WORKFLOW-PAUSE] User chose to continue");
                        }
                        PauseDecision::Append(instr) => {
                            tracing::info!("[WORKFLOW-PAUSE] User appended instruction");
                            self.session.push_user(instr);
                        }
                        PauseDecision::Terminate => {
                            tracing::info!("[WORKFLOW-PAUSE] User chose to terminate");
                            self.session.push_user(
                                "⚠ 用户要求立即停止当前操作。请立即整理已有成果，输出你已完成的内容和当前状态。不要继续执行任何工具调用。".to_string()
                            );
                            user_requested_stop = true;
                            self.user_terminated = true;
                        }
                    }
                }
            }

            self.session.strip_incomplete_tools();

            // ── 追加指令注入：与 react_loop 同一注入位——执行中用户发送的追加
            // 指令（busy 锁占用时入队 mobile_append）在迭代边界 drain，插入下一轮。
            // 电脑端/手机端追加指令在 workflow 执行中同样生效（与 Leader 模式一致）。
            let mobile_appends = crate::mobile_append::drain_for_injection();
            if !mobile_appends.is_empty() {
                self.session
                    .push_user(crate::mobile_append::format_mobile_append_section(
                        &mobile_appends,
                    ));
            }

            // ── Context watermark warning (WorkflowAgent 无压缩机制) ──
            if self.inject_context_warning() {
                // Forbidden warning injected, give LLM one more chance
            } else if self.max_warning_injected >= 4 {
                tracing::warn!("[WORKFLOW-FORBID] Context watermark exceeded limit");
                break;
            }

            // ── LLM call with real-time streaming ──
            let events = self.llm_stream_with_streaming(cancel_flag).await?;

            let assistant_blocks = self.process_events(events);

            // Text and reasoning already emitted in real-time during streaming
            // (TextDelta via process_text_delta, Reasoning via direct forward).
            // Only emit_exec preview for final text block.

            self.emit(NuphusEvent::ExecutionProgress {
                iteration: iteration as u32 + 1,
                max_iterations: self.config.max_iterations as u32,
                tool_calls_so_far: self.tool_call_count,
            });

            if assistant_blocks.is_empty() {
                continue;
            }

            let tool_calls = crate::agent::common::extract_tool_calls(&assistant_blocks);

            let valid_ids: std::collections::HashSet<&str> =
                tool_calls.iter().map(|c| c.id.as_str()).collect();
            let filtered_blocks: Vec<ContentBlock> = assistant_blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, .. } if !valid_ids.contains(id.as_str()) => None,
                    _ => Some(b.clone()),
                })
                .collect();

            // ── No tool calls → return result ──
            if tool_calls.is_empty() {
                self.session.push_assistant(filtered_blocks);
                let text = assistant_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text, .. } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let result_msg = text.trim().to_string();
                let result_msg = if result_msg.is_empty() && self.tool_call_count > 0 {
                    format!("任务完成，共执行 {} 次工具调用", self.tool_call_count)
                } else {
                    result_msg
                };
                let total_duration = self.execution_started_at.elapsed().as_millis() as u64;
                self.last_output = Some(result_msg.clone());
                self.emit(NuphusEvent::ExecutionCompleted {
                    step_index: 0,
                    output: StepOutput {
                        step_index: 0,
                        result_message: result_msg.clone(),
                        artifacts: vec![],
                        tool_calls_count: self.tool_call_count,
                    },
                    total_duration_ms: total_duration,
                    total_calls: self.tool_call_count,
                });
                self.store_turn_memory(input, &result_msg, true);
                return Ok(AgentOutput {
                    success: true,
                    message: result_msg,
                    steps: vec![],
                    retry_session: None,
                });
            }

            // User requested stop + LLM still returns tools → skip execution
            if user_requested_stop {
                self.session.push_assistant(filtered_blocks);
                let text = assistant_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text, .. } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let result_msg = if text.trim().is_empty() {
                    "操作已停止".to_string()
                } else {
                    text.trim().to_string()
                };
                let total_duration = self.execution_started_at.elapsed().as_millis() as u64;
                self.last_output = Some(result_msg.clone());
                self.emit(NuphusEvent::ExecutionCompleted {
                    step_index: 0,
                    output: StepOutput {
                        step_index: 0,
                        result_message: result_msg.clone(),
                        artifacts: vec![],
                        tool_calls_count: self.tool_call_count,
                    },
                    total_duration_ms: total_duration,
                    total_calls: self.tool_call_count,
                });
                // ── Session distillation before user stop exit ──
                let ctx_window = crate::agent::goal_types::get_context_window(&self.model_label);
                distill::maybe_refine_session(
                    &mut self.session,
                    ctx_window,
                    self.refine_threshold,
                    self.emitter.as_deref(),
                    &mut self.refine_count,
                )
                .await;
                self.store_turn_memory(input, &result_msg, true);
                return Ok(AgentOutput {
                    success: true,
                    message: result_msg,
                    steps: vec![],
                    retry_session: None,
                });
            }

            // ── Has tool calls: push to session, then execute ──
            self.session.push_assistant(assistant_blocks);

            // ── Sequential tool execution (workflow design is deliberation-heavy) ──
            let mut protection_warnings: Vec<String> = Vec::new();

            for call in &tool_calls {
                self.tool_call_count += 1;
                self.emit(NuphusEvent::ToolCallStart {
                    call_id: call.id.clone(),
                    tool_name: call.tool.clone(),
                    params: call.params.clone(),
                    iteration: iteration as u32,
                    from_task: false,
                });

                // Protection check
                let alert = self.protection.check_pre_call(call);
                if let Some(ref a) = alert {
                    tracing::warn!("[WORKFLOW-PROTECT] {}: tool={}", a.label(), call.tool);
                    protection_warnings.push(a.to_session_warning());
                }

                // Safety check (permissions + security guard)
                if let Some(err_result) = self.check_tool_safety(call, cancel_flag).await {
                    self.emit(NuphusEvent::ToolCallEnd {
                        call_id: call.id.clone(),
                        tool_name: call.tool.clone(),
                        success: false,
                        duration_ms: 0,
                        output_preview: String::new(),
                        output_full_size: 0,
                        is_truncated: false,
                        error: err_result.error.clone(),
                        from_task: false,
                    });

                    match exec_tool::breaker_check(self.safety_failures) {
                        exec_tool::BreakerAction::Halt => {
                            tracing::error!(
                                "[WORKFLOW-BREAKER] {} consecutive safety failures, aborting",
                                self.safety_failures
                            );
                            self.session.strip_incomplete_tools();
                            return Ok(AgentOutput {
                                success: false,
                                message: format!(
                                    "执行中止: 连续 {} 次安全检查未通过",
                                    self.safety_failures
                                ),
                                steps: vec![],
                                retry_session: None,
                            });
                        }
                        exec_tool::BreakerAction::Warn | exec_tool::BreakerAction::Restrict => {
                            if let Some(msg) = exec_tool::breaker_message(self.safety_failures) {
                                self.session.push_user(msg);
                            }
                        }
                        exec_tool::BreakerAction::None => {}
                    }

                    if let Some(ref a) = self.protection.check_post_call(call) {
                        protection_warnings.push(a.to_session_warning());
                    }
                    self.session.push_tool_result(
                        call.id.clone(),
                        err_result.error.clone().unwrap_or_default(),
                        true,
                    );
                    continue;
                }

                // ── Execute tool ──
                let start = std::time::Instant::now();

                // Inject session_id for workflow_memory_update
                let exec_params = if call.tool == "workflow_memory_update" {
                    let mut p = call.params.clone();
                    if let serde_json::Value::Object(ref mut map) = p {
                        map.insert(
                            "session_id".to_string(),
                            serde_json::Value::String(self.session.id.clone()),
                        );
                    }
                    p
                } else {
                    call.params.clone()
                };

                // ── workflow_validate: 静态编译检查 ──
                let mut result = if call.tool == "workflow_validate" {
                    let raw_id = call.params.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    match self.workflow_engine.as_ref() {
                        Some(engine) => {
                            // 热刷新
                            {
                                if let Err(e) = engine.read().await.store.load_all().await {
                                    tracing::warn!("workflow_validate 热刷新失败: {}", e);
                                }
                            }
                            let engine_r = engine.read().await;
                            let tools_schemas = self.tools.get_schemas();
                            match engine_r.store.get(raw_id).await {
                                Some(wf) => {
                                    let report = crate::workflow::compiler::Compiler::validate_workflow_with_tools(&wf, &tools_schemas);
                                    let json = serde_json::json!({
                                        "passed": report.passed,
                                        "errors": report.errors,
                                        "warnings": report.warnings,
                                    });
                                    crate::ToolResult::success(json.to_string())
                                }
                                None => crate::ToolResult::failure(format!(
                                    "Workflow '{}' not found",
                                    raw_id
                                )),
                            }
                        }
                        None => {
                            crate::ToolResult::failure("workflow_engine not injected".to_string())
                        }
                    }
                // ── workflow_run: delegate to WorkflowEngine ──
                } else if call.tool == "workflow_run" {
                    let raw_id = call.params.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let inputs: Option<std::collections::HashMap<String, serde_json::Value>> = call
                        .params
                        .get("inputs")
                        .and_then(|v| v.as_object())
                        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
                    match self.workflow_engine.as_ref() {
                        Some(engine) => {
                            // 热刷新：确保最新工作流被加载
                            {
                                if let Err(e) = engine.read().await.store.load_all().await {
                                    tracing::warn!("workflow_run 热刷新失败: {}", e);
                                }
                            }
                            let workflow_id = {
                                let engine_r = engine.read().await;
                                let summaries = engine_r.store.list().await;
                                if summaries.iter().any(|s| s.id == raw_id) {
                                    raw_id.to_string()
                                } else {
                                    let matched: Vec<_> = summaries
                                        .iter()
                                        .filter(|s| s.id.contains(raw_id))
                                        .collect();
                                    if matched.len() == 1 {
                                        matched[0].id.clone()
                                    } else if matched.len() > 1 {
                                        let err_msg = format!("workflow_run: ambiguous id '{}' matches {} workflows: {}",
                                            raw_id, matched.len(),
                                            matched.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(", "));
                                        self.store_turn_memory(input, &err_msg, false);
                                        return Ok(AgentOutput {
                                            success: false,
                                            message: err_msg,
                                            steps: vec![],
                                            retry_session: None,
                                        });
                                    } else {
                                        raw_id.to_string()
                                    }
                                }
                            };
                            // Inject LLM client + ToolRegistry for ChatAgent step support (write lock briefly)
                            {
                                let mut engine_w = engine.write().await;
                                engine_w.set_llm_client(self.llm.clone());
                                engine_w.set_tools(Arc::new(self.tools.clone()));
                            }
                            // Acquire read lock for execution (allows concurrent pause/cancel)
                            let engine_r = engine.read().await;
                            let tool_schemas = engine_r.tools().map(|t| t.get_schemas());
                            let tool_exec = |tool: String, params: serde_json::Value| {
                                let tools = &self.tools;
                                async move {
                                    let result = if tool.starts_with("browser_") {
                                        tools
                                            .execute_browser_tool(&tool, &params)
                                            .await
                                            .map_err(|e| e.to_string())?
                                    } else {
                                        tools
                                            .execute(&tool, &params)
                                            .await
                                            .map_err(|e| e.to_string())?
                                    };
                                    if result.success {
                                        Ok(result.output.unwrap_or_default())
                                    } else {
                                        Err(result.error.unwrap_or_default())
                                    }
                                }
                            };
                            let exec_result = engine_r
                                .execute_workflow(
                                    &workflow_id,
                                    tool_exec,
                                    tool_schemas,
                                    self.emitter.as_deref(),
                                    inputs,
                                    crate::workflow::WorkflowRunSource::Agent {
                                        owner: crate::runtime::Mode::Workflow,
                                    },
                                )
                                .await;
                            // ── 用户主动终止检测（与 react_loop 对齐）──
                            let was_user_cancelled =
                                crate::workflow::hud_control::take_user_cancelled();
                            let (success, mut output, error) = match exec_result {
                                Ok(msg) => (true, Some(msg), None),
                                Err(e) => (false, None, Some(e.to_string())),
                            };
                            if was_user_cancelled {
                                let note = "\n\n[用户已终止此工作流的执行]";
                                output = Some(match output {
                                    Some(s) => s + note,
                                    None => note.to_string(),
                                });
                                self.pending_warnings.push(
                                    "用户已终止工作流的执行。请直接向用户报告结果并结束当前任务，不要重新调用 workflow_run。".to_string(),
                                );
                                if let Some(ref emitter) = self.emitter {
                                    emitter.emit(NuphusEvent::HudUpdate {
                                        text: "工作流已由用户终止".into(),
                                        phase: "warning".into(),
                                        step_kind: None,
                                    });
                                }
                            }
                            crate::ToolResult {
                                success,
                                output,
                                error,
                                exit_code: None,
                            }
                        }
                        None => crate::ToolResult::failure(
                            "workflow_engine not injected to WorkflowAgent".to_string(),
                        ),
                    }
                } else {
                    crate::agent::exec_tool::execute_tool_only(
                        &self.tools,
                        &call.tool,
                        &exec_params,
                        None,
                        self.emitter.as_deref(),
                    )
                    .await
                };

                // ── Post-exec: request_user_input → emit UserInputRequest + wait for user ──
                if call.tool == "request_user_input" && result.success {
                    if let Some(ref output) = result.output {
                        if let Some(action_id) = crate::agent::exec_tool::extract_action_id(output)
                        {
                            if let Some(pending) =
                                crate::security::user_input::get(self.tools.signals(), &action_id)
                            {
                                self.emit(NuphusEvent::UserInputRequest {
                                    action_id: pending.action_id.clone(),
                                    title: pending.title.clone(),
                                    prompt: pending.prompt.clone(),
                                    sensitive: pending.sensitive,
                                    input_type: pending.input_type.clone(),
                                    icon_path: pending.icon_path.clone(),
                                    default_name: pending.default_name.clone(),
                                    default_shortcut: pending.default_shortcut.clone(),
                                    rel_x: pending.rel_x,
                                    rel_y: pending.rel_y,
                                    default_note: pending.default_note.clone(),
                                });
                            }
                            let value = crate::agent::exec_tool::wait_for_user_input_async(
                                self.tools.signals(),
                                &action_id,
                                cancel_flag,
                                self.emitter.as_deref(),
                            )
                            .await;
                            match value {
                                Some(v) => {
                                    let parsed: serde_json::Value = serde_json::from_str(&v)
                                        .unwrap_or(serde_json::Value::String(v));
                                    result = crate::ToolResult::success(parsed.to_string());
                                }
                                None => {
                                    result = crate::ToolResult::failure(
                                        "用户未提供输入或操作已取消".to_string(),
                                    );
                                }
                            }
                        }
                    }
                }

                let duration_ms = start.elapsed().as_millis() as u64;

                let output_str = result
                    .output
                    .as_deref()
                    .or(result.error.as_deref())
                    .unwrap_or("");
                tracing::debug!(
                    "[WORKFLOW] tool result: tool={}, success={}",
                    call.tool,
                    result.success,
                );

                // ── Automatic memory: collect tool execution data ──
                if !self.tools_used_this_turn.contains(&call.tool) {
                    self.tools_used_this_turn.push(call.tool.clone());
                }
                self.execution_steps
                    .push(crate::memory::entry::PersistedStep::new(
                        call.tool.clone(),
                        &call.params,
                        if result.success {
                            Some(output_str)
                        } else {
                            let err_msg = result.error.as_deref().unwrap_or(output_str);
                            Some(err_msg)
                        },
                        result.success,
                        Some(duration_ms),
                    ));

                let preview_limit = 2000;
                self.emit(NuphusEvent::ToolCallEnd {
                    call_id: call.id.clone(),
                    tool_name: call.tool.clone(),
                    success: result.success,
                    duration_ms,
                    output_preview: output_str.chars().take(preview_limit).collect(),
                    output_full_size: output_str.len(),
                    is_truncated: output_str.chars().count() > preview_limit,
                    error: result.error.clone(),
                    from_task: false,
                });

                // External content: sanitize + injection scan + untrusted boundary (unified entry)
                let filtered = crate::filter::ToolOutputFilter::apply(&call.tool, output_str);
                let filtered = crate::security::injection::process_external_output(
                    &call.tool,
                    Some(&call.params),
                    &filtered,
                );
                let truncated = crate::utils::truncate_tool_output(&filtered, 8000, &call.tool);
                self.session
                    .push_tool_result(call.id.clone(), truncated, !result.success);

                if !result.success {
                    if let Some(ref a) = self.protection.check_post_call(call) {
                        protection_warnings.push(a.to_session_warning());
                    }
                } else {
                    self.safety_failures = 0;
                    self.protection.reset_consecutive_errors();
                }

                if cancel_flag.load(Ordering::SeqCst) {
                    break;
                }
            }

            // Flush warnings into ReminderQueue
            for w in protection_warnings.drain(..) {
                self.reminders.enqueue(
                    w,
                    3,
                    ReminderPriority::High,
                    ReminderCategory::DeviationCorrect,
                );
            }
            for w in self.pending_warnings.drain(..) {
                self.reminders.enqueue(
                    w,
                    3,
                    ReminderPriority::High,
                    ReminderCategory::DeviationCorrect,
                );
            }
        }

        // Max iterations reached
        let result_msg = format!(
            "达到最大迭代次数（已执行 {} 次工具调用）",
            self.tool_call_count
        );
        self.emit(NuphusEvent::ExecutionError {
            step_index: 0,
            error: "达到最大迭代次数".to_string(),
        });
        // ── Session distillation before max iterations exit ──
        let ctx_window = crate::agent::goal_types::get_context_window(&self.model_label);
        distill::maybe_refine_session(
            &mut self.session,
            ctx_window,
            self.refine_threshold,
            self.emitter.as_deref(),
            &mut self.refine_count,
        )
        .await;
        self.store_turn_memory(input, &result_msg, false);
        Ok(AgentOutput {
            success: false,
            message: result_msg,
            steps: vec![],
            retry_session: None,
        })
    }

    /// ── Automatic turn memory storage ──
    ///
    /// Stores the current turn's execution data to SQLite memory_entries.
    /// Called at every exit point of run(), ensuring every turn is recorded
    /// regardless of success/failure/cancellation.
    fn store_turn_memory(&self, input: &str, result_msg: &str, success: bool) {
        use crate::memory::entry::{
            build_entry_id, normalize_tags, tool_category_tags, truncate, AgentType, MemoryEntry,
            MemoryKind,
        };

        if input.is_empty() && self.tools_used_this_turn.is_empty() {
            return; // Nothing meaningful to store
        }

        let turn_id = self.session.turn_count.to_string();
        let mut entry = MemoryEntry::new(
            build_entry_id(AgentType::WorkAgent, &self.session.id, &turn_id, 0),
            self.session.id.clone(),
            turn_id.clone(),
            AgentType::WorkAgent,
            MemoryKind::TaskTrace,
        );

        entry.intent = truncate(input, 200);
        entry.summary = if !result_msg.is_empty() {
            truncate(result_msg, 300)
        } else if !self.tools_used_this_turn.is_empty() {
            let names = self.tools_used_this_turn.join(", ");
            truncate(
                &format!(
                    "Called {} tool(s): {}",
                    self.tools_used_this_turn.len(),
                    names
                ),
                300,
            )
        } else {
            "no tools executed".to_string()
        };
        entry.user_message = truncate(input, 2000);
        entry.assistant_message = truncate(result_msg, 2000);
        entry.tools_used = self.tools_used_this_turn.clone();
        entry.success = success;
        // 紧凑轨迹最多保留最后 20 步（与 entry_from_exec_steps 一致）
        let keep_from = self.execution_steps.len().saturating_sub(20);
        entry.execution_steps = self.execution_steps[keep_from..].to_vec();
        entry.goal_type = Some("workflow_turn".to_string());

        let mut tags = vec![
            "workflow_turn".to_string(),
            if success { "success" } else { "failure" }.to_string(),
        ];
        tags.extend(tool_category_tags(&entry.tools_used));
        entry.tags = normalize_tags(&tags);

        if let Err(e) = crate::store::memory::insert_entry(&entry) {
            tracing::warn!("[WORKFLOW-MEMORY] Failed to store turn entry: {}", e);
        } else {
            tracing::info!(
                "[WORKFLOW-MEMORY] Stored turn turn={} session={} tools={} success={}",
                turn_id,
                &entry.session_id[..8.min(entry.session_id.len())],
                self.tools_used_this_turn.len(),
                success
            );
            // 同步 sessions 表：确保 WorkflowAgent session 出现在历史列表中
            let now = chrono::Utc::now().to_rfc3339();
            let existing = crate::store::session::get_session(&entry.session_id)
                .ok()
                .flatten();
            let row = crate::store::session::SessionRow {
                id: entry.session_id.clone(),
                parent_id: existing.as_ref().and_then(|r| r.parent_id.clone()),
                depth: existing.as_ref().map(|r| r.depth).unwrap_or(0),
                created_at: existing
                    .as_ref()
                    .map(|r| r.created_at.clone())
                    .unwrap_or_else(|| now.clone()),
                updated_at: now,
                message_count: existing.as_ref().map(|r| r.message_count + 1).unwrap_or(1),
                token_count: 0,
                summary: if !result_msg.is_empty() {
                    crate::memory::entry::truncate(result_msg, 200)
                } else {
                    crate::memory::entry::truncate(input, 200)
                },
            };
            let _ = crate::store::session::upsert_session(&row);
        }
    }

    /// LLM call with real-time streaming to frontend + network retry
    async fn llm_stream_with_streaming(
        &mut self,
        cancel_flag: &AtomicBool,
    ) -> crate::Result<Vec<AssistantEvent>> {
        const MAX_RETRIES: u32 = 2;

        // Tool schemas cached
        let tools = {
            if self.cached_tools.is_none() {
                self.cached_tools = Some(self.get_filtered_schemas());
            }
            self.cached_tools
                .as_ref()
                .expect("cached_tools set to Some just above")
                .clone()
        };

        // System prompt cached
        let base = self.build_system_prompt();

        for attempt in 0..=MAX_RETRIES {
            // Push reminders as user message to keep system_prompt stable
            if let Some(rem) = self.reminders.format_for_prompt() {
                if !rem.is_empty() {
                    self.session.push_user(rem);
                }
            }
            let messages = self.session.to_api_messages(self.supports_vision);
            let request = MessageRequest::new("", messages)
                .with_system(base.clone())
                .with_tools(tools.clone());

            // Collect events while streaming text deltas to frontend in real-time
            // AtomicU32 tracks nesting depth to prevent premature close when
            // LLM discusses `` tags within thinking content.
            let in_think = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
            let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let collected_clone = collected.clone();
            let stream_emitter = self.emitter.clone();
            let think_state = in_think.clone();
            let emitter = Box::new(move |event: crate::api::AssistantEvent| {
                if let crate::api::AssistantEvent::TextDelta(text) = &event {
                    // Route <think> content to reasoning (is_thinking=true) and
                    // non-think text to chat bubble (is_thinking=false).
                    // Each thinking delta emitted immediately — no buffering.
                    let (reasoning, text_clean) =
                        crate::utils::process_text_delta(text, &think_state);
                    if let Some(ref em) = stream_emitter {
                        if let Some(r) = reasoning {
                            em.emit(NuphusEvent::LlmTextDelta {
                                text: r,
                                is_thinking: true,
                                from_task: false,
                            });
                        }
                        if !text_clean.is_empty() {
                            em.emit(NuphusEvent::LlmTextDelta {
                                text: text_clean,
                                is_thinking: false,
                                from_task: false,
                            });
                        }
                    }
                }
                if let crate::api::AssistantEvent::Reasoning(text) = &event {
                    // DeepSeek thinking mode: reasoning_content deltas arrive as
                    // Reasoning events (not TextDelta). Forward in real-time so
                    // thinking appears BEFORE text in the frontend timeline.
                    if let Some(ref em) = stream_emitter {
                        em.emit(NuphusEvent::LlmTextDelta {
                            text: text.clone(),
                            is_thinking: true,
                            from_task: false,
                        });
                    }
                }
                collected_clone
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(event);
            });

            match self
                .llm
                .stream_with_emitter(request, cancel_flag, emitter)
                .await
            {
                Ok(()) => {
                    return Ok(std::mem::take(
                        &mut *collected.lock().unwrap_or_else(|e| e.into_inner()),
                    ))
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if attempt == MAX_RETRIES || !crate::agent::common::is_network_error(&err_str) {
                        return Err(e);
                    }
                    tracing::info!(
                        "LLM network error, retry {}/{}: {}",
                        attempt + 1,
                        MAX_RETRIES + 1,
                        err_str,
                    );
                    self.emit(NuphusEvent::Warning {
                        code: "llm_network_retry".to_string(),
                        message: format!(
                            "Network connection timeout, retrying ({}/{})",
                            attempt + 1,
                            MAX_RETRIES + 1,
                        ),
                    });
                    tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
                }
            }
        }
        // All paths inside the loop return, but this is a defense-in-depth fallback
        Err(crate::NuphusError::LLM(
            crate::LLMError::RetryLoopExhausted {
                last_error:
                    "LLM stream retry loop exhausted without returning — unexpected control flow"
                        .to_string(),
            },
        ))
    }

    /// Process streaming events: extract blocks + emit token usage events
    fn process_events(&mut self, events: Vec<AssistantEvent>) -> Vec<ContentBlock> {
        let content_tool_tags = crate::config::registry::ProviderRegistry::builtin()
            .get(self.llm.provider_kind().as_str())
            .map(|p| p.quirks().content_tool_tags)
            .unwrap_or(&[]);
        let result = crate::agent::common::process_events(events, content_tool_tags);
        if let Some((input, output)) = &result.usage {
            self.session.update_api_input_tokens(*input as u64);
            // Per-call consumption for exec tracking (shows "XX tok" in status bar)
            self.emit(NuphusEvent::TokenUsage {
                input_tokens: *input,
                output_tokens: *output,
                cache_hit_tokens: result.cache_hit_tokens,
                source: "workflow".to_string(),
            });
            // Cumulative session usage for context bar (like Leader's "main" source)
            self.emit(NuphusEvent::TokenUsage {
                input_tokens: self.session.api_input_tokens as u32,
                output_tokens: 0,
                cache_hit_tokens: result.cache_hit_tokens,
                source: "main".to_string(),
            });
        }
        result.blocks
    }

    /// Inject progressive warnings based on context watermark
    /// WorkflowAgent 无压缩机制，上下文用完后会直接截断，必须提前预警
    fn inject_context_warning(&mut self) -> bool {
        const WARN_REMIND: f64 = 0.60;
        const WARN_EMPHASIS: f64 = 0.75;
        const WARN_REDLINE: f64 = 0.88;
        const WARN_FORBID: f64 = 0.95;

        let usage = self.session.estimate_token_usage();
        let ctx_window = crate::agent::goal_types::get_context_window(&self.model_label);
        let ratio = usage as f64 / ctx_window as f64;

        let level = if ratio >= WARN_FORBID {
            4
        } else if ratio >= WARN_REDLINE {
            3
        } else if ratio >= WARN_EMPHASIS {
            2
        } else if ratio >= WARN_REMIND {
            1
        } else {
            0
        };

        if level <= self.max_warning_injected {
            return false;
        }
        self.max_warning_injected = level;

        let pct = (ratio * 100.0) as u32;
        match level {
             1 => self.reminders.enqueue(
                format!("上下文使用量已达 {}%（{}/{}）。Workflow 模式无自动压缩，需主动控制上下文用量。如已收集足够信息请停止探索输出结果。", pct, usage, ctx_window),
                2, ReminderPriority::Normal, ReminderCategory::DeviationCorrect,
            ),
             2 => self.reminders.enqueue(
                format!("上下文使用量已达 {}%（{}/{}）。请尽快输出结果，避免后续截断。", pct, usage, ctx_window),
                2, ReminderPriority::High, ReminderCategory::DeviationCorrect,
            ),
            3 => self.reminders.enqueue(
                format!("上下文使用量已达 {}%（{}/{}），即将达到上限。必须立即输出最终结果。", pct, usage, ctx_window),
                2, ReminderPriority::Critical, ReminderCategory::DeviationCorrect,
            ),
             4 => {
                self.reminders.enqueue(
                    format!("上下文已达上限（{}/{}），工具调用将被关闭，立即产出回复。", usage, ctx_window),
                    2, ReminderPriority::Critical, ReminderCategory::DeviationCorrect,
                );
            }
            _ => {}
        }
        level >= 4
    }

    /// Safety check: session authorization → permissions → SecurityGuard
    async fn check_tool_safety(
        &mut self,
        call: &ToolCall,
        cancel_flag: &AtomicBool,
    ) -> Option<ToolResult> {
        // Use permissions inherited from Runtime (synced before each run via set_tool_permissions)
        let policy = crate::permissions::PermissionPolicy::new(self.tool_permissions)
            .with_categories(self.tools.all_tool_categories());

        let emitter: Option<&dyn EventEmitter> = self.emitter.as_deref();

        crate::agent::exec_tool::check_tool_security(
            self.tools.signals(),
            &call.tool,
            &call.params,
            &policy,
            emitter,
            cancel_flag,
            Some(&self.tool_permissions),
            &mut self.safety_failures,
            &mut self.pending_warnings,
        )
        .await
    }
}
