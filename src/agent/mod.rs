//! Agent module - ReAct core implementation
//!
//! Design decisions:
//! 1. Don't use dyn LLM/Memory, use generic constraints
//! 2. ReactAgent directly holds LLM/Session/Tools/Policy
//! 3. Use streaming API event processing

pub mod common;
pub mod diagnostics;
pub mod distill;
pub mod events;
pub mod exec_tool;
pub mod goal_types;
pub mod pause;
pub mod prompt;
pub mod reminders;
use crate::agent::events::{EventEmitter, NuphusEvent};
use crate::agent::reminders::ReminderQueue;
use crate::{
    api::ApiClient,
    hooks::{HookConfig, HookRunner},
    permissions::{PermissionPolicy, ToolCategory, ToolPermissions},
    session::Session,
    ExecutionStep, Result, ToolCall, ToolRegistry, ToolResult,
};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Agent configuration
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub model: String,
    pub provider: String,
    pub max_iterations: usize,
    pub enable_memory: bool,
    pub tool_permissions: ToolPermissions,
    /// Context refine threshold (0.0~1.0, default 0.50)
    pub refine_threshold: f64,
    /// Shell Hooks configuration
    pub hooks: HookConfig,
    /// 视觉模型（None=未配置，Some=模型名）
    /// 可能来自 capabilities.vision 显式配置，或主模型自身支持
    pub vision_model: Option<String>,
    /// 主模型是否原生支持视觉（来自 ModelEntry/ModelDef.supports_vision）
    pub supports_vision: bool,
    /// 主模型是否原生支持图片生成（来自 ModelEntry/ModelDef.supports_image_generation）
    pub supports_image_generation: bool,
    /// 推理深度（reasoning_effort，来自 config.toml [[providers]] reasoning_effort）。
    /// None = 未配置。作为 config-match 的一部分：修改 effort 会触发 Runtime 重建，
    /// 使 Leader 客户端立即使用新 effort。
    pub reasoning_effort: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        // Default model from ProviderRegistry metadata, avoid hardcoding a single model
        let registry = crate::config::registry::ProviderRegistry::builtin();
        let provider = registry.get("minimax").unwrap_or_else(|| {
            registry
                .get("deepseek")
                .expect("builtin registry missing deepseek provider")
        });
        Self {
            model: provider.default_model().to_string(),
            provider: provider.id().to_string(),
            max_iterations: crate::agent::goal_types::GoalType::MAX_ITERATIONS,
            enable_memory: true,
            tool_permissions: ToolPermissions::default(),
            refine_threshold: 0.5,
            hooks: HookConfig::default(),
            vision_model: None,
            supports_vision: false,
            supports_image_generation: false,
            reasoning_effort: None,
        }
    }
}

/// Tool calls extracted from text (via <tool_call> tags)
#[derive(Debug, Clone)]
pub(crate) struct TextToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Parse XML tool call tags from LLM text response.
///
/// By default only recognises `<tool_call>...</tool_call>`. Pass additional
/// tag names in `extra_tags` to handle provider-specific formats (e.g.
/// `&["function_call"]` for MiniMax).
///
/// Returns (cleaned text, extracted tool calls list).
pub(crate) fn extract_tool_calls_from_text_with_tags(
    text: &str,
    extra_tags: &[&str],
) -> (String, Vec<TextToolCall>) {
    let mut result = text.to_string();
    let mut calls = Vec::new();
    let mut counter: u32 = 0;

    // Build full tag list: built-in + provider extras
    let all_tags: Vec<&str> = std::iter::once("tool_call")
        .chain(extra_tags.iter().copied())
        .collect();

    for tag_name in &all_tags {
        let open_tag = format!("<{}>", tag_name);
        let close_tag = format!("</{}>", tag_name);

        while let Some(open_pos) = result.find(&open_tag) {
            let search_from = open_pos + open_tag.len();
            let close_pos = match result[search_from..].find(&close_tag) {
                Some(p) => search_from + p,
                None => break,
            };

            let json_content = result[search_from..close_pos].trim().to_string();
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_content) {
                let name = parsed
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let arguments = parsed
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                if !name.is_empty() {
                    counter += 1;
                    calls.push(TextToolCall {
                        id: format!("text-call-{}", counter),
                        name,
                        arguments,
                    });
                }
            }
            // Remove entire <tag>...</tag>
            result.replace_range(open_pos..close_pos + close_tag.len(), "");
        }
    }

    (result.trim().to_string(), calls)
}

