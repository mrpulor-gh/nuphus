//! Loop — Runtime main loop: agent lifecycle, session recovery, ReAct loop orchestration, and post-processing.

use crate::agent::events::{EventEmitter, NuphusEvent, StepOutput};
use crate::agent::goal_types::RelationConfig;
use crate::agent::prompt::LeaderContext;
use crate::agent::{AgentConfig, ReactAgent};
use crate::annotation::store::AnnotationStore;
use crate::api::ApiClient;
use crate::permissions::ToolPermissions;
use crate::runtime::Mode;
use crate::session::Session;
use crate::tools::ToolRegistry;
use crate::{ExecutionStep, Result};

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Runtime internal event — unified type, ultimately mapped to NuphusEvent
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEvent {
    /// User message received
    UserMessageReceived {
        content: String,
        source: String,
        #[serde(default)]
        images: Vec<String>,
    },
    /// Execution started
    ExecutionStarted {
        mode: Mode,
        goal: String,
        /// Message source: "desktop" | "mobile"
        source: String,
    },
    /// LLM text delta
    LlmTextDelta {
        text: String,
        is_thinking: bool,
        from_task: bool,
    },
    /// Tool call start
    ToolCallStart {
        tool_name: String,
        params: serde_json::Value,
    },
    /// Tool call end
    ToolCallEnd {
        tool_name: String,
        success: bool,
        duration_ms: u64,
        output_preview: String,
    },
    /// Execution completed
    ExecutionCompleted {
        message: String,
        total_calls: usize,
        total_duration_ms: u64,
    },
    /// Error
    Error {
        code: String,
        message: String,
        retryable: bool,
    },
    /// Token usage
    TokenUsage {
        input_tokens: u32,
        output_tokens: u32,
        cache_hit_tokens: u32,
    },
    /// System message
    SystemMessage { message: String },
    /// Security check
    SecurityCheck {
        action_id: String,
        tool: String,
        reason: String,
    },
    /// Execution paused
    ExecutionPaused { action_id: String },
}

impl RuntimeEvent {
    /// Map to front-end NuphusEvent
    pub fn to_nuphus_event(&self) -> NuphusEvent {
        match self.clone() {
            RuntimeEvent::UserMessageReceived {
                content,
                source,
                images,
            } => NuphusEvent::UserMessageReceived {
                content,
                source,
                images,
            },
            RuntimeEvent::ExecutionStarted { mode, goal, source } => {
                NuphusEvent::ExecutionStarted {
                    step_index: 0,
                    goal,
                    tools: vec![],
                    source,
                    mode: mode.to_string(),
                }
            }
            RuntimeEvent::LlmTextDelta {
                text,
                is_thinking,
                from_task,
            } => NuphusEvent::LlmTextDelta {
                text,
                is_thinking,
                from_task,
            },
            RuntimeEvent::ToolCallStart { tool_name, params } => NuphusEvent::ToolCallStart {
                call_id: String::new(),
                tool_name,
                params,
                iteration: 0,
                from_task: false,
            },
            RuntimeEvent::ToolCallEnd {
                tool_name,
                success,
                duration_ms,
                output_preview,
            } => {
                let preview_len = output_preview.len();
                NuphusEvent::ToolCallEnd {
                    call_id: String::new(),
                    tool_name,
                    success,
                    duration_ms,
                    output_preview,
                    output_full_size: preview_len,
                    is_truncated: false,
                    error: None,
                    from_task: false,
                }
            }
            RuntimeEvent::ExecutionCompleted {
                message,
                total_calls,
                total_duration_ms,
            } => NuphusEvent::ExecutionCompleted {
                step_index: 0,
                output: StepOutput {
                    step_index: 0,
                    result_message: message,
                    artifacts: vec![],
                    tool_calls_count: total_calls,
                },
                total_duration_ms,
                total_calls,
            },
            RuntimeEvent::Error {
                code,
                message,
                retryable,
            } => NuphusEvent::Error {
                code,
                message,
                retryable,
                from_subtask: false,
            },
            RuntimeEvent::TokenUsage {
                input_tokens,
                output_tokens,
                cache_hit_tokens,
            } => NuphusEvent::TokenUsage {
                input_tokens,
                output_tokens,
                cache_hit_tokens,
                source: "main".to_string(),
            },
            RuntimeEvent::SystemMessage { message } => NuphusEvent::DirectResponse { message },
            RuntimeEvent::SecurityCheck {
                action_id,
                tool,
                reason,
            } => NuphusEvent::SecurityCheck {
                action_id,
                tool,
                params: String::new(),
                risk: crate::agent::events::RiskLevel::Medium,
                reason,
            },
            RuntimeEvent::ExecutionPaused { action_id } => {
                NuphusEvent::ExecutionPaused { action_id }
            }
        }
    }

