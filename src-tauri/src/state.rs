use nuphus::permissions::ToolPermissions;
use nuphus::runtime::Runtime;
use nuphus::runtime::WorkflowAgent;
use nuphus_index::IndexEngine;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64};
use std::sync::{Arc, Mutex};

// ── App State ──

pub struct AppState {
    pub tools: nuphus::ToolRegistry,
    /// Group: LLM config, permissions, agent, context window, threshold (5→1 Mutex)
    pub runtime: Mutex<RuntimeContext>,
    /// Group: session identity, message dedup, backups (4→1 Mutex)
    pub session: Mutex<SessionState>,
    /// Group: security, retry, dedup queue, knowledge engine (4→1 Mutex)
    pub execution: Mutex<ExecutionState>,
    pub llm_config_path: std::path::PathBuf,
    pub tool_permissions_path: std::path::PathBuf,
    /// Shared tool permissions — cloned into Runtime for real-time policy updates
    pub tool_permissions_ref: Arc<std::sync::Mutex<ToolPermissions>>,
    pub cancel_flag: Arc<AtomicBool>,
    pub pause_flag: Arc<AtomicBool>,
    /// 后端权威当前运行模式（"leader" | "workflow" | "custom"），chat_history 按此选择 agent 会话。
    /// 由 set_mode_impl（显式切换）与 submit_user_message（发送确认）维护；默认 "leader"。
    pub current_mode: Arc<std::sync::RwLock<String>>,
    pub busy: AtomicBool,
    pub last_process_time: AtomicI64,
    pub last_completion_time: AtomicI64,
    pub event_seq: Arc<AtomicU64>,
    /// When true, StateChecker skips LLM call (refine in progress, avoid race)
    pub refine_active: Arc<AtomicBool>,
    /// Workflow engine (RwLock: wf_stop can acquire read lock to cancel while workflow_run tool holds read lock)
    pub workflow_engine: Arc<tokio::sync::RwLock<nuphus::workflow::WorkflowEngine>>,
    /// 全进程唯一的会话级信号状态（pause/security/workflow）——core 库无全局 static，
    /// 由本实例持有并注入 ToolRegistry / WorkflowEngine / 各命令处理函数
    pub signals: nuphus::state::SharedSignals,
    /// Speech-to-text subsystem (lazy: recognizer loads on first stt_start)
    pub speech: crate::speech::SpeechState,
    /// Mobile server WS broadcaster — Some(tx) when mobile_server running, None when stopped.
    /// CompoundEmitter reads this per message round; None → pure Tauri push (desktop-only behavior).
    pub mobile_ws_tx: Arc<std::sync::Mutex<Option<tokio::sync::broadcast::Sender<String>>>>,
    /// Mobile server shutdown handle — Some while server running (drop/send triggers graceful stop)
    pub mobile_server_shutdown: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    /// Mobile access token — shared with the running server so token regeneration
    /// takes effect immediately without restart (persisted in mobile_server.json)
    pub mobile_token: Arc<std::sync::RwLock<String>>,
    /// 最近一次生效的身份关系配置（桌面端 soul 配置随消息传入，手机端无配置通道，
    /// localStorage 隔离拿不到——发消息不传 relation 时用此兜底，保证手机端触发的
    /// 执行 agent 身份与桌面端一致；同时供 GET /identity 下发手机端显示名）
    pub relation_cache: Arc<std::sync::RwLock<Option<nuphus::agent::goal_types::RelationConfig>>>,
    /// 当前生效主题快照（/plugins-shared/theme.css 渲染源；useTheme 变化时由
    /// theme_snapshot_save 更新，插件 iframe 经 theme.css 获得与主窗口一致的主题）
    pub theme_snapshot: Mutex<ThemeSnapshot>,
    /// 插件 agent.chat 在途集合（每插件同时只允许一个在途 chat；
    /// guard 模式确保 panic/取消也移除，见 plugin_apps::PluginChatGuard）
    pub plugin_chat_inflight: Mutex<std::collections::HashSet<String>>,
    /// 插件 workflow.run 在途集合（每插件同时只允许一个在途工作流执行；
    /// guard 模式确保 panic/取消也移除，见 plugin_apps::PluginWorkflowGuard）
    pub plugin_workflow_inflight: Mutex<std::collections::HashSet<String>>,
    /// Session Shelf —— 浅层会话展示台（内存 LRU ≤10 + 磁盘镜像），见 process/shelf.rs
    pub shelf: Mutex<crate::commands::process::shelf::ShelfState>,
}