/// ReactAgent — state container for ReAct loop execution.
/// Runtime owns the execution loop; ReactAgent holds session, tools, policy, and LLM connection.
/// SubTaskRunner (Exec) handles focused tool execution; ReactAgent handles global conversation and fallback retries.
pub struct ReactAgent {
    pub(crate) llm: Arc<dyn ApiClient>,
    pub(crate) tools: ToolRegistry,
    pub(crate) policy: PermissionPolicy,
    pub(crate) config: AgentConfig,
    pub(crate) session: Session,
    pub(crate) steps: Vec<ExecutionStep>,
    /// Shell Hooks runtime (initialized from config.hooks)
    pub(crate) hooks: Option<HookRunner>,
    /// Full tool registry (includes system_shell, file_edit, desktop_*, etc.),
    /// Used for creating ExecuteAgent during task_dispatch
    pub(crate) exec_tools: Option<ToolRegistry>,
    /// LLM instance used by execution Agent (same connection pool as main LLM)
    pub(crate) exec_llm: Option<Arc<dyn ApiClient>>,
    /// Event emitter for execution Agent
    pub(crate) exec_emitter: Option<Arc<dyn EventEmitter>>,
    /// Leader prompt context (soul/relation)
    pub(crate) leader_ctx: prompt::LeaderContext,
    /// Model switching factory (for runtime model switching)
    pub(crate) client_factory: Option<crate::llm::ClientFactory>,
    /// Pause flag (shared with AppState, inherited by dispatch Exec)
    pub(crate) pause_flag: Option<Arc<AtomicBool>>,
    /// 强制提炼计数器（防止无限循环提炼）
    pub refine_count: u32,
    /// 注意力提示标记（每次 refine 后重置，大窗口专用，每 refine 周期一次）
    pub context_hint_shown: bool,
    /// 上次触发记忆保存提示时的 token 数（每 100K 递增触发）
    pub last_memory_hint_at: u64,
    /// L0 Kernel cache (identity/run loop/tool priority/execution/verification/completion/safety/priority)
    /// Built only once per session lifecycle to maximize prefix caching hit rate
    pub(crate) cached_base_prompt: Option<String>,
    /// L1 dynamic prompt cache (l2_leader + schemas + tenets + cross-phase context + environment)
    /// Like L0, fixed session content, built once and reused permanently
    pub(crate) cached_l1_prompt: Option<Vec<String>>,
    /// Merged system prompt cache: L0 + L2 + L1 combined into a single system message.
    /// Built once per session, never rebuilt — guarantees 100% KV cache prefix stability.
    pub(crate) cached_merged_system_prompt: Option<String>,
    /// Tool schemas cache — built once per session, deterministic prefix for DeepSeek prompt cache
    pub(crate) cached_tools_schemas: Option<Vec<crate::api::ToolDefinition>>,
    /// Custom mode tool whitelist (None = no filtering). Set on mode switch:
    /// Some(card.tools) when entering Custom with a non-empty whitelist, else None.
    /// Gates three choke points: prompt tool list, API function-calling schemas, execution.
    pub(crate) custom_tool_whitelist: Option<Vec<String>>,
    /// Last run's final output text (same source as DirectResponse event,
    /// learn_from_success uses this instead of scanning session for assistant_message)
    pub(crate) last_output_text: Option<String>,
    /// All text outputs accumulated across the entire react_loop turn
    /// (reset per run(); captures every text block the model produces,
    /// including intermediate analysis before tool calls, not just the final reply)
    pub(crate) all_turn_texts: Vec<String>,
    /// Consecutive safety check failure count (accumulated in execute_tool_with_permission, circuit-break after 3)
    pub(crate) safety_consecutive_failures: u32,
    /// Exec Agent pool: multiple tasks of the same plan reuse the same Exec session,
    /// avoiding repeated reads of project structure and key files.
    /// key = plan_path (adhoc tasks use "adhoc:{goal_type}")
    pub(crate) exec_agent_pool: HashMap<String, crate::runtime::SubTaskRunner>,
    /// Multi-turn persistent reminder queue
    pub reminders: ReminderQueue,
    /// Workflow engine reference (injected by Runtime for workflow_run tool)
    pub(crate) workflow_engine: Option<Arc<tokio::sync::RwLock<crate::workflow::WorkflowEngine>>>,
}

