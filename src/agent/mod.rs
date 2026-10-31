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
pub mod task_run;
pub mod turn_meta;
use crate::agent::events::{EventEmitter, NuphusEvent};
use crate::agent::reminders::ReminderQueue;
use crate::{
    api::ApiClient,
    hooks::{HookConfig, HookRunner},
    permissions::{PermissionPolicy, ToolCategory, ToolPermissions},
    session::{PromptSnapshot, Session},
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
    /// Large-window force-refine ratio (0.50~0.80, default 0.80).
    /// Only takes effect for `RefineTier::Large` (context_window > 600K);
    /// Small/Medium tiers ignore it and use their fixed ratios.
    pub large_force_refine_threshold: f64,
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
            large_force_refine_threshold: crate::agent::distill::LARGE_FORCE_DEFAULT,
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
            // Close tag search accepts optional whitespace before '>' (e.g.
            // `</tool_call >`) and reports the matched length so the whole
            // tag is removed. No close → break: the residual block is stripped
            // by the terminal cleaner in process_events.
            let (close_pos, close_len) =
                match crate::utils::close_tag_search(&result[search_from..], &close_tag) {
                    Some((p, l)) => (search_from + p, l),
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
            // Remove entire <tag>...</tag> (close_len covers any whitespace before '>')
            result.replace_range(open_pos..close_pos + close_len, "");
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
    /// 本轮执行的元数据累加器（耗时 / token / 步数）。
    ///
    /// 生命周期与「一轮用户请求」对齐：收到输入时 `reset_for_turn()`，
    /// 每次 LLM 调用累加 token，完成时作为 `ExecutionCompleted.meta` 下发。
    /// 单一数据源——消息底部 / 执行面板 / ctx 弹窗共用，避免各处口径不一。
    pub(crate) turn_meta: crate::agent::turn_meta::TurnMeta,
    /// Shell Hooks runtime (initialized from config.hooks)
    pub(crate) hooks: Option<HookRunner>,
    /// Full tool registry (includes system_shell, file_edit, desktop_*, etc.),
    /// Used for creating ExecuteAgent during task_dispatch
    pub(crate) exec_tools: Option<ToolRegistry>,
    /// LLM instance used by execution Agent (same connection pool as main LLM)
    pub(crate) exec_llm: Option<Arc<dyn ApiClient>>,
    /// Event emitter for execution Agent
    pub(crate) exec_emitter: Option<Arc<dyn EventEmitter>>,
    /// ExecAgent 执行生命周期台账（task 面板的唯一数据源，进程级单例）
    pub(crate) task_runs: Option<Arc<task_run::TaskRunRegistry>>,
    /// Leader prompt context (soul/relation)
    pub(crate) leader_ctx: prompt::LeaderContext,
    /// Model switching factory (for runtime model switching)
    pub(crate) client_factory: Option<crate::llm::ClientFactory>,
    /// Pause flag (shared with AppState, inherited by dispatch Exec)
    pub(crate) pause_flag: Option<Arc<AtomicBool>>,
    /// 强制提炼计数器（防止无限循环提炼）
    pub refine_count: u32,
    /// 提炼轮禁工具开关（**代码级**，非提示词约束）：true 时 build_request
    /// 不携带任何 tools，react_loop 剔除模型产出的 tool_use 块——提炼轮
    /// 只允许输出摘要文本，杜绝「提炼轮继续调工具膨胀上下文」。
    /// 仅 in-loop refine 的提炼轮临时置 true，结束立即复位。
    pub tools_suppressed: bool,
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
    /// 一次性模型切换提示账本——比对基准：上次**实际使用过**的 (provider, model)
    /// 绑定（即上次 build_request 真正发请求时用的绑定）。这是「模型是否真的变了」
    /// 的唯一基准，不是「上一次切换动作的绑定」。初值取创建时的 config 绑定
    /// （= 当前激活模型），避免 agent 重建后对当前模型做无副作用的重配时产生空提示；
    /// 首次 build_request 后即被真实使用绑定覆盖。
    pub(crate) last_used_binding: Option<(String, String)>,
    /// 挂起中的模型切换提示。Some(binding) = 下次 build_request() 需在 messages
    /// 末尾一次性追加「当前模型已切换至…」并立即清零。切换动作只在此登记
    /// （queue_model_notice），绝不直接写入 session.messages；未发消息则一直挂起，
    /// 不因时间流逝丢失（无定时器/轮询）。
    pub(crate) pending_model_notice: Option<(String, String)>,
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
        // 一次性模型切换提示账本初值：创建时的 config 绑定 = 当前激活模型，作为
        // 「上次实际使用」的首个基准（见 last_used_binding 字段注释）。必须在 config
        // 移入结构体前克隆，否则 use-after-move。
        let last_used_binding = Some((config.provider.clone(), config.model.clone()));
        Self {
            llm,
            tools,
            policy,
            config,
            session: Session::new(),
            steps: Vec::new(),
            turn_meta: crate::agent::turn_meta::TurnMeta::default(),
            hooks,
            exec_tools: None,
            exec_llm: None,
            exec_emitter: None,
            task_runs: None,
            leader_ctx: prompt::LeaderContext::default(),
            client_factory: None,
            pause_flag: None,
            refine_count: 0,
            tools_suppressed: false,
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
            last_used_binding,
            pending_model_notice: None,
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
    /// memory.md 的中途变更一律下个 session 才生效，
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

    /// 彻底作废提示词前缀：清运行时缓存 + 丢弃 session 快照。
    ///
    /// 用于「这份提示词内容已确定不再适用于本会话」的场景（换 mode、换 Custom
    /// 卡片、换模型）。仅清缓存会保留 session 快照，下轮会被快照还原回来，
    /// 等于没失效——所以这两个动作必须成对。
    pub fn invalidate_prompt_cache(&mut self) {
        self.drop_prompt_prefix_cache();
        self.session.clear_prompt_snapshot();
    }

    /// 换项目：清运行时缓存；仅当会话快照归属的项目已变时才丢弃快照。
    ///
    /// 切到归属同项目的会话时快照仍有效，保留可命中 provider 前缀缓存。
    pub fn invalidate_prompt_cache_for_project(&mut self, project: Option<&str>) {
        self.drop_prompt_prefix_cache();
        let unchanged = self
            .session
            .prompt_snapshot()
            .map(|snapshot| snapshot.project_path.as_deref() == project)
            .unwrap_or(false);
        if !unchanged {
            self.session.clear_prompt_snapshot();
        }
    }

    /// 只丢弃内存里的运行时构建缓存（cached_base/l1/merged/tools）。
    ///
    /// **不动 session 快照**——切走会话时用这个，让目标会话的快照留待下次
    /// 切回时命中。要连快照一起作废请用 [`Self::invalidate_prompt_cache`]。
    pub fn drop_prompt_prefix_cache(&mut self) {
        self.cached_base_prompt = None;
        self.cached_l1_prompt = None;
        self.cached_merged_system_prompt = None;
        self.cached_tools_schemas = None;
    }

    fn prompt_tool_schemas(&self) -> Vec<crate::api::ToolDefinition> {
        match &self.custom_tool_whitelist {
            Some(wl) => self.tools.get_schemas_for(wl),
            None => self.tools.get_schemas(),
        }
    }

    fn snapshot_matches(
        &self,
        snapshot: &PromptSnapshot,
        mode: &str,
        tools: &[crate::api::ToolDefinition],
    ) -> bool {
        snapshot.provider == self.config.provider
            && snapshot.model == self.config.model
            && snapshot.mode == mode
            && snapshot.project_path == crate::utils::active_project().map(|(_, dir)| dir)
            && snapshot.custom_agent_id == crate::custom_agents::current_custom_agent_id()
            && snapshot.supports_vision == self.config.supports_vision
            && snapshot.vision_model == self.config.vision_model
            && snapshot.tools == tools
    }

    pub(crate) fn prepare_session_prompt_snapshot(&mut self, mode: &str) -> bool {
        let Some(snapshot) = self.session.prompt_snapshot().cloned() else {
            return false;
        };
        let tools = self.prompt_tool_schemas();
        if !self.snapshot_matches(&snapshot, mode, &tools) {
            // 逐字段对照：两种失败（快照在序列化跳板上丢失 / 判据不匹配）在日志里
            // 签名完全相同（都是 rebuilt + captured），不拆开就只能靠猜。
            tracing::info!(
                "[PromptSnapshot] rejected: provider({}→{}) model({}→{}) mode({}→{}) \
                 project({:?}→{:?}) custom({:?}→{:?}) vision({}→{}) vmodel({:?}→{:?}) \
                 tools_eq({})",
                snapshot.provider,
                self.config.provider,
                snapshot.model,
                self.config.model,
                snapshot.mode,
                mode,
                snapshot.project_path,
                crate::utils::active_project().map(|(_, dir)| dir),
                snapshot.custom_agent_id,
                crate::custom_agents::current_custom_agent_id(),
                snapshot.supports_vision,
                self.config.supports_vision,
                snapshot.vision_model,
                self.config.vision_model,
                snapshot.tools == tools,
            );
            self.drop_prompt_prefix_cache();
            self.session.clear_prompt_snapshot();
            return false;
        }
        self.cached_merged_system_prompt = Some(snapshot.system_prompt);
        self.cached_tools_schemas = Some(snapshot.tools);
        tracing::info!("[PromptSnapshot] restored session prompt prefix");
        true
    }

    pub(crate) fn capture_session_prompt_snapshot(&mut self, mode: &str) {
        if self.session.prompt_snapshot().is_some() {
            return;
        }
        let (Some(system_prompt), Some(tools)) = (
            self.cached_merged_system_prompt.clone(),
            self.cached_tools_schemas.clone(),
        ) else {
            return;
        };
        self.session.set_prompt_snapshot(PromptSnapshot {
            system_prompt,
            tools,
            provider: self.config.provider.clone(),
            model: self.config.model.clone(),
            mode: mode.to_string(),
            project_path: crate::utils::active_project().map(|(_, dir)| dir),
            custom_agent_id: crate::custom_agents::current_custom_agent_id(),
            supports_vision: self.config.supports_vision,
            vision_model: self.config.vision_model.clone(),
            tenets_section: prompt::tenets_section(),
        });
        tracing::info!("[PromptSnapshot] captured session prompt prefix");
    }

    pub(crate) fn inject_tenet_update(&mut self) -> bool {
        let current = prompt::tenets_section();
        let previous = match self.session.prompt_snapshot() {
            Some(snapshot) => snapshot.tenets_section.clone(),
            None => return false,
        };
        if previous == current {
            return false;
        }
        self.session
            .push_user_internal(prompt::format_tenet_update(&current));
        self.session.set_prompt_snapshot_tenets(current);
        tracing::info!(
            "[PromptSnapshot] injected tenet update ({} -> {} chars)",
            previous.len(),
            self.session
                .prompt_snapshot()
                .map(|s| s.tenets_section.len())
                .unwrap_or(0)
        );
        true
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

    /// 注入执行生命周期台账（进程级单例，见 `task_run::global_registry`）。
    /// 不注入时 dispatch 退化为「不记不显示」——台账是显示层唯一数据源，缺失即不可见，
    /// 因此这里不用 Option 兜假状态。
    pub fn set_task_runs(&mut self, reg: Arc<task_run::TaskRunRegistry>) {
        self.task_runs = Some(reg);
    }

    /// 登记一次性模型切换提示（挂起态）——切换命令的唯一落点。
    ///
    /// 语义（配套 `pending_model_notice` / `last_used_binding` 字段注释）：
    /// - 比对字段 = (provider, model) 双字段；基准 = 上次**实际使用过**的绑定
    ///   （`last_used_binding`），不是上一次切换动作的绑定。
    /// - 新绑定 == 基准 → 撤销挂起（切回曾用绑定不提示；含 agent 重建后对当前
    ///   激活模型做无副作用重配的场景）。
    /// - 基准为 None（无任何使用记录）或绑定已变 → 挂起，等下次 build_request
    ///   一次性消费。未发消息则一直挂起，不因时间流逝丢失（无定时器/轮询）。
    /// - 绝不写入 session.messages——注入由 build_request 以 transient 形态完成。
    pub fn queue_model_notice(&mut self, provider: &str, model: &str) {
        let binding = (provider.to_string(), model.to_string());
        if self.last_used_binding.as_ref() == Some(&binding) {
            self.pending_model_notice = None;
        } else {
            self.pending_model_notice = Some(binding);
        }
    }

    /// Build API request with merged system prompt (L0+L2+L1 as single message)
    pub(crate) fn build_request(
        &mut self,
        merged_system_prompt: &str,
    ) -> crate::api::MessageRequest {
        let mut messages = self.session.to_api_messages(self.config.supports_vision);
        // 一次性模型切换提示：仅进当次请求体（transient），绝不入 session.messages
        // （否则每轮重发，未达一次性目标）。消费即清零并刷新「上次实际使用」基准；
        // 同一 react 轮后续迭代再次调用不得追加。role=user 与旧 push_system 经
        // to_api_messages 序列化后的线上形态一致（session 内 System → user）。
        if let Some((provider, model)) = self.pending_model_notice.take() {
            messages.push(serde_json::json!({
                "role": "user",
                "content": format!(
                    "当前模型已切换至 {}（provider: {}）",
                    model, provider
                ),
            }));
            self.last_used_binding = Some((provider, model));
        }
        let mut request = crate::api::MessageRequest::new(&self.config.model, messages)
            .with_merged_system(merged_system_prompt);
        // 输入占用提示（官方口径）：传输层据此把 `input + max_tokens` 收进上下文
        // 窗口（Anthropic 超窗：老模型 400，4.5+ stop_reason=model_context_window_
        // exceeded）。读数来自 API usage 而非本地估算；0 = 新会话/提炼后清零，
        // 照样透传——clamp 只会因此取到最松的余量，下一次真实读数到达即自校正。
        request.input_tokens_hint = u32::try_from(self.session.context_occupancy()).ok();
        // 提炼轮禁工具（代码级，非提示词约束）：请求体完全不携带 tools 字段，
        // 模型无从发起工具调用——85% 硬止损的提炼轮必须只产出摘要，不能继续
        // 膨胀上下文（19e52dee 仅靠提示词「禁止调用任何工具」，模型不遵守时
        // 提炼轮本身就会失控）。其余请求路径不受影响（默认 false）。
        if !self.tools_suppressed {
            let tools = if let Some(tools) = &self.cached_tools_schemas {
                tools.clone()
            } else {
                let schemas = self.prompt_tool_schemas();
                tracing::info!(
                    "[PromptCache] Tool schemas cached ({} tools) for session",
                    schemas.len()
                );
                self.cached_tools_schemas = Some(schemas.clone());
                schemas
            };
            request = request.with_tools(tools);
        }
        request
    }

    /// Set model factory (for runtime model switching)
    pub fn set_client_factory(&mut self, factory: crate::llm::ClientFactory) {
        self.client_factory = Some(factory);
    }

    /// Set pause flag (shared with AppState, inherited by dispatch subtasks)
    pub fn set_pause_flag(&mut self, flag: Option<Arc<AtomicBool>>) {
        self.pause_flag = flag;
    }

    /// refine 成功后重建 L1（系统提示词动态块）缓存。
    ///
    /// 背景（2026-10-25 大王定权）：refine 现在**只保留最新摘要**（旧摘要
    /// 删除，不再叠加）。若系统提示词中的 L1 记忆块（distill 标题索引 +
    /// 项目记忆尾部）不随之重建，最新摘要与 L1 索引之间会形成断层——模型
    /// 按索引检索时看不到刚生成的提炼记录。重建只清 L1 与合并缓存（L0 base
    /// 保留），下次 build_request 时 L1 从记忆库重读、system 前缀随之更新。
    ///
    /// 代价：system 段 provider 缓存失效一次（有意取舍——收益是索引与上下文
    /// 一致，检索链不断层；此后前缀恢复稳定）。
    pub fn invalidate_l1_prompt_cache(&mut self) {
        self.cached_l1_prompt = None;
        self.cached_merged_system_prompt = None;
    }

    /// Load cross-phase context — the single L1 memory injection block:
    /// 1. Project memory journal tail (.nuphus/memory/{tag}.md, ≤2000 chars) — current state
    /// 2. Recent distill titles (global, guides memory_search self-check)
    /// 3. Tail lines: current session id + journal path + other-projects index
    ///    （跨话题切换时 read 对应项目文件恢复感知，不被 tag 隔离切断）
    pub fn load_cross_session_context(session_id: Option<&str>) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();

        // Part 1: 项目记忆日志尾部（唯一注入源；append 日志按条目取 tail，≤2000 字符）
        let md_path = crate::utils::active_memory_md_path();
        if md_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&md_path) {
                let tail = crate::utils::memory_journal_tail(&content, 2000);
                if !tail.trim().is_empty() {
                    parts.push(format!("## Leader Memory Update (跨会话传递)\n{tail}"));
                }
            }
        }

        // Part 2: 本项目最近提炼记录（标题 + 可直接用于 memory_session_context 的键）
        //   项目隔离（2026-10-26 定权）：只注入**当前项目**的 refine——提炼记录本身已按
        //   项目隔离、准确承载项目进度，注入其它项目的 refine 会污染新会话的上下文。
        //   跨项目过渡一律走 Part 1（leader_memory_update 写入的项目记忆日志）。
        //   键必须给 session_id（memory_session_context 的匹配键），不得给 entry id：
        //   entry id 形如 `refine-{session_id}-{ts}`，与 session_id 字段不同值，曾造成检索断线。
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
            false,
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
                    let key = if e.turn_id.is_empty() {
                        format!("session {}", e.session_id)
                    } else {
                        format!("session {} · turn {}", e.session_id, e.turn_id)
                    };
                    format!("- {} ({})", title, key)
                })
                .collect();
            if !distill_titles.is_empty() {
                parts.push(format!(
"## 历史对话提炼（本项目；精查用 memory_session_context 传下方 session id）\n{}",
                    distill_titles.join("\n")
                ));
            }
        }

        if parts.is_empty() && session_id.is_none() {
            return None;
        }

        // Part 3: 尾部导航——当前 session id（细节精查外键）+ 记忆文件路径 + 其它项目索引
        let mut nav = String::from("## 记忆导航");
        if let Some(sid) = session_id {
            nav.push_str(&format!(
                "\n- 当前 session id：{sid}（memory_session_context 精查本会话细节）"
            ));
        }
        nav.push_str(&format!(
            "\n- 更多记忆摘要：read {}（记忆日志全文，工作记忆）",
            md_path.display()
        ));
        let others = crate::utils::other_project_memory_paths();
        if !others.is_empty() {
            nav.push_str("\n- 其它项目记忆（跨话题切换时 read 对应文件恢复项目感知）：");
            for (tag, path) in others.iter().take(8) {
                nav.push_str(&format!("\n  - {tag}: {}", path.display()));
            }
        }
        parts.push(nav);

        Some(parts.join("\n\n"))
    }

    /// Switch current model using an exact provider + model binding.
    pub fn switch_model_for(
        &mut self,
        provider: &str,
        model_id: &str,
    ) -> Result<Arc<dyn ApiClient>> {
        let factory = self.client_factory.as_ref().ok_or_else(|| {
            crate::NuphusError::Agent(crate::AgentError::ModelSwitchFailed {
                error: "ClientFactory not set, cannot switch model".to_string(),
            })
        })?;
        let new_client = factory.create_client_for(provider, model_id).map_err(|e| {
            crate::NuphusError::Agent(crate::AgentError::ModelSwitchFailed {
                error: e.to_string(),
            })
        })?;
        self.llm = new_client.clone();
        self.config.model = model_id.to_string();
        self.config.provider = provider.to_string();
        tracing::info!("Switched to provider={}, model={}", provider, model_id);
        Ok(new_client)
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

        // HUD: 是否上屏由仲裁侧唯一判定（hud_arbiter::classify_tool 的同一张表）——
        // 本地副本（曾为 desktop_/browser_/system_shell 三条件）与仲裁集合漂移时，
        // 会出现「上了屏却没人发终态帧」→ 退化为兜底合成（HUD 粘住的温床）。
        let needs_hud = crate::hud_arbiter::is_hud_tool(&call.tool);
        // running / 终态同源取显示名：调用方 title 优先，缺省回落工具名。
        // 短命令在 HUD 上几乎只会被看到终态那一帧，若这里只有工具名，
        // 用户就永远看不到自己填的标题（实测反馈 2026-10-26）。
        let hud_label = crate::hud_arbiter::hud_title_from_params(Some(&call.params))
            .unwrap_or_else(|| call.tool.clone());
        if needs_hud {
            if let Some(ref emitter) = self.exec_emitter {
                emitter.emit(NuphusEvent::HudUpdate {
                    text: format!("{} — 执行中...", hud_label),
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
                                default_stage: pending.default_stage.clone(),
                            });
                            // H3：等待用户输入不是执行——HUD 从前序工具的 running 计时
                            // 转为等待语义（workflow_wait 无 auto-hide，常驻到用户响应，
                            // 等待结束后由下方 done 收尾，不会永停计时）。
                            emitter.emit(NuphusEvent::HudUpdate {
                                text: "等待用户输入".into(),
                                phase: "workflow_wait".into(),
                                step_kind: None,
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
                            // H3：等待结束，HUD 从 workflow_wait 收尾（done 无害：
                            // 15s auto-hide，随后由后续工具/轮次事件接管）
                            if let Some(ref emitter) = self.exec_emitter {
                                emitter.emit(NuphusEvent::HudUpdate {
                                    text: "输入已收到".into(),
                                    phase: "done".into(),
                                    step_kind: None,
                                });
                            }
                            // 非文本类型返回 JSON 字符串，解析后传给 LLM 以便直接使用 path/region 等字段
                            let parsed: serde_json::Value =
                                serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v));
                            return Ok(ToolResult::success(parsed.to_string()));
                        }
                        None => {
                            // H3：取消/超时同样收尾，HUD 不停在 workflow_wait
                            if let Some(ref emitter) = self.exec_emitter {
                                emitter.emit(NuphusEvent::HudUpdate {
                                    text: "输入已取消".into(),
                                    phase: "done".into(),
                                    step_kind: None,
                                });
                            }
                            return Ok(ToolResult::failure(
                                "用户未提供输入或操作已取消".to_string(),
                            ));
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
                    // 与 running 帧同一显示源（title 优先）——终态帧是短命令下
                    // HUD 唯一看得清的一帧，必须带标题
                    text: format!("{} — {}", hud_label, desc),
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
    ///
    /// 换会话必须清运行时缓存：否则上一会话已构建的 merged prompt 会被直接复用。
    /// 新会话自带的快照（若有）由下轮 `prepare_session_prompt_snapshot` 还原。
    pub fn set_session(&mut self, session: Session) {
        self.session = session;
        self.steps.clear();
        self.drop_prompt_prefix_cache();
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
    use async_trait::async_trait;

    // ═══ 一次性模型切换提示账本 ═══

    /// 测试用 ApiClient：账本逻辑不发起真实请求，只需满足 ReactAgent::new 签名。
    struct NoopApiClient;

    #[async_trait]
    impl ApiClient for NoopApiClient {
        async fn stream(
            &self,
            _request: crate::api::MessageRequest,
        ) -> crate::Result<Vec<crate::api::AssistantEvent>> {
            Ok(vec![])
        }

        fn model_name(&self) -> &str {
            "mock-model"
        }

        fn provider_kind(&self) -> crate::api::ProviderKind {
            crate::api::ProviderKind::MiniMax
        }

        fn provider_name(&self) -> &str {
            ""
        }
    }

    /// 构造指定绑定的最小 Agent（last_used 初值 = 该绑定）。
    fn agent_with_binding(provider: &str, model: &str) -> ReactAgent {
        let config = AgentConfig {
            provider: provider.to_string(),
            model: model.to_string(),
            ..AgentConfig::default()
        };
        ReactAgent::new(Arc::new(NoopApiClient), ToolRegistry::builtin(), config)
    }

    #[test]
    fn test_queue_notice_pends_when_binding_changed() {
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.queue_model_notice("provider-b", "model-2");
        assert_eq!(
            agent.pending_model_notice,
            Some(("provider-b".to_string(), "model-2".to_string()))
        );
    }

    #[test]
    fn test_queue_notice_provider_only_change_pends() {
        // 双字段比对：同模型跨 provider 必须挂起
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.queue_model_notice("provider-b", "model-1");
        assert_eq!(
            agent.pending_model_notice,
            Some(("provider-b".to_string(), "model-1".to_string()))
        );
    }

    #[test]
    fn test_queue_notice_same_binding_no_pend() {
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.queue_model_notice("provider-a", "model-1");
        assert_eq!(agent.pending_model_notice, None);
    }

    #[test]
    fn test_queue_notice_revokes_on_switch_back() {
        // last_used=(A,M1)：依次 queue (B,M2)、(A,M1) 且不发消息 → 最终挂起 None
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.queue_model_notice("provider-b", "model-2");
        assert!(agent.pending_model_notice.is_some());
        agent.queue_model_notice("provider-a", "model-1");
        assert_eq!(agent.pending_model_notice, None);
    }

    #[test]
    fn test_queue_notice_none_last_used_pends() {
        // 无任何使用记录（last_used=None）时首次 queue → 产生挂起
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.last_used_binding = None;
        agent.queue_model_notice("provider-a", "model-1");
        assert_eq!(
            agent.pending_model_notice,
            Some(("provider-a".to_string(), "model-1".to_string()))
        );
    }

    #[test]
    fn test_build_request_consumes_notice_once_and_keeps_session_intact() {
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.session_mut().push_user("问题".to_string());
        agent.queue_model_notice("provider-b", "model-2");
        let session_len = agent.session().len();

        // 首次：messages 末一次性追加提示（transient）
        let req1 = agent.build_request("sys");
        let last = req1.messages.last().expect("messages 非空");
        assert_eq!(last["role"], "user");
        assert_eq!(
            last["content"],
            "当前模型已切换至 model-2（provider: provider-b）"
        );
        // 消费即清零 + 刷新「上次实际使用」基准
        assert_eq!(agent.pending_model_notice, None);
        assert_eq!(
            agent.last_used_binding,
            Some(("provider-b".to_string(), "model-2".to_string()))
        );
        // session.messages 长度不变（提示绝不落库）
        assert_eq!(agent.session().len(), session_len);

        // 再次调用不得再追加
        let req2 = agent.build_request("sys");
        assert_eq!(req2.messages.len(), req1.messages.len() - 1);
        assert!(
            !req2.messages.iter().any(|m| m["content"]
                .as_str()
                .is_some_and(|c| c.contains("当前模型已切换至"))),
            "第二次 build_request 不得再含切换提示"
        );
        assert_eq!(agent.session().len(), session_len);
    }

    #[test]
    fn test_build_request_without_pending_has_no_notice() {
        let mut agent = agent_with_binding("provider-a", "model-1");
        let req = agent.build_request("sys");
        assert!(
            !req.messages.iter().any(|m| m["content"]
                .as_str()
                .is_some_and(|c| c.contains("当前模型已切换至"))),
            "无挂起时不得注入切换提示"
        );
    }

    /// A2：输入占用提示随每次 build_request 透传——传输层窗口余量 clamp 的
    /// 唯一依据。取 `Session::context_occupancy()`（官方 usage 口径：
    /// 末轮 input + output）；新会话无读数 → 0（clamp 只会取到最松余量）。
    #[test]
    fn test_build_request_fills_input_tokens_hint_from_occupancy() {
        let mut agent = agent_with_binding("provider-a", "model-1");
        // 新会话：尚无官方 usage 读数 → 0
        assert_eq!(agent.build_request("sys").input_tokens_hint, Some(0));
        // 官方读数到位后：input + output 原样透传（不做本地估算）
        agent.session_mut().update_api_input_tokens(120_000);
        agent.session_mut().update_api_output_tokens(3_000);
        assert_eq!(
            agent.build_request("sys").input_tokens_hint,
            Some(123_000),
            "hint 必须等于 context_occupancy（input + output）"
        );
    }

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

    #[test]
    fn test_extract_tool_call_close_tag_with_space() {
        // </tool_call >（'>' 前空白）应被空格容错识别并连同整块一起剥除
        let (clean, calls) = extract_tool_calls_from_text_with_tags(
            "前<tool_call>{\"name\":\"Read\",\"arguments\":{\"path\":\"/tmp/a\"}}</tool_call >后",
            &[],
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "Read");
        assert_eq!(calls[0].arguments["path"], "/tmp/a");
        assert_eq!(clean, "前后");
    }

    #[test]
    fn test_extract_tool_call_no_close_keeps_text() {
        // 无闭合标签：既有 break 语义保留，交由终态 strip 兜底剥除
        let (clean, calls) =
            extract_tool_calls_from_text_with_tags("text <tool_call>{\"name\":\"Read\"}", &[]);
        assert!(calls.is_empty());
        assert!(clean.contains("<tool_call>"));
    }

    #[test]
    fn test_prompt_snapshot_restored_after_session_switch() {
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.session_mut().push_user("hi".to_string());
        agent.cached_merged_system_prompt = Some("SNAP".to_string());
        agent.build_request("SNAP");
        agent.capture_session_prompt_snapshot("leader");
        assert!(agent.session().prompt_snapshot().is_some());

        let session = agent.session().clone();
        agent.set_session(session);
        assert!(agent.cached_merged_system_prompt.is_none());
        assert!(agent.cached_tools_schemas.is_none());
        assert!(agent.prepare_session_prompt_snapshot("leader"));
        assert_eq!(agent.cached_merged_system_prompt.as_deref(), Some("SNAP"));
        assert!(agent.cached_tools_schemas.is_some());
    }

    #[test]
    fn test_prompt_snapshot_rejected_on_model_or_mode_change() {
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.session_mut().push_user("hi".to_string());
        agent.cached_merged_system_prompt = Some("SNAP".to_string());
        agent.build_request("SNAP");
        agent.capture_session_prompt_snapshot("leader");

        assert!(!agent.prepare_session_prompt_snapshot("custom"));
        agent.config.model = "model-2".to_string();
        assert!(!agent.prepare_session_prompt_snapshot("leader"));
        assert!(agent.cached_merged_system_prompt.is_none());
    }

    #[test]
    fn test_invalidate_prompt_cache_drops_snapshot() {
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.session_mut().push_user("hi".to_string());
        agent.cached_merged_system_prompt = Some("SNAP".to_string());
        agent.build_request("SNAP");
        agent.capture_session_prompt_snapshot("leader");
        agent.invalidate_prompt_cache();
        assert!(agent.session().prompt_snapshot().is_none());
        assert!(!agent.prepare_session_prompt_snapshot("leader"));
    }

    #[test]
    fn test_prompt_snapshot_rejected_on_tools_change() {
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.session_mut().push_user("hi".to_string());
        agent.cached_merged_system_prompt = Some("SNAP".to_string());
        agent.build_request("SNAP");
        agent.capture_session_prompt_snapshot("leader");
        // 工具集变化（换 Custom 卡片白名单 / enhanced 开关）→ 前缀不再成立
        agent
            .session_mut()
            .prompt_snapshot
            .as_mut()
            .unwrap()
            .tools
            .clear();
        assert!(!agent.prepare_session_prompt_snapshot("leader"));
        assert!(agent.session().prompt_snapshot().is_none());
    }

    #[test]
    fn test_project_change_keeps_snapshot_for_same_project() {
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.session_mut().push_user("hi".to_string());
        agent.cached_merged_system_prompt = Some("SNAP".to_string());
        agent.build_request("SNAP");
        agent.capture_session_prompt_snapshot("leader");
        let current = crate::utils::active_project().map(|(_, dir)| dir);
        agent.invalidate_prompt_cache_for_project(current.as_deref());
        assert!(agent.session().prompt_snapshot().is_some());
        agent
            .session_mut()
            .prompt_snapshot
            .as_mut()
            .unwrap()
            .project_path = Some("__other_project__".to_string());
        assert!(!agent.prepare_session_prompt_snapshot("leader"));
        assert!(agent.session().prompt_snapshot().is_none());
    }

    #[test]
    fn test_capture_keeps_existing_snapshot_marker() {
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.session_mut().push_user("hi".to_string());
        agent.cached_merged_system_prompt = Some("SNAP".to_string());
        agent.build_request("SNAP");
        agent.capture_session_prompt_snapshot("leader");
        agent
            .session_mut()
            .set_prompt_snapshot_tenets("stale-tenets".to_string());
        agent.capture_session_prompt_snapshot("leader");
        assert_eq!(
            agent.session().prompt_snapshot().unwrap().tenets_section,
            "stale-tenets"
        );
    }

    #[test]
    fn test_tenet_update_injected_once() {
        let mut agent = agent_with_binding("provider-a", "model-1");
        agent.session_mut().push_user("hi".to_string());
        agent
            .session_mut()
            .set_prompt_snapshot(crate::session::PromptSnapshot {
                system_prompt: "SNAP".to_string(),
                tools: Vec::new(),
                provider: "provider-a".to_string(),
                model: "model-1".to_string(),
                mode: "leader".to_string(),
                project_path: None,
                custom_agent_id: None,
                supports_vision: false,
                vision_model: None,
                tenets_section: "stale-tenets".to_string(),
            });
        let before = agent.session().len();
        assert!(agent.inject_tenet_update());
        assert_eq!(agent.session().len(), before + 1);
        assert!(!agent.inject_tenet_update());
        assert_eq!(
            agent.session().prompt_snapshot().unwrap().tenets_section,
            prompt::tenets_section()
        );
    }
}