    /// Emit directly to EventEmitter
    pub fn emit(&self, emitter: &dyn EventEmitter) {
        emitter.emit(self.to_nuphus_event());
    }
}

/// Runtime builder — replaces old `build_agent()` + `run_leader_with_config()`
pub struct RuntimeBuilder {
    llm: Option<Arc<dyn ApiClient>>,
    tools: Option<ToolRegistry>,
    config: RuntimeConfig,
    emitter: Option<Arc<dyn EventEmitter>>,
    pause_flag: Option<Arc<AtomicBool>>,
    client_factory: Option<crate::llm::ClientFactory>,
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self {
            llm: None,
            tools: None,
            config: RuntimeConfig::default(),
            emitter: None,
            pause_flag: None,
            client_factory: None,
        }
    }

    pub fn llm(mut self, llm: Arc<dyn ApiClient>) -> Self {
        self.llm = Some(llm);
        self
    }

    pub fn tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn config(mut self, config: RuntimeConfig) -> Self {
        self.config = config;
        self
    }

    pub fn mode(mut self, mode: Mode) -> Self {
        self.config.mode = mode;
        self
    }

    pub fn emitter(mut self, emitter: Arc<dyn EventEmitter>) -> Self {
        self.emitter = Some(emitter);
        self
    }

    pub fn pause_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.pause_flag = Some(flag);
        self
    }

    pub fn client_factory(mut self, factory: crate::llm::ClientFactory) -> Self {
        self.client_factory = Some(factory);
        self
    }

    /// Build Runtime (replaces old `build_agent()`)
    pub fn build(self) -> std::result::Result<Runtime, String> {
        let llm = self
            .llm
            .ok_or_else(|| "RuntimeBuilder: llm is required".to_string())?;
        let tools = self
            .tools
            .ok_or_else(|| "RuntimeBuilder: tools is required".to_string())?;

        let mut agent =
            ReactAgent::new(llm.clone(), tools.clone(), self.config.agent_config.clone());

        // Pass pause_flag to agent (react_loop reads via agent.pause_flag)
        if let Some(ref pf) = self.pause_flag {
            agent.set_pause_flag(Some(pf.clone()));
        }

        let tool_permissions = self.config.tool_permissions.clone();

        let mut runtime = Runtime {
            config: self.config,
            llm,
            agent,
            emitter: self.emitter,
            tool_permissions,
            injected_annotations: HashSet::new(),
            source: "desktop".to_string(),
        };

        // 视觉能力判定：capabilities.vision > 主模型 supports_vision > 无
        // 不依赖 client_factory — 任何 session 都应正确判定
        let vision_model = match crate::config::resolve_vision_strategy() {
            crate::config::VisionStrategy::Capability(name) => Some(name),
            crate::config::VisionStrategy::Main => Some(runtime.agent.config.model.clone()),
            crate::config::VisionStrategy::None => None,
        };
        // 主模型 supports_vision：从配置/builtin 读取，不依赖 resolve_vision_strategy 的结论
        let main_supports_vision = crate::config::load_registry()
            .ok()
            .and_then(|r| {
                r.find_model(&runtime.agent.config.model)
                    .map(|(_, m)| m.supports_vision)
            })
            .unwrap_or(false);
        runtime.agent.config.vision_model = vision_model;
        runtime.agent.config.supports_vision = main_supports_vision;

        // 主模型 supports_image_generation：从配置/builtin 读取
        let main_supports_image_gen = crate::config::load_registry()
            .ok()
            .and_then(|r| {
                r.find_model(&runtime.agent.config.model)
                    .map(|(_, m)| m.supports_image_generation)
            })
            .unwrap_or(false);
        runtime.agent.config.supports_image_generation = main_supports_image_gen;

        if let Some(ref factory) = self.client_factory {
            runtime.agent.set_client_factory(factory.clone());
        }

        Ok(runtime)
    }
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Runtime configuration
#[derive(Clone)]
pub struct RuntimeConfig {
    /// Current mode
    pub mode: Mode,
    /// Agent configuration
    pub agent_config: AgentConfig,
    /// Context refine threshold
    pub refine_threshold: f64,
    /// Shared tool permissions (updated by Tauri layer, read by runtime before each tool call)
    #[allow(clippy::type_complexity)]
    pub tool_permissions: Arc<std::sync::Mutex<ToolPermissions>>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            mode: Mode::Leader,
            agent_config: AgentConfig::default(),
            refine_threshold: 0.5,
            tool_permissions: Arc::new(std::sync::Mutex::new(ToolPermissions::default())),
        }
    }
}