/// Detect if user input has an explicit "execution tone" (imperative/command), not query/discussion/thinking.
///
/// Positive signals: contains imperative words like "help me" "run" "create" etc.
/// Negative signals: contains obvious query words like "analyze" "what is" "why" etc.
impl ReactAgent {
    pub fn new(llm: Arc<dyn ApiClient>, tools: ToolRegistry, config: AgentConfig) -> Self {
        let mut policy = PermissionPolicy::new(config.tool_permissions);
        // Extract permission requirements from all tools in ToolRegistry
        // Register both internal name (file_read) and API flat name (file_read), since LLM may return either
        {
            let mut reqs: Vec<(String, ToolCategory)> = Vec::new();
            for def in tools.all_defs() {
                reqs.push((def.name.clone(), def.category));
                let api_name = def.name.clone();
                reqs.push((api_name, def.category));
            }
            policy = policy.with_categories(reqs);
        }
        // Register all tool categories (ToolDef + desktop/browser schemas — dynamically collected)
        policy = policy.with_categories(tools.all_tool_categories());
        let hooks = if config.hooks.has_any_hook() {
            Some(HookRunner::new(config.hooks.clone()))
        } else {
            None
        };
        Self {
            llm,
            tools,
            policy,
            config,
            session: Session::new(),
            steps: Vec::new(),
            hooks,
            exec_tools: None,
            exec_llm: None,
            exec_emitter: None,
            leader_ctx: prompt::LeaderContext::default(),
            client_factory: None,
            pause_flag: None,
            refine_count: 0,
            context_hint_shown: false,
            last_memory_hint_at: 0,
            cached_base_prompt: None,
            cached_l1_prompt: None,
            cached_merged_system_prompt: None,
            cached_tools_schemas: None,
            custom_tool_whitelist: None,
            last_output_text: None,
            all_turn_texts: Vec::new(),
            safety_consecutive_failures: 0,
            exec_agent_pool: HashMap::new(),
            reminders: ReminderQueue::new(),
            workflow_engine: None,
        }
    }

    /// Inject workflow engine for workflow_run tool support
    pub(crate) fn set_workflow_engine(
        &mut self,
        engine: Arc<tokio::sync::RwLock<crate::workflow::WorkflowEngine>>,
    ) {
        self.workflow_engine = Some(engine);
    }

    /// Set Leader prompt context (soul/relation)
    ///
    /// 缓存第一：merged system prompt 每 session 只构建一次。
    /// 一旦已构建，后续 set_leader_context 调用直接忽略——soul/relation/
    /// tenets/memory.md 的中途变更一律下个 session 才生效，
    /// 保证 provider 侧 prompt cache 前缀在整个 session 内稳定。
    pub fn set_leader_context(&mut self, ctx: prompt::LeaderContext) {
        if self.cached_merged_system_prompt.is_some() {
            return;
        }
        self.leader_ctx = ctx;
        self.cached_base_prompt = None;
        self.cached_l1_prompt = None;
        self.cached_merged_system_prompt = None;
        self.cached_tools_schemas = None;
    }

    /// Invalidate prompt/tool-schema caches without touching leader_ctx.
    /// Called on mode switch (Leader ↔ Workflow ↔ Custom) because Custom swaps
    /// the L2 block and narrows the tool whitelist — cached merged prompt must rebuild.
    pub fn invalidate_prompt_cache(&mut self) {
        self.cached_base_prompt = None;
        self.cached_l1_prompt = None;
        self.cached_merged_system_prompt = None;
        self.cached_tools_schemas = None;
    }