/// 主题快照：base 为主题标识（dark/light），overrides 为 documentElement 内联覆盖
/// 的 CSS 自定义属性（key 以 `--` 开头）。插件 iframe 跨 origin 无法读取主窗口
/// DOM，由 mobile_server 的 /plugins-shared/theme.css 据此渲染为 CSS 文件下发。
#[derive(Debug, Clone)]
pub struct ThemeSnapshot {
    pub base: String,
    pub overrides: std::collections::HashMap<String, String>,
}

impl Default for ThemeSnapshot {
    fn default() -> Self {
        Self {
            base: "dark".to_string(),
            overrides: std::collections::HashMap::new(),
        }
    }
}

// ── Group structs ──

pub struct RuntimeContext {
    pub llm_config: Option<LlamaConfig>,
    pub tool_permissions: ToolPermissions,
    pub leader_agent: Option<Runtime>,
    pub workflow_agent: Option<WorkflowAgent>,
    pub model_context_window: usize,
    pub refine_threshold: f64,
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self {
            llm_config: None,
            tool_permissions: ToolPermissions::default(),
            leader_agent: None,
            workflow_agent: None,
            model_context_window: 0,
            refine_threshold: 0.5,
        }
    }
}

#[derive(Default)]
pub struct SessionState {
    pub last_message: String,
    pub last_send_id: Option<String>,
    /// 与 last_message 同步记录非 busy 受理的图片（data URL 列表）——执行中刷新
    /// 走 session_backup 回退路径时，append_last_turn_user 用它补回当前轮带图消息。
    pub last_message_images: Vec<String>,
    pub session_backup: Option<String>,
}

#[derive(Default)]
pub struct ExecutionState {
    pub pending_security: std::collections::HashMap<String, SecurityPending>,
    pub pending_retry: Option<(String, LlamaConfig, String)>,
    pub completed_send_ids: std::collections::VecDeque<String>,
    pub knowledge_engine: Option<IndexEngine>,
}

// ── Timestamp helpers ──

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

impl Default for AppState {
    fn default() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("nuphus");

        // 启动时加载已持久化的身份关系配置（relation.json）：
        // 老用户升级后桌面端未发消息时，手机端首条指令经 relation_cache 也能拿到用户定义的称呼。
        let relation_cache = {
            let path = config_dir.join("relation.json");
            std::fs::read_to_string(&path).ok().and_then(|s| {
                serde_json::from_str::<nuphus::agent::goal_types::RelationConfig>(&s).ok()
            })
        };

        // 锚定模型注册表到规范 providers.toml，防止 cwd 下无关 config.toml
        // 劫持 load_registry（曾导致 k3 上下文 1M 被误读为回退值 128K）
        nuphus::config::set_config_override(config_dir.join("providers.toml"));

        // Load persisted permissions or use defaults (mirrors AppState::new())
        let tool_permissions_path = config_dir.join("tool_permissions.json");
        let tool_permissions = std::fs::read_to_string(&tool_permissions_path)
            .ok()
            .and_then(|data| serde_json::from_str::<ToolPermissions>(&data).ok())
            .unwrap_or_default();
        let tool_permissions_ref = Arc::new(std::sync::Mutex::new(tool_permissions));

        // 全进程唯一信号状态实例：注入 ToolRegistry 与 WorkflowEngine，
        // core 库内所有 pause/security/workflow 信号读写均经此句柄
        let signals = nuphus::state::new_shared_signals();
        let mut tools = nuphus::ToolRegistry::builtin_with_desktop();
        tools.set_signals(signals.clone());
        let mut workflow_engine = nuphus::workflow::WorkflowEngine::new();
        workflow_engine.set_signals(signals.clone());