/// Runtime main loop — unified entry point
///
/// Replaces the full lifecycle of old `build_agent()` + `run_leader_with_config()`.
/// ReactAgent::run() is inlined here, ReactAgent is a pure state container.
pub struct Runtime {
    pub(crate) config: RuntimeConfig,
    llm: Arc<dyn ApiClient>,
    pub(crate) agent: ReactAgent,
    emitter: Option<Arc<dyn EventEmitter>>,
    /// Shared tool permissions — read before each tool call for real-time policy updates
    pub(crate) tool_permissions: Arc<std::sync::Mutex<ToolPermissions>>,
    injected_annotations: HashSet<String>,
    /// Message source marker ("desktop" | "mobile"), stamped on UserMessageReceived / ExecutionStarted
    source: String,
}

impl Runtime {
    // ── Lifecycle ──

    /// Set Leader context (soul + relation), called before each message round
    pub fn set_context(&mut self, soul: &str, relation: Option<RelationConfig>) {
        self.agent.set_leader_context(LeaderContext {
            soul: soul.to_string(),
            relation: relation.clone(),
        });
    }

    /// Set message source marker ("desktop" | "mobile"), called before each message round.
    /// Default "desktop"; the mobile HTTP entry sets "mobile" so events carry the origin.
    pub fn set_source(&mut self, source: &str) {
        self.source = source.to_string();
    }

    /// Take event emitter (refine 等内部流程临时静默生命周期事件用，run 后 restore)
    pub fn take_emitter(&mut self) -> Option<Arc<dyn EventEmitter>> {
        self.emitter.take()
    }

    /// Restore event emitter after internal silent run
    pub fn restore_emitter(&mut self, emitter: Option<Arc<dyn EventEmitter>>) {
        self.emitter = emitter;
    }

    /// Restore session from history (when creating a new agent and frontend requests history recovery)
    pub fn set_history(&mut self, history: &[(String, String)]) {
        if !history.is_empty() && self.agent.session().is_empty() {
            let session = Session::from_history(history.to_vec());
            self.agent.set_session(session);
        }
    }