    /// Set task_dispatch execution resources (full tools + LLM + event emitter)
    pub fn set_exec_resources<E: EventEmitter + 'static>(
        &mut self,
        tools: ToolRegistry,
        llm: Arc<dyn ApiClient>,
        emitter: E,
    ) {
        self.exec_tools = Some(tools);
        self.exec_llm = Some(llm);
        let arc: Arc<dyn EventEmitter> = Arc::new(emitter);
        self.exec_emitter = Some(arc);
    }

    /// Build API request with merged system prompt (L0+L2+L1 as single message)
    pub(crate) fn build_request(
        &mut self,
        merged_system_prompt: &str,
    ) -> crate::api::MessageRequest {
        let messages = self.session.to_api_messages(self.config.supports_vision);
        let tools = self
            .cached_tools_schemas
            .get_or_insert_with(|| {
                // Custom mode whitelist narrows the API-visible function-calling schemas.
                let schemas = match &self.custom_tool_whitelist {
                    Some(wl) => self.tools.get_schemas_for(wl),
                    None => self.tools.get_schemas(),
                };
                tracing::info!(
                    "[PromptCache] Tool schemas cached ({} tools) for session",
                    schemas.len()
                );
                schemas
            })
            .clone();

        crate::api::MessageRequest::new(&self.config.model, messages)
            .with_merged_system(merged_system_prompt)
            .with_tools(tools)
    }

    /// Set model factory (for runtime model switching)
    pub fn set_client_factory(&mut self, factory: crate::llm::ClientFactory) {
        self.client_factory = Some(factory);
    }

    /// Set pause flag (shared with AppState, inherited by dispatch subtasks)
    pub fn set_pause_flag(&mut self, flag: Option<Arc<AtomicBool>>) {
        self.pause_flag = flag;
    }

    /// Load cross-phase context (historical conversation refine titles + current memory.md)
    /// 1. Read recent N titles/IDs with goal_type=session_refine from SQLite (guide LLM to self-check memory)
    /// 2. Read .nuphus/memory.md as current state reference
    ///
    /// Return format:
    ///   [Historical Conversation Refine]
    ///   - <title> (ID: <refine-id>)
    ///   [Current State]
    ///   <memory.md content>
    pub fn load_cross_session_context() -> Option<String> {
        let mut parts: Vec<String> = Vec::new();

        // Part 1: Recent distill record titles+IDs (to guide LLM to call memory_search for self-check)
        if let Ok(entries) = crate::store::memory::search_entries_filtered(
            None,
            None,
            None,
            None,
            Some(crate::memory::entry::MemoryKind::Distill),
            None,
            None,
            None,
            None,
            None,
            None,
            5,
        ) {
            let distill_titles: Vec<String> = entries
                .iter()
                .filter(|e| !e.intent.is_empty() || !e.summary.is_empty())
                .map(|e| {
                    let title = if e.intent.starts_with('#') || e.intent.len() < 10 {
                        e.summary.chars().take(100).collect::<String>()
                    } else {
                        e.intent.clone()
                    };
                    format!("- {} (ID: {})", title, e.id)
                })
                .collect();
            if !distill_titles.is_empty() {
                parts.push(format!(
                    "## 历史对话提炼（标题可用于检索，LLM自查）\n{}",
                    distill_titles.join("\n")
                ));
            }
        }

        // Part 2: Current memory.md
        let md_path = crate::utils::nuphus_data_dir().join("memory.md");
        if md_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&md_path) {
                let trimmed = content.trim().to_string();
                if !trimmed.is_empty() {
                    parts.push(format!("## Leader Memory Update (跨会话传递)\n{}", trimmed));
                }
            }
        }

        if parts.is_empty() {
            return None;
        }
        Some(parts.join("\n\n"))
    }

    /// Read .nuphus/memory.md as cross-phase session reference (kept old function name as alias)
    /// Now delegates to load_cross_session_context, returns merged refine titles
    /// Switch current model
    pub fn switch_model(&mut self, model_id: &str) -> Result<()> {
        if let Some(ref factory) = self.client_factory {
            let new_client = factory.create_client(model_id).map_err(|e| {
                crate::NuphusError::Agent(crate::AgentError::ModelSwitchFailed {
                    error: format!("switch model failed: {}", e),
                })
            })?;
            self.llm = new_client;
            self.config.model = model_id.to_string();
            tracing::info!("Switched to model: {}", model_id);
            Ok(())
        } else {
            Err(crate::NuphusError::Agent(
                crate::AgentError::ModelSwitchFailed {
                    error: "ClientFactory not set, cannot switch model".to_string(),
                },
            ))
        }
    }

    /// Execute a single tool call (with full-chain safety checks)
    ///
    /// Permission/safety checks delegated to exec_tool::check_tool_security,
    /// hooks remain at this layer as a Leader-exclusive feature.
    pub(crate) async fn execute_tool_with_permission(
        &mut self,
        call: &ToolCall,
        post_tool_warnings: &mut Vec<String>,
        cancel_flag: &AtomicBool,
    ) -> Result<ToolResult> {
        // 0. Custom mode tool whitelist — hard gate before any permission/security check.
        //    Match both API name (underscore) and internal name (::) to prevent bypass.
        if let Some(ref wl) = self.custom_tool_whitelist {
            let api_name = call.tool.replace("::", "_");
            let allowed = wl.iter().any(|w| w == &call.tool || w == &api_name);
            if !allowed {
                return Ok(ToolResult::failure(format!(
                    "Tool '{}' is not enabled for this Custom agent (whitelist)",
                    call.tool
                )));
            }
        }

        // 1. Permission check + SecurityGuard unified detection
        if let Some(blocked) = exec_tool::check_tool_security(
            self.tools.signals(),
            &call.tool,
            &call.params,
            &self.policy,
            self.exec_emitter.as_deref(),
            cancel_flag,
            Some(&self.policy.permissions()),
            &mut self.safety_consecutive_failures,
            post_tool_warnings,
        )
        .await
        {
            match exec_tool::breaker_check(self.safety_consecutive_failures) {
                exec_tool::BreakerAction::Halt => {
                    self.session.strip_incomplete_tools();
                    return Err(crate::NuphusError::agent(format!(
                        "执行中止:连续 {} 次安全检查未通过",
                        self.safety_consecutive_failures
                    )));
                }
                exec_tool::BreakerAction::Warn | exec_tool::BreakerAction::Restrict => {
                    if let Some(msg) = exec_tool::breaker_message(self.safety_consecutive_failures)
                    {
                        self.session.push_system(msg);
                    }
                }
                exec_tool::BreakerAction::None => {}
            }
            return Ok(blocked);
        }

        // 2. Hook: pre-tool (Leader exclusive)
        if let Some(ref hooks) = self.hooks {
            let allowed = hooks.run_pre_tool_call(&call.tool, &call.params);
            if !allowed {
                return Ok(ToolResult::failure(format!("Hook vetoed: {}", call.tool)));
            }
        }

        // 3. Execute tool (with per-line output callback)
        let call_id = call.id.clone();

        // HUD: emit running for desktop/browser/system_shell tools
        let needs_hud = call.tool.starts_with("desktop_")
            || call.tool.starts_with("browser_")
            || call.tool == "system_shell";
        if needs_hud {
            if let Some(ref emitter) = self.exec_emitter {
                emitter.emit(NuphusEvent::HudUpdate {
                    text: format!("{} — 执行中...", call.tool),
                    phase: "running".into(),
                    step_kind: None,
                });
            }
        }

        let on_line: Option<Box<dyn Fn(String, bool) + Send>> =
            self.exec_emitter.as_ref().map(|emitter| {
                let emitter = emitter.clone();
                let cb: Box<dyn Fn(String, bool) + Send> =
                    Box::new(move |line: String, is_stderr: bool| {
                        emitter.emit(NuphusEvent::ToolOutputLine {
                            call_id: call_id.clone(),
                            line,
                            is_stderr,
                        });
                    });
                cb
            });
        // Inject session_id for memory_snapshot tools so they can link back to real session
        let params = if call.tool == "leader_memory_update" || call.tool == "leader::memory_update"
        {
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
        let result = exec_tool::execute_tool_only(
            &self.tools,
            &call.tool,
            &params,
            on_line,
            self.exec_emitter.as_deref(),
        )
        .await;

        // 4. Post-execution: if request_user_input, emit UserInputRequest and wait for user response
        if call.tool == "request_user_input" && result.success {
            if let Some(ref output) = result.output {
                if let Some(action_id) = exec_tool::extract_action_id(output) {
                    if let Some(emitter) = &self.exec_emitter {
                        if let Some(pending) =
                            crate::security::user_input::get(self.tools.signals(), &action_id)
                        {
                            emitter.emit(NuphusEvent::UserInputRequest {
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
                    }
                    let value = exec_tool::wait_for_user_input_async(
                        self.tools.signals(),
                        &action_id,
                        cancel_flag,
                        self.exec_emitter.as_deref(),
                    )
                    .await;
                    match value {
                        Some(v) => {
                            // 非文本类型返回 JSON 字符串，解析后传给 LLM 以便直接使用 path/region 等字段
                            let parsed: serde_json::Value =
                                serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v));
                            return Ok(ToolResult::success(parsed.to_string()));
                        }
                        None => {
                            return Ok(ToolResult::failure(
                                "用户未提供输入或操作已取消".to_string(),
                            ))
                        }
                    }
                }
            }
        }

        // 5. Hook: post-tool
        if let Some(ref hooks) = self.hooks {
            hooks.run_post_tool_call(&call.tool, &call.params, &result);
        }

        // HUD: emit done/error for desktop/browser/system_shell tools
        if needs_hud {
            if let Some(ref emitter) = self.exec_emitter {
                let (phase, desc) = if result.success {
                    ("done", "完成")
                } else {
                    ("error", "失败")
                };
                emitter.emit(NuphusEvent::HudUpdate {
                    text: format!("{} — {}", call.tool, desc),
                    phase: phase.into(),
                    step_kind: None,
                });
            }
        }

        Ok(result)
    }

    /// Get Agent config reference
    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    /// Get session reference
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Get execution steps (read-only)
    pub fn steps(&self) -> &[ExecutionStep] {
        &self.steps
    }

    /// Get last output text
    pub fn last_output_text(&self) -> &Option<String> {
        &self.last_output_text
    }

    /// Get all text outputs accumulated during the current turn
    pub fn all_turn_texts(&self) -> &[String] {
        &self.all_turn_texts
    }

    /// Get mutable session reference
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Set session (for restoring from history)
    pub fn set_session(&mut self, session: Session) {
        self.session = session;
        self.steps.clear();
    }

    /// Consume Agent and return execution steps (for persisting timeline)
    pub fn into_steps(self) -> Vec<ExecutionStep> {
        self.steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::ContentBlock;

    #[test]
    fn test_default_config() {
        let config = AgentConfig::default();
        assert_eq!(
            config.max_iterations,
            crate::agent::goal_types::GoalType::MAX_ITERATIONS
        );
        assert!(config.enable_memory);
        assert!(config.tool_permissions.file_access);
        assert!(config.tool_permissions.web_search);
        assert!(!config.tool_permissions.system_automation);
    }

    #[test]
    fn test_extract_tool_calls() {
        let blocks = [
            ContentBlock::Text {
                text: "Hello".to_string(),
                reasoning: None,
            },
            ContentBlock::ToolUse {
                id: "tool-1".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({"path": "/tmp/test"}),
            },
        ];

        // Directly call top-level function test
        let calls: Vec<ToolCall> = blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, name, input } => Some(ToolCall {
                    id: id.clone(),
                    tool: name.clone(),
                    params: input.clone(),
                }),
                _ => None,
            })
            .collect();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool, "Read");
    }
}