        Self {
            tools,
            runtime: Mutex::new(RuntimeContext {
                tool_permissions,
                ..Default::default()
            }),
            session: Mutex::new(SessionState::default()),
            execution: Mutex::new(ExecutionState::default()),
            llm_config_path: config_dir.join("providers.toml"),
            tool_permissions_path,
            tool_permissions_ref,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            pause_flag: Arc::new(AtomicBool::new(false)),
            current_mode: Arc::new(std::sync::RwLock::new("leader".to_string())),
            busy: AtomicBool::new(false),
            last_process_time: AtomicI64::new(0),
            last_completion_time: AtomicI64::new(0),
            event_seq: Arc::new(AtomicU64::new(0)),
            refine_active: Arc::new(AtomicBool::new(false)),
            workflow_engine: Arc::new(tokio::sync::RwLock::new(workflow_engine)),
            signals,
            speech: crate::speech::SpeechState::default(),
            mobile_ws_tx: Arc::new(std::sync::Mutex::new(None)),
            mobile_server_shutdown: std::sync::Mutex::new(None),
            mobile_token: Arc::new(std::sync::RwLock::new(
                crate::mobile_server::load_config().token,
            )),
            relation_cache: Arc::new(std::sync::RwLock::new(relation_cache)),
            theme_snapshot: Mutex::new(ThemeSnapshot::default()),
            plugin_chat_inflight: Mutex::new(std::collections::HashSet::new()),
            plugin_workflow_inflight: Mutex::new(std::collections::HashSet::new()),
            shelf: Mutex::new(crate::commands::process::shelf::ShelfState::default()),
        }
    }
}

impl AppState {
    pub fn record_process_start(&self) {
        self.last_process_time
            .store(now_millis(), std::sync::atomic::Ordering::SeqCst);
    }

    pub fn record_completion(&self) {
        self.last_completion_time
            .store(now_millis(), std::sync::atomic::Ordering::SeqCst);
    }

    pub fn elapsed_since_process_start(&self) -> u64 {
        let stored = self
            .last_process_time
            .load(std::sync::atomic::Ordering::SeqCst);
        if stored == 0 {
            return u64::MAX;
        }
        ((now_millis() - stored).max(0) / 1000) as u64
    }

    pub fn elapsed_since_completion(&self) -> u64 {
        let stored = self
            .last_completion_time
            .load(std::sync::atomic::Ordering::SeqCst);
        if stored == 0 {
            return u64::MAX;
        }
        ((now_millis() - stored).max(0) / 1000) as u64
    }
}

#[derive(Debug, Clone)]
pub struct SecurityPending {
    pub approved: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlamaConfig {
    pub provider: String,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
    #[serde(default)]
    pub parameters: Option<GenerationParameters>,
    /// Reasoning depth (config.toml `[[providers]] reasoning_effort`), threaded
    /// into the transport at client build time.
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryTraceItem {
    /// "thinking" | "text" | "tool"
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// "running" | "ok" | "fail"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio: Vec<String>,
    /// 消息创建时间（Unix 毫秒）。旧数据/降级恢复路径可能缺失（None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
    /// 执行过程（思考/流式文本/工具调用，按实际顺序）——Session 完整存储，
    /// 历史拉取时下发，手机端显示完成状态（非「不显示不误导」的妥协）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace_items: Vec<HistoryTraceItem>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProcessInputResponse {
    pub success: bool,
    pub message: String,
    /// 执行中发送被接受为追加指令（不开启新执行；双端统一，不拒绝不丢弃）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appended: Option<bool>,
    /// 图片降级警告：主模型与 vision 模型都不支持视觉时返回，前端弹窗提示。
    /// 图片仍降级发送（保存临时文件路径占位），不阻塞消息。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_warning: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    /// 画布工具面板展示分组键（workflow_tool_group 唯一来源）；get_tools 不填充（None）。
    /// 反序列化缺省容错旧缓存；None 时不序列化，get_tools 响应体不变。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DesktopStatus {
    pub connected: bool,
    pub tools_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HookScriptInfo {
    pub path: String,
    pub exists: bool,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HooksConfigStatus {
    pub pre_tool_call: Option<HookScriptInfo>,
    pub post_tool_call: Option<HookScriptInfo>,
    pub on_session_start: Option<HookScriptInfo>,
    pub on_session_end: Option<HookScriptInfo>,
    pub config_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryStats {
    pub total_entries: usize,
    pub patterns: usize,
    pub skills: usize,
    pub principles: usize,
    pub templates: usize,
    pub seeds: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TimelineIndexStats {
    pub total_entries: usize,
    pub total_sessions: usize,
    pub successful: usize,
    pub failed: usize,
    pub by_intent: std::collections::HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDetailEntry {
    pub id: String,
    pub kind: String,
    pub user_message: String,
    pub assistant_message: String,
    pub steps_summary: Vec<String>,
    pub goal_type: Option<String>,
    pub timestamp: String,
    pub success: bool,
}