    /// Set execution resources (for task_dispatch)
    pub fn set_exec_resources<E: EventEmitter + 'static>(
        &mut self,
        exec_tools: ToolRegistry,
        exec_llm: Arc<dyn ApiClient>,
        emitter: E,
    ) {
        self.agent.set_exec_resources(exec_tools, exec_llm, emitter);
    }

    /// Get current mode
    pub fn mode(&self) -> Mode {
        self.config.mode
    }

    /// Current exec (subtask) model name — used by orchestrator to detect
    /// agent-level exec model changes that require a Runtime rebuild.
    pub fn exec_model(&self) -> String {
        self.agent
            .exec_llm
            .as_ref()
            .map(|l| l.model_name().to_string())
            .unwrap_or_default()
    }

    /// Switch mode — handles WorkflowAgent session lifecycle
    pub fn set_mode(&mut self, mode: Mode) {
        let old_mode = self.config.mode;
        self.config.mode = mode;
        // Mode change swaps L2 (Custom uses its own card) → rebuild merged prompt.
        if old_mode != mode {
            self.agent.invalidate_prompt_cache();
        }
        // Custom 状态应用：白名单 + 记忆归属锚点（缓存失效由上方统一处理）。
        let active_custom = if mode == Mode::Custom {
            crate::custom_agents::CustomAgentStore::get_active()
        } else {
            None
        };
        self.apply_custom_card_state(active_custom.as_ref());
        match (old_mode, mode) {
            (Mode::Workflow, Mode::Leader) => {
                tracing::info!("[MODE] Exiting Workflow mode, WorkflowAgent session preserved");
            }
            (Mode::Leader, Mode::Workflow) => {
                tracing::info!("[MODE] Entering Workflow mode");
            }
            (_, Mode::Custom) => {
                tracing::info!("[MODE] Entering Custom mode");
            }
            (Mode::Custom, _) => {
                tracing::info!("[MODE] Exiting Custom mode");
            }
            _ => {}
        }
    }

    /// Apply a Custom card's runtime state: tool whitelist + memory isolation anchor.
    ///
    /// `None` clears both (leaving Custom mode). Does NOT invalidate prompt cache —
    /// callers handle that (set_mode does it on any change; set_active does it explicitly).
    /// Whitelist semantics: empty card.tools = no filtering → None (all tools allowed).
    pub fn apply_custom_card_state(
        &mut self,
        config: Option<&crate::custom_agents::CustomAgentConfig>,
    ) {
        self.agent.custom_tool_whitelist = config
            .filter(|c| c.has_tool_whitelist())
            .map(|c| c.tools.clone());
        crate::custom_agents::set_current_custom_agent_id(config.map(|c| c.id.clone()));
    }

    /// Inject workflow engine for workflow_run tool support (Leader)
    pub fn set_workflow_engine(
        &mut self,
        engine: Arc<tokio::sync::RwLock<crate::workflow::WorkflowEngine>>,
    ) {
        self.agent.set_workflow_engine(engine);
    }

    /// Get Agent config reference
    pub fn config(&self) -> &AgentConfig {
        &self.config.agent_config
    }

    /// Get Session reference
    pub fn session(&self) -> &Session {
        self.agent.session()
    }

    /// Get execution steps (read-only)
    pub fn steps(&self) -> &[crate::ExecutionStep] {
        self.agent.steps()
    }

    /// Get last output text
    pub fn last_output_text(&self) -> &Option<String> {
        self.agent.last_output_text()
    }

    /// Get all text outputs accumulated during the current react_loop turn
    pub fn all_turn_texts(&self) -> &[String] {
        self.agent.all_turn_texts()
    }

    /// Get LLM client reference
    pub fn llm(&self) -> &Arc<dyn ApiClient> {
        &self.llm
    }

    /// Get mutable Session reference
    pub fn session_mut(&mut self) -> &mut Session {
        self.agent.session_mut()
    }

    /// Set Session (used when restoring from history)
    pub fn set_session(&mut self, session: Session) {
        self.agent.set_session(session);
    }

    /// Get internal Agent reference
    pub fn agent(&self) -> &ReactAgent {
        &self.agent
    }

    /// Get internal mutable Agent reference
    pub fn agent_mut(&mut self) -> &mut ReactAgent {
        &mut self.agent
    }

    /// Consume Runtime to return internal Agent
    pub fn into_agent(self) -> ReactAgent {
        self.agent
    }

    // ── Proxy methods: delegate to internal ReactAgent ──

    pub async fn maybe_refine_session(
        &mut self,
        cancel_flag: &AtomicBool,
        context_window: usize,
        refine_threshold: f64,
    ) {
        self.agent
            .maybe_refine_session(cancel_flag, context_window, refine_threshold)
            .await
    }

    pub fn save_refine_entry(
        &self,
        summary: &str,
        source: &str,
    ) -> std::result::Result<(), String> {
        self.agent.save_refine_entry(summary, source)
    }

    pub fn into_steps(self) -> Vec<ExecutionStep> {
        self.agent.into_steps()
    }

    // ── Emit helpers ──
    fn emit(&self, event: RuntimeEvent) {
        if let Some(ref emitter) = self.emitter {
            event.emit(emitter.as_ref());
        }
    }

    // -- Main loop --

    /// Run main loop
    ///
    /// Flow:
    /// 1. Emit RuntimeEvent::UserMessageReceived
    /// 2. Emit ExecutionStarted
    /// 3. Auto-inject annotations matched from user input
    /// 4. Inline ReAct loop (react_loop)
    /// 5. Emit ExecutionCompleted
    pub async fn run(
        &mut self,
        input: &str,
        images: &Option<Vec<String>>,
        cancel_flag: &AtomicBool,
    ) -> Result<crate::AgentOutput> {
        // ── Session span: all child calls inherit session_id ──
        let session_span = tracing::info_span!("session", session_id = %self.session().id);
        let _session_enter = session_span.enter();

        let _start = std::time::Instant::now();

        // Clear steps from previous turn — each run() starts a fresh execution trace
        self.agent.steps.clear();

        // Clear accumulated texts from previous turn
        self.agent.all_turn_texts.clear();

        // Advance turn counter — each user message is one turn
        let turn_id = self.agent.session.advance_turn();
        tracing::info!("[TURN] Advance to {}", turn_id);

        let effective_input = input.to_string();

        // Auto-inject annotations: match keywords in user input (injected once per round)
        let effective_input = {
            tracing::info!("[Runtime] 开始注入标注...");
            let lower_input = input.to_lowercase();
            let mut matched: Vec<String> = Vec::new();

            let annotations = AnnotationStore::list();
            tracing::info!("[Runtime] 标注加载完成，共 {} 条", annotations.len());
            for a in &annotations {
                if self.injected_annotations.contains(&a.keyword) {
                    continue;
                }
                let kw = a.keyword.to_lowercase();
                // 双向匹配：用户输入包含关键词，或关键词包含用户输入（用户输入需 ≥3 字符，避免单字母误匹配）
                let mut matched_keyword = lower_input.contains(&kw)
                    || (lower_input.len() >= 3 && kw.contains(&lower_input));
                if !matched_keyword {
                    for ek in &a.keywords {
                        let ek_lower = ek.to_lowercase();
                        if lower_input.contains(&ek_lower)
                            || (lower_input.len() >= 3 && ek_lower.contains(&lower_input))
                        {
                            matched_keyword = true;
                            break;
                        }
                    }
                }
                if matched_keyword {
                    self.injected_annotations.insert(a.keyword.clone());
                    matched.push(format!("[相关标注: {}] {}", a.keyword, a.description));
                }
            }

            if matched.is_empty() {
                effective_input
            } else {
                format!("{}\n\n{}", matched.join("\n"), effective_input)
            }
        };

        // 1-2. Emit unified events
        self.emit(RuntimeEvent::UserMessageReceived {
            content: effective_input.clone(),
            source: self.source.clone(),
            images: images.clone().unwrap_or_default(),
        });
        self.emit(RuntimeEvent::ExecutionStarted {
            mode: self.config.mode,
            goal: effective_input.chars().take(120).collect(),
            source: self.source.clone(),
        });

        tracing::info!(
            "[Runtime] mode={}, input={:?}",
            self.config.mode,
            effective_input.chars().take(80).collect::<String>()
        );

        // 4. Inline ReAct loop
        let output = self
            .react_loop(&effective_input, images, cancel_flag, false)
            .await?;

        // ExecutionCompleted is emitted by the orchestrator (process.rs), Runtime does not re-emit
        // Runtime is only responsible for the execution lifecycle; completion events are decided by upper orchestration when and with what data to emit
        Ok(output)
    }

    /// 断点续跑（重试专用）
    ///
    /// 与 run() 的差异：不 advance_turn、不 push_user、不做 mode/标注注入。
    /// LLM 调用失败时错误内容从未进入 session（流式内容在缓冲区，成功才落库），
    /// strip_incomplete_tools 又已清理悬挂工具对——session 末尾即断点
    /// （user 消息或工具结果），直接进入 ReAct 循环继续即可。
    ///
    /// 效果：失败回合的全部进度（含已完成的工具调用）完整保留；
    /// turn id 不变 → 记忆持久化按 entry id 覆盖失败记录，无噪声；
    /// 0 进度失败时 session 末尾正是 user 消息，行为与重发等价（路径 1 的严格超集）。
    pub async fn resume(
        &mut self,
        goal: &str,
        cancel_flag: &AtomicBool,
    ) -> Result<crate::AgentOutput> {
        let session_span = tracing::info_span!("session", session_id = %self.session().id);
        let _session_enter = session_span.enter();

        if self.config.mode == Mode::Workflow {
            // workflow 失败不产生 pending_retry（retry_session=None），防御性兜底
            tracing::warn!("[RESUME] resume called in Workflow mode, continuing with react loop");
        }

        tracing::info!(
            "[RESUME] Resume turn {} at breakpoint ({} msgs)",
            self.agent.session.current_turn_id(),
            self.agent.session.len()
        );

        self.emit(RuntimeEvent::ExecutionStarted {
            mode: self.config.mode,
            goal: goal.chars().take(120).collect(),
            source: self.source.clone(),
        });

        // goal 作为 input 传入但不 push——供 wants_file_output 等回合级启发式使用
        self.react_loop(goal, &None, cancel_flag, true).await
    }
}