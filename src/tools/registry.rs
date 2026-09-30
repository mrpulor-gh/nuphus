//! Tool registry — enhanced tool definitions and permission binding

use crate::automation_gate::{AutomationGate, HoldKind, ResourceClass, OWNER_MANUAL_TOOL};
use crate::browser::BrowserClient;
use crate::desktop::DesktopClient;
use crate::permissions::{PermissionOutcome, PermissionPolicy, ToolCategory};
use crate::ToolResult;
use serde::Deserialize;
use std::collections::HashMap;
use std::string::String;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use super::semantic_desktop::SemanticDesktopBackend;

/// Tool execution context — bundle of injected handles passed to every executor.
///
/// PR-2 (AppState 合并): 携带 `SharedSignals`（pause/security/workflow 信号句柄），
/// 供 tenet_add / request_user_input 等需要写信号状态的工具使用。
/// 设计见 docs/internal/2026-08-06-appstate-merge-design.md §2.3 方案 A。
pub type ScheduleToolCallback =
    Arc<dyn Fn(&serde_json::Value) -> std::result::Result<crate::ToolResult, String> + Send + Sync>;

#[derive(Clone, Default)]
pub struct ToolCtx {
    /// 共享信号状态句柄（由 ToolRegistry 在 execute() 注入）
    pub signals: crate::state::SharedSignals,
    /// Desktop host bridge for schedule_cron. None in headless/library-only contexts.
    pub schedule_tool: Option<ScheduleToolCallback>,
}

impl std::fmt::Debug for ToolCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolCtx")
            .field("signals", &self.signals)
            .field("schedule_tool", &self.schedule_tool.is_some())
            .finish()
    }
}

/// Tool definition
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub category: ToolCategory,
    pub executor: fn(
        params: &serde_json::Value,
        ctx: &ToolCtx,
    ) -> std::result::Result<crate::ToolResult, String>,
    /// List of dependent tools: these tools must have been called this round or historically before this one
    pub depends_on: Vec<String>,
}

/// Tool registry
pub struct ToolRegistry {
    pub(super) tools: HashMap<String, ToolDef>,
    pub(super) desktop_client: Arc<RwLock<Option<DesktopClient>>>,
    pub(super) desktop_targets:
        Arc<RwLock<Option<Arc<crate::desktop::targets::DesktopTargetService>>>>,
    /// Accessibility/UIA-first desktop backend. Unlike `DesktopClient`, this
    /// backend never exposes coordinates or native handles to the model.
    pub(super) semantic_desktop: Option<SemanticDesktopBackend>,
    /// Whether WorkflowAgent may delegate one bounded semantic choice to the
    /// optional enhanced decision model. Ordinary semantic tools stay enabled.
    pub(super) enhanced_mode: bool,
    /// Browser client (Rust native CDP)
    pub(super) browser_client: Arc<tokio::sync::Mutex<Option<BrowserClient>>>,
    /// Rendered prompt cache (cleared on register, lazy-built on render)
    pub(super) prompt_cache: Arc<RwLock<Option<String>>>,
    /// API name ↔ internal name mapping (currently all underscore format, kept for compatibility)
    canonical_map: HashMap<String, String>,
    /// Shared signal state (pause/security/workflow) — injected by the desktop shell,
    /// passed to tool executors via ToolCtx at the execute() choke point.
    /// Clone shares the same Arc (same pattern as desktop_client).
    signals: crate::state::SharedSignals,
    schedule_tool: Arc<RwLock<Option<ScheduleToolCallback>>>,
    /// 自动化工具（`desktop_*` / `browser_*`）开关。
    ///
    /// false = 既不暴露 schema，也拒绝执行。ExecAgent 走此隔离：
    /// 约束是「Exec 执行自动化操作属黑盒」，故必须双端阻断——仅在 schema 层
    /// 隐藏不够，模型仍可能凭上下文或历史消息臆造工具名直接调用。
    pub(super) automation_tools_enabled: bool,
    /// Process-wide resource gate injected by the desktop shell. Core-only
    /// callers may omit it; production registries all share AppState's gate.
    automation_gate: Option<Arc<AutomationGate>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self {
            tools: HashMap::new(),
            desktop_client: Arc::new(RwLock::new(None)),
            desktop_targets: Arc::new(RwLock::new(None)),
            semantic_desktop: None,
            enhanced_mode: false,
            browser_client: crate::browser::shared_client(),
            prompt_cache: Arc::new(RwLock::new(None)),
            canonical_map: HashMap::new(),
            signals: crate::state::new_shared_signals(),
            schedule_tool: Arc::new(RwLock::new(None)),
            automation_tools_enabled: true,
            automation_gate: None,
        }
    }
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        Self {
            tools: self.tools.clone(),
            desktop_client: self.desktop_client.clone(),
            desktop_targets: self.desktop_targets.clone(),
            semantic_desktop: self.semantic_desktop.clone(),
            enhanced_mode: self.enhanced_mode,
            browser_client: self.browser_client.clone(),
            prompt_cache: self.prompt_cache.clone(),
            canonical_map: self.canonical_map.clone(),
            signals: self.signals.clone(),
            schedule_tool: self.schedule_tool.clone(),
            automation_tools_enabled: self.automation_tools_enabled,
            automation_gate: self.automation_gate.clone(),
        }
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.tools)
            .field(
                "desktop_client",
                &self
                    .desktop_client
                    .read()
                    .map(|g| g.is_some())
                    .unwrap_or(false),
            )
            .field("browser_client", &"async-mutex")
            .field(
                "prompt_cached",
                &self
                    .prompt_cache
                    .read()
                    .map(|g| g.is_some())
                    .unwrap_or(false),
            )
            .finish()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 共享信号状态句柄（pause/security/workflow）
    pub fn signals(&self) -> &crate::state::SharedSignals {
        &self.signals
    }

    /// 注入共享信号句柄（desktop shell 启动时对各构造路径生成的 registry 调用，
    /// 保证全进程指向同一 SignalState 实例）
    pub fn set_signals(&mut self, signals: crate::state::SharedSignals) {
        self.signals = signals;
    }

    pub fn set_automation_gate(&mut self, gate: Arc<AutomationGate>) {
        self.automation_gate = Some(gate);
    }

    /// Reuse the same process-wide automation gate when deriving a role-
    /// specific registry. This mirrors `signals()` and avoids constructing a
    /// second gate for Leader/Workflow/plugin runtimes.
    pub fn automation_gate(&self) -> Option<Arc<AutomationGate>> {
        self.automation_gate.clone()
    }

    fn acquire_semantic_desktop_lease(
        &self,
    ) -> Result<Option<crate::automation_gate::AutomationLease>, String> {
        let Some(gate) = &self.automation_gate else {
            return Ok(None);
        };
        let current_owner = crate::automation_gate::current_execution_owner();
        let (owner, kind) = match current_owner {
            Some(owner) => (owner, HoldKind::ExecutionBody),
            None => (
                format!("{OWNER_MANUAL_TOOL}:{}", uuid::Uuid::new_v4()),
                HoldKind::ManualTool,
            ),
        };
        gate.try_acquire(ResourceClass::Desktop, kind, owner)
            .map(Some)
            .map_err(|busy| busy.to_string())
    }

    pub fn set_schedule_tool_callback(&self, callback: ScheduleToolCallback) {
        if let Ok(mut slot) = self.schedule_tool.write() {
            *slot = Some(callback);
        }
    }

    /// Register tool (automatically clears prompt cache)
    pub fn register(&mut self, def: ToolDef) {
        tracing::debug!("Registering tool: {}", def.name);
        let canonical = def.name.replace("::", "_");
        self.canonical_map.insert(canonical, def.name.clone());
        self.tools.insert(def.name.clone(), def);
        if let Ok(mut guard) = self.prompt_cache.write() {
            *guard = None;
        }
    }

    /// Get tool definition (supports original and canonical names, e.g. system_shell)
    pub fn get(&self, name: &str) -> Option<&ToolDef> {
        self.tools.get(name).or_else(|| {
            self.canonical_map
                .get(name)
                .and_then(|original| self.tools.get(original))
        })
    }

    /// Check if tool exists (flat name + internal name dual resolution + desktop tools + browser tools)
    pub fn has_tool(&self, name: &str) -> bool {
        self.get(name).is_some()
            || (self.automation_tools_enabled
                && (Self::is_desktop_tool(name) || Self::is_browser_tool(name)))
    }

    /// Check if tool dependencies are satisfied
    ///
    /// `called_tools` is the set of already-called tool names (including original and flat names).
    /// Returns missing dependencies; empty means all satisfied.
    pub fn check_dependencies(
        &self,
        tool_name: &str,
        called_tools: &std::collections::HashSet<String>,
    ) -> Vec<String> {
        let Some(def) = self.get(tool_name) else {
            return Vec::new();
        };
        if def.depends_on.is_empty() {
            return Vec::new();
        }
        def.depends_on
            .iter()
            .filter(|dep| !called_tools.contains(*dep))
            .cloned()
            .collect()
    }

    /// Load tool dependencies from TOML file, overriding registered tools
    ///
    /// File format: each section name as group, inner key=tool name (suffix), value=dependency list.
    /// Full tool name = "group_key" (e.g. [file] write = ["file_mkdir"] → file_write depends on file_mkdir).
    /// Silently skip if file doesn't exist, only log warning.
    fn load_depends_from_file(&mut self, path: &str) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Tool deps file '{}' not loaded: {}", path, e);
                return;
            }
        };

        #[derive(Deserialize)]
        struct DepsFile {
            #[serde(flatten)]
            groups:
                std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<String>>>,
        }

        let deps: DepsFile = match toml::from_str(&content) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Failed to parse tool deps file '{}': {}", path, e);
                return;
            }
        };

        let mut applied = 0usize;
        for (group, tools) in &deps.groups {
            for (tool_suffix, depends) in tools {
                let full_name = format!("{}_{}", group, tool_suffix);
                if let Some(def) = self.tools.get_mut(&full_name) {
                    def.depends_on = depends.clone();
                    applied += 1;
                } else {
                    tracing::debug!("Tool deps: tool '{}' not registered, skipping", full_name);
                }
            }
        }
        tracing::info!(
            "Loaded tool dependencies from '{}': {} applied",
            path,
            applied
        );
    }

    /// Execute tool (async)
    pub async fn execute(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> std::result::Result<ToolResult, String> {
        // 自动化工具开关（ExecAgent 关闭）——执行侧终点。
        // schema 层已不暴露，此处兜住「凭历史上下文臆造工具名直接调用」的路径。
        if !self.automation_tools_enabled
            && (Self::is_browser_tool(tool_name) || Self::is_desktop_tool(tool_name))
        {
            return Ok(ToolResult::failure(format!(
                "Tool '{}' is unavailable for this agent role (automation tools disabled).",
                tool_name
            )));
        }

        // 检查是否是浏览器工具
        if tool_name.starts_with("browser_") {
            // 浏览器工具需要异步执行，这里返回提示
            return Ok(ToolResult::failure(
                "Browser tools require async execution. Use execute_browser_tool() instead."
                    .to_string(),
            ));
        }

        // 检查是否是桌面工具，使用 DesktopClient 执行
        // 先 clone client 释放 MutexGuard，避免 guard 跨越 await 点
        if tool_name.starts_with("desktop_") {
            if Self::is_semantic_desktop_tool(tool_name) {
                let _lease = self.acquire_semantic_desktop_lease()?;
                return self.execute_semantic_desktop_tool(tool_name, params).await;
            }
            // Capture provenance is registry-local. Keep both producers and
            // consumers on this backend; routing only one half through MCP
            // would lose the transform/cache or make IDs refer to another app.
            if matches!(
                tool_name,
                "desktop_screenshot" | "desktop_window_screenshot" | "desktop_perceive"
            ) || (tool_name == "desktop_mouse"
                && (params.get("capture_id").is_some() || params.get("element_id").is_some()))
                || (tool_name == "desktop_mouse_drag"
                    && [
                        "start_capture_id",
                        "start_element_id",
                        "end_capture_id",
                        "end_element_id",
                    ]
                    .iter()
                    .any(|key| params.get(*key).is_some()))
                || (tool_name == "desktop_input" && params.get("target_locator").is_some())
            {
                let _lease = self.acquire_semantic_desktop_lease()?;
                let lock = crate::utils::automation_lock::AutomationLock::new();
                let _guard = lock.acquire(tool_name).map_err(|e| e.to_string())?;
                let client = self.desktop_client().ok_or("本地桌面服务尚未连接")?;
                return self.execute_desktop_tool(&client, tool_name, params).await;
            }
            // 跨进程自动化锁：与其它 Agent 实例通过同一锁文件互斥，防止并发操作同一桌面。
            let lock = crate::utils::automation_lock::AutomationLock::new();
            let _lock_guard = match lock.acquire(tool_name) {
                Ok(guard) => guard,
                Err(e) => return Ok(ToolResult::failure(e)),
            };
            let client = self
                .desktop_client
                .read()
                .map_err(|e| e.to_string())?
                .as_ref()
                .cloned();
            if let Some(ref client) = client {
                return self.execute_desktop_tool(client, tool_name, params).await;
            }
        }

        // 否则使用注册的 executor（支持 canonical 名映射）
        let def = self.get(tool_name).ok_or_else(|| {
            format!(
                "未知工具「{}」。请仅使用工具列表中已注册的工具名，不要编造工具名。",
                tool_name
            )
        })?;

        // 捕获工具 panic + 将同步执行隔离到阻塞线程池，
        // 避免长时间工具（system_shell / system_sleep / 文件 IO 等）
        // 占用 tokio worker 线程导致整个 runtime 假死（取消无响应、LLM 流中断）。
        let executor = def.executor; // fn 指针 Copy + Send + 'static
        let params_owned = params.clone();
        // ToolCtx 携带本 registry 的共享信号句柄（src-tauri 启动时注入的唯一实例）
        let ctx = ToolCtx {
            signals: self.signals.clone(),
            schedule_tool: self.schedule_tool.read().ok().and_then(|slot| slot.clone()),
        };
        // 超时分档与每档取值推导见 Self::tool_timeout（无出处魔数禁止入链）
        let timeout = Self::tool_timeout(tool_name);
        match tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    executor(&params_owned, &ctx)
                }))
            }),
        )
        .await
        {
            Ok(Ok(panic_result)) => match panic_result {
                Ok(result) => result,
                Err(_panic_info) => {
                    let msg = format!("工具 '{}' 执行时发生内部错误（panic），已拦截", tool_name);
                    tracing::error!("[PANIC] {}", msg);
                    Ok(ToolResult::failure(msg))
                }
            },
            Ok(Err(join_err)) => {
                let msg = format!("工具 '{}' 执行线程异常退出: {}", tool_name, join_err);
                tracing::error!("[BLOCKING] {}", msg);
                Ok(ToolResult::failure(msg))
            }
            Err(_elapsed) => {
                // 文案口径钉在 Self::timeout_message：超时不取消任何东西，
                // 禁止出现「已取消」这类与实现矛盾的表述
                let msg = Self::timeout_message(tool_name, timeout);
                tracing::error!("[TIMEOUT] {}", msg);
                Ok(ToolResult::failure(msg))
            }
        }
    }

    /// 工具超时分级：`execute` 同步执行段（spawn_blocking）的等待上限。
    ///
    /// 纪律：每个档位必须能讲清「为什么是这个数」——无出处魔数禁止入链；
    /// 能复用已有档位就不新造数字。各档推导：
    /// - system_shell/system_sleep：**1800s，与 video 档同级**。原 600s 是
    ///   「容纳默认 180s + 余量」，但实测 Release 全量构建/测试在乾淨 target
    ///   下可跑 5-10 分钟，600s 会把长任务拦腰砍断。而本桶取消不了任何东西
    ///   （见下方 timeouts 说明），砍断只会诱导重试 → 重跑 → 再砍断的循环。
    ///   取宽档：宁可晚、不可早。工具的 `timeout` 参数上限同步提高到 1800s。
    /// - web_search/web_extract/http_request 走 reqwest::blocking 慢抓取，120s 防误杀
    /// - video_subtitle_extract 含 yt-dlp 下载 + ffmpeg 转码 + 本地 ASR，
    ///   长视频兜底链路给 900s（15min）
    /// - image_generate 同步出图走 web 桶；video_generate 异步轮询（默认
    ///   300s 上限 600s + 下载），给 900s 桶与 video_subtitle_extract 同级
    /// - Read 读扫描件 PDF 时走「前端渲染(≤60s) + 逐页 OCR」兜底链路，
    ///   普通文件读取仍是毫秒级，仅上限放宽，180s
    /// - memory_search semantic=true 首次调用需冷加载本地 candle embed 模型
    ///   （debug 构建下可达数十秒），默认 15s 桶会误杀，放宽到 60s
    /// - agent_dispatch：耗时完全由 team.toml 的 dispatch_steps 决定——序列环上
    ///   没有 LLM，默认四步（激活窗口/逐字输入/enter）仅 1–2s，打不穿任何档位；
    ///   实测能打穿 15s 兜底桶的是三种：① 长 message override——sendinput 逐字
    ///   注入 char_delay_ms=5 ⇒ 每千字符约 5s，3000 字符即压线；② 用户自写长
    ///   __sleep / 多步序列；③ 单步桌面调用偶发阻塞（window_activate 路径有
    ///   sleep 上限、input_send 每码点校验前台窗口）。取 180s（复用 Read 档位）
    ///   的关键理由：**本桶取消不了任何东西**——spawn_blocking 不可被 timeout
    ///   取消、ext_agent 侧亦无 kill 路径，桶超时只是让 Leader 早收一条文本，
    ///   后台序列照跑。所以桶的价值仅是「多晚停止说谎」：误报（太早超时）会
    ///   诱导 Leader 判投递中断而重投 ⇒ 双投递、两条序列还可能交错敲键。
    ///   宁可晚、不可早，取宽档。
    /// - 其余工具 15s 防文件系统卡死
    ///
    /// 注：desktop_/browser_ 工具在上方分支已提前返回，不经过此处
    fn tool_timeout(tool_name: &str) -> Duration {
        if tool_name == "system_shell" || tool_name == "system_sleep" {
            Duration::from_secs(1800) // 与 video 档同级，容纳 Release 全量构建/测试
        } else if tool_name == "web_search"
            || tool_name == "web_extract"
            || tool_name == "http_request"
            || tool_name == "image_generate"
        {
            Duration::from_secs(120)
        } else if tool_name == "video_subtitle_extract" || tool_name == "video_generate" {
            Duration::from_secs(900)
        } else if tool_name == "Read" {
            Duration::from_secs(180) // 容纳扫描 PDF「渲染 60s + 50 页 OCR」兜底上限
        } else if tool_name == "agent_dispatch" {
            // 与 Read 同档 180s，不引新魔数；取值推导见本函数文档
            Duration::from_secs(180)
        } else if tool_name == "memory_search" {
            Duration::from_secs(60) // 容纳 semantic 路径 embed 模型冷加载（debug 数十秒）
        } else {
            Duration::from_secs(15)
        }
    }

    /// 超时返回文案。纪律：必须讲实情——spawn_blocking 上的同步执行不可能被
    /// timeout 取消，收到这段文本时它通常仍在后台运行。写「已取消」会诱导
    /// 「可以安全重试」的误判（agent_dispatch 即双投递、两条序列交错敲键）。
    fn timeout_message(tool_name: &str, timeout: Duration) -> String {
        if tool_name == "agent_dispatch" {
            format!(
                "工具 'agent_dispatch' 执行超过 {}秒未返回（到达超时上限，非取消）。\n\
                 同步投递序列无法被强制中止——dispatch_steps 可能仍在后台逐条执行。\n\
                 处置：先核对目标窗口/进程实况（windows_list / 截图看回显），确认指令是否已进入终端：\n\
                 已进入 → 勿重复投递（双序列会交错敲键、任务被外部 Agent 执行两遍）；\n\
                 确认未进入 → 才补输单行指令「Read {{brief_path}} and execute it.」补完投递，brief 无需重新上板。",
                timeout.as_secs()
            )
        } else if tool_name == "system_shell" {
            format!(
                "工具 'system_shell' 已等待 {}秒仍未结束（到达等待上限）。\n\
                 若本次调用 timeout > 300s（长任务：构建/全量测试/大下载），进程**未被终止**，仍在后台运行 —— 输出若重定向到文件，稍后直接读取，**勿重复执行**。\n\
                 若为普通命令，进程已被终止，可修正后重试。",
                timeout.as_secs()
            )
        } else {
            format!(
                "工具 '{}' 执行超过 {}秒未返回（到达超时上限，非取消）。同步执行无法被超时强制中止，它可能仍在后台运行——重试前请先核对该工具的副作用实况（文件/窗口/进程），避免重复执行。",
                tool_name,
                timeout.as_secs()
            )
        }
    }

    /// Convert DesktopClient's serde_json::Value result to human-readable text
    /// (Implementation migrated to src/tools/desktop_executors.rs)
    ///
    /// Desktop tool executor mapping
    /// (Implementation migrated to src/tools/desktop_executors.rs)
    /// Set browser client
    /// (Implementation migrated to src/tools/browser_tools.rs)
    /// Execute browser tool (async)
    /// (Implementation migrated to src/tools/browser_tools.rs)
    /// Execute tool (with permission check, async)
    pub async fn execute_with_permission(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
        policy: &PermissionPolicy,
    ) -> std::result::Result<ToolResult, String> {
        // 映射 canonical 名回原始名（当前均为 _ 格式），供权限检查使用
        let resolved_name = self
            .canonical_map
            .get(tool_name)
            .map(|s| s.as_str())
            .unwrap_or(tool_name);

        // 1. 检查权限
        let outcome = match self.tools.get(resolved_name) {
            Some(tool) => policy.authorize_by_category(resolved_name, tool.category),
            None => PermissionOutcome::Allow,
        };
        if !outcome.is_allowed() {
            let reason = match outcome {
                PermissionOutcome::Allow => "allowed".to_string(),
                PermissionOutcome::Deny { ref reason } => reason.clone(),
            };
            return Ok(ToolResult::failure(format!(
                "Permission denied: {}",
                reason
            )));
        }

        // 2. SecurityGuard 安全检查（保护路径、危险命令、格式注入等）
        match crate::security::SecurityGuard::check(resolved_name, params) {
            crate::security::SecurityDecision::Deny { reason } => {
                return Ok(ToolResult::failure(format!("安全拦截: {}", reason)));
            }
            crate::security::SecurityDecision::RequireConfirmation { reason, .. } => {
                // Tauri 路径无审批弹窗机制，需确认的操作直接拒绝
                return Ok(ToolResult::failure(format!("需用户确认: {}", reason)));
            }
            crate::security::SecurityDecision::Allow => {}
        }

        // 3. 执行工具（execute 内部已支持 canonical 名映射）
        self.execute(tool_name, params).await
    }

    /// Get schemas for all tools (including built-in and desktop tools).
    /// Sorted by name for deterministic order — critical for DeepSeek prompt cache prefix match.
    pub fn get_schemas(&self) -> Vec<crate::api::ToolDefinition> {
        let mut schemas: Vec<_> = self
            .tools
            .values()
            .map(|t| crate::api::ToolDefinition {
                tool_type: "function".to_string(),
                function: crate::api::FunctionDefinition {
                    // API requires name matching ^[a-zA-Z0-9_-]+$, so return canonical name
                    name: t.name.replace("::", "_"),
                    description: Some(t.description.clone()),
                    parameters: t.parameters.clone(),
                    // Don't send permission to API (non-standard field, would cause DeepSeek etc. to reject)
                    permission: None,
                },
            })
            .collect();

        // 添加桌面工具 schema
        schemas.extend(self.get_desktop_schemas());

        // Sort by name for deterministic order (HashMap iteration is non-deterministic)
        schemas.sort_by(|a, b| a.function.name.cmp(&b.function.name));
        schemas
    }

    /// Get schemas filtered by name whitelist (case-sensitive exact match).
    /// Returns only tools whose function.name appears in the whitelist.
    pub fn get_schemas_for(&self, whitelist: &[String]) -> Vec<crate::api::ToolDefinition> {
        let all = self.get_schemas();
        all.into_iter()
            .filter(|td| {
                whitelist
                    .iter()
                    .any(|w| w.as_str() == td.function.name.as_str())
            })
            .collect()
    }

    /// Render tool schemas as JSON string, embedded in system prompt's <tools> tag
    ///
    /// Result is cached (lazy-built), automatically cleared when register adds/updates tools.
    pub fn render_tools_for_prompt(&self) -> String {
        // 尝试读缓存
        {
            let guard = self.prompt_cache.read().unwrap_or_else(|e| e.into_inner());
            if let Some(ref cached) = *guard {
                return cached.clone();
            }
        }

        // Cache miss → build
        let schemas = self.get_schemas();
        let result = render_tool_names_by_group(&schemas);

        // Write cache
        {
            let mut guard = self.prompt_cache.write().unwrap_or_else(|e| e.into_inner());
            *guard = Some(result.clone());
        }

        result
    }

    /// Render tool schemas filtered by a name whitelist (Custom mode).
    /// Bypasses prompt_cache — the whitelist is session/mode-specific, not global.
    pub fn render_tools_for_prompt_filtered(&self, whitelist: &[String]) -> String {
        let schemas = self.get_schemas_for(whitelist);
        render_tool_names_by_group(&schemas)
    }

    /// Get references to all tool definitions
    pub fn all_defs(&self) -> Vec<ToolDef> {
        self.tools.values().cloned().collect()
    }

    /// Return a cloned registry excluding the specified tool
    pub fn without_tool(&self, name: &str) -> Self {
        let mut cloned = self.clone();
        cloned.tools.remove(name);
        cloned
    }

    /// Get tool count
    pub fn len(&self) -> usize {
        let has_desktop = self
            .desktop_client
            .read()
            .map(|g| g.is_some())
            .unwrap_or(false);
        // builtin tools + browser (always) + desktop (conditional)
        self.tools.len() + 17 + if has_desktop { 22 } else { 0 }
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Get all tool names (derived from actual registrations + schemas, no hardcoded lists)
    pub fn tool_names(&self) -> Vec<String> {
        self.all_tool_categories()
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    /// Set DesktopClient
    pub fn set_desktop_client(&self, client: DesktopClient) {
        *self
            .desktop_targets
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(
            crate::desktop::targets::DesktopTargetService::new(client.clone()),
        ));
        let mut guard = self
            .desktop_client
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = Some(client);
    }

    pub(super) fn target_service(
        &self,
    ) -> Result<Arc<crate::desktop::targets::DesktopTargetService>, String> {
        self.desktop_targets
            .read()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or_else(|| "target_unavailable: 桌面目标服务尚未连接".to_string())
    }

    /// Install one semantic adapter instance for observation, candidate
    /// construction and execution. Sharing the same instance preserves the
    /// adapter's opaque candidate-id to native-locator mapping.
    pub fn set_semantic_desktop_adapter<T>(&mut self, adapter: Arc<T>)
    where
        T: crate::desktop_automation::ComputerObserver
            + crate::desktop_automation::CandidateBuilder
            + crate::desktop_automation::ComputerExecutor
            + 'static,
    {
        self.semantic_desktop = Some(SemanticDesktopBackend::new(adapter));
        if let Ok(mut guard) = self.prompt_cache.write() {
            *guard = None;
        }
    }

    /// Switch only the optional enhanced decision layer. UIA observation and
    /// candidate-id execution remain available in both modes.
    pub fn set_enhanced_mode(&mut self, enabled: bool) {
        if self.enhanced_mode != enabled {
            self.enhanced_mode = enabled;
            if let Ok(mut guard) = self.prompt_cache.write() {
                *guard = None;
            }
        }
    }

    pub fn enhanced_mode(&self) -> bool {
        self.enhanced_mode
    }

    /// Get DesktopClient clone (preserves original reference)
    pub fn desktop_client(&self) -> Option<DesktopClient> {
        self.desktop_client
            .read()
            .ok()
            .and_then(|g| g.as_ref().cloned())
    }

    /// Check if tool name is a desktop tool
    pub fn is_desktop_tool(name: &str) -> bool {
        name.starts_with("desktop_")
    }

    pub fn is_semantic_desktop_tool(name: &str) -> bool {
        matches!(
            name,
            "desktop_semantic_observe"
                | "desktop_semantic_candidate"
                | "desktop_targets_list"
                | "desktop_target_bind"
                | "desktop_semantic_execute"
                | "desktop_semantic_action"
                | "desktop_verify_state"
                | "desktop_agent_step"
        )
    }

    /// Check if tool name is a browser tool
    pub fn is_browser_tool(name: &str) -> bool {
        name.starts_with("browser_")
    }

    /// Get permission level required for desktop tools (DangerFullAccess, same as system_shell)
    pub fn desktop_tool_category() -> ToolCategory {
        ToolCategory::SystemAutomation
    }

    /// Build complete tool name → category map from ALL registered sources (ToolDef + schemas)
    pub fn all_tool_categories(&self) -> Vec<(String, ToolCategory)> {
        let mut result: Vec<(String, ToolCategory)> = Vec::new();
        for (name, def) in &self.tools {
            result.push((name.clone(), def.category));
        }
        for schema in self.get_desktop_schemas() {
            let name = schema.function.name;
            if result.iter().any(|(n, _)| n == &name) {
                continue;
            }
            let cat = if name.starts_with("browser_") {
                ToolCategory::WebSearch
            } else {
                ToolCategory::SystemAutomation
            };
            result.push((name, cat));
        }
        result
    }
}

// ── 工作流步骤可执行工具过滤（wf_tools 命令的唯一过滤来源）──

/// 工作流 tool 步骤不可执行的工具：agent 编排 / 记忆与会话检索 / 会话标注 /
/// 工作流管理 / 人机交互类。这些工具依赖 agent 会话上下文（dispatch、会话内
/// 暂停等待输入、工作流自编排），在 workflow step 语境下无执行语义。
/// 注意：wf_call 不在此列——它由 Executor 内部处理子工作流调用，必须保留。
pub const WORKFLOW_TOOL_EXCLUDE: &[&str] = &[
    // Agent 编排（Leader 专属）
    "task_dispatch",
    "planner_create",
    "planner_parse",
    "planner_archive",
    "planner_list",
    "tenet_add",
    // 记忆 / 会话检索（agent 上下文交互，注册表实际名见 definitions/memory.rs）
    "leader::memory_update",
    "workflow_memory_update",
    "memory_stats",
    "memory_search",
    "memory_recent",
    "memory_session_context",
    // 会话标注（Leader 专属）
    "annotation_add",
    "annotation_remove",
    "annotation_search",
    // 工作流管理（WorkflowAgent 编排自身，步骤不可执行）
    "workflow_run",
    "workflow_validate",
    "workflow_report_progress",
    "schedule_cron",
    // WorkflowAgent exploration helper. Saved workflows use ordinary semantic
    // actions and must not depend on a per-session enhanced-mode toggle.
    "desktop_agent_step",
    // 会话内人机交互（暂停等待输入，步骤语境无意义）
    "request_user_input",
];

/// 按前缀排除的工具族：UI 地图 / 经验库（agent 记忆沉淀类，见 builtin/ui_maps.rs、builtin/experience.rs）
pub const WORKFLOW_TOOL_EXCLUDE_PREFIX: &[&str] = &["ui_maps_", "experience_"];

/// wf_tools 过滤谓词：工具是否可作为工作流 tool 步骤执行（单一来源，前端不复制名单）
pub fn is_workflow_step_tool(name: &str) -> bool {
    !WORKFLOW_TOOL_EXCLUDE.contains(&name)
        && !WORKFLOW_TOOL_EXCLUDE_PREFIX
            .iter()
            .any(|p| name.starts_with(p))
}

// ── 工作流工具展示分组（wf_tools 的 group 字段唯一来源，前端不复制归属规则）──
//
// 注意：与 ToolCategory（src/permissions.rs，权限 taxonomy）语义不同，禁止复用。
// 这里分组键是纯展示语义，供画布工具面板分组渲染。

/// file 组成员：注册表真实工具名（见 definitions/file.rs 与 builtin/file.rs）
const WORKFLOW_GROUP_FILE: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Delete",
    "Rename",
    "Copy",
    "CreateDir",
    "RemoveDir",
    "ListDir",
    "FilesInfo",
    "Append",
    "Glob",
    "Grep",
    "Diff",
];

/// 工具名 → 展示分组键（6 组）：desktop / browser / file / system / generation / misc（兜底）
///
/// 分组语义按使用场景收敛：
/// - browser：browser_ 前缀族 + 网络访问三件套（web_search / web_extract / http_request）
/// - misc：兜底吸收无专属分组的工具（video_subtitle_extract / skill_* / knowledge_search / wf_call 等）
pub fn workflow_tool_group(name: &str) -> &'static str {
    if WORKFLOW_GROUP_FILE.contains(&name) {
        return "file";
    }
    if name.starts_with("desktop_") {
        return "desktop";
    }
    if name.starts_with("browser_") {
        return "browser";
    }
    if name.starts_with("system_") || name.starts_with("process_") {
        return "system";
    }
    match name {
        "web_search" | "web_extract" | "http_request" => "browser",
        "image_generate" | "video_generate" => "generation",
        _ => "misc",
    }
}

// ── Prompt 工具总览分组（system prompt 的「可用工具」节唯一来源）──
//
// 纯展示语义，与 ToolCategory（权限 taxonomy）、workflow_tool_group（画布面板）
// 均独立。按工具名的实际形态判定，不维护名单——不同 agent（Leader / Exec /
// Workflow / Custom 白名单）各自渲染出自己的分类，缺失的分组不出现。

/// 分组展示顺序（未列出的分组排在其后，保持字典序稳定）
pub const PROMPT_TOOL_GROUP_ORDER: &[&str] = &[
    "文件",
    "系统",
    "搜索",
    "记忆",
    "协作",
    "编排",
    "技能",
    "生成",
    "桌面",
    "浏览器",
    "工作流",
    "其它",
];

/// 工具名 → prompt 总览分组键。**纯判定函数，按名称形态归组；不匹配任何
/// 已知族时归入「其它」**——新增工具若不落族，会自然出现在「其它」而非消失。
pub fn prompt_tool_group(name: &str) -> &'static str {
    // 文件族（大小写混用，须显式列举）
    if matches!(
        name,
        "Read"
            | "Write"
            | "Edit"
            | "Delete"
            | "Rename"
            | "Copy"
            | "CreateDir"
            | "RemoveDir"
            | "ListDir"
            | "FilesInfo"
            | "Append"
            | "Glob"
            | "Grep"
            | "Diff"
            | "Open"
            | "Search"
    ) {
        return "文件";
    }
    if name.starts_with("system_") || name.starts_with("process_") {
        return "系统";
    }
    if name.starts_with("web_") || name == "http_request" {
        return "搜索";
    }
    if name.starts_with("desktop_") {
        return "桌面";
    }
    if name.starts_with("browser_") {
        return "浏览器";
    }
    if name.starts_with("memory_")
        || name.starts_with("annotation_")
        || name.starts_with("timeline_")
        || name.contains("memory_update")
    {
        return "记忆";
    }
    if name.starts_with("skill_") || name.starts_with("knowledge_") {
        return "技能";
    }
    if name.starts_with("workflow_")
        || name.starts_with("wf_")
        || name.starts_with("experience_")
        || name.starts_with("ui_maps_")
        || name == "schedule_cron"
    {
        return "工作流";
    }
    match name {
        "image_generate" | "video_generate" | "video_subtitle_extract" => "生成",
        "task_dispatch" | "agent_dispatch" | "request_user_input" => "协作",
        "planner_create" | "planner_parse" | "planner_archive" | "planner_list" | "tenet_add" => {
            "编排"
        }
        _ => "其它",
    }
}

/// 按 [`prompt_tool_group`] 聚合，渲染为「**分组名**\n工具名1, 工具名2, …」。
///
/// 分类 + 最省渲染：同类工具挤一行，名称以 `,` 分隔；空组不输出。
/// 详细参数由请求体 API `tools` 字段完整下发，本节仅承担「有哪些工具」的
/// 能力边界提示。
pub fn render_tool_names_by_group(schemas: &[crate::api::ToolDefinition]) -> String {
    let mut buckets: std::collections::BTreeMap<&'static str, Vec<String>> =
        std::collections::BTreeMap::new();
    for s in schemas {
        // 分组键按归一后的名字判定（`leader::memory_update` → `leader_memory_update`），
        // 避免 `::` 前缀族逃逸归类。
        let name = s.function.name.replace("::", "_");
        let group = prompt_tool_group(&name);
        buckets.entry(group).or_default().push(name);
    }

    let mut out = String::new();
    let emit = |group: &str, names: &[String], out: &mut String| {
        if names.is_empty() {
            return;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("**{group}**\n"));
        out.push_str(&names.join(", "));
        out.push('\n');
    };

    for group in PROMPT_TOOL_GROUP_ORDER {
        if let Some(names) = buckets.remove(group) {
            emit(group, &names, &mut out);
        }
    }
    // 兜底：未在顺序表列出的分组（不应发生，防御性保留）
    for (group, names) in buckets {
        emit(group, &names, &mut out);
    }
    out.trim_end().to_string()
}

// ── Built-in tools ──

impl ToolRegistry {
    // ── 内部辅助方法 ──

    /// 注册基础工具集（Leader / Exec / WorkflowAgent 共有 30 个工具）
    pub(crate) fn register_base_tools(&mut self) {
        // 文件 (11)
        self.register_read_file();
        self.register_write_file();
        self.register_edit_file();
        self.register_delete();
        self.register_rename();
        self.register_copy();
        self.register_create_dir();
        self.register_remove_dir();
        self.register_list_dir();
        self.register_files_info();
        self.register_append();
        // 系统 (7)
        self.register_system_info();
        self.register_diff();
        self.register_system_env_get();
        self.register_glob();
        self.register_grep();
        self.register_execute_shell();
        self.register_sleep();
        // 记忆只读 (3)
        self.register_search_timeline();
        self.register_recent_timeline();
        self.register_session_context();
        // Web (3)
        self.register_web_search();
        self.register_web_extract();
        self.register_http_request();
        // 视频字幕 (1)
        self.register_video_subtitle_extract();
        // 多模态生成 (2)
        self.register_image_generate();
        self.register_video_generate();
        // 进程 (2)
        self.register_process_list();
        self.register_process_kill();
        // 技能 (2)
        self.register_skill_query();
        self.register_skill_read();
        // 知识库 (1)
        self.register_knowledge_search();
    }

    /// Leader 独占工具（12 个）
    pub(crate) fn register_leader_only_tools(&mut self) {
        self.register_leader_memory_update();
        self.register_timeline_stats();
        self.register_task_dispatch();
        self.register_planner_create();
        self.register_planner_parse();
        self.register_planner_archive();
        self.register_planner_list();
        self.register_tenet_add();
        self.register_annotation_add();
        self.register_annotation_remove();
        self.register_annotation_search();
        self.register_agent_dispatch();
    }

    /// WorkflowAgent 独占工具
    pub(crate) fn register_workflow_only_tools(&mut self) {
        self.register(ToolDef {
            name: "workflow_report_progress".into(),
            description: "向用户简短说明打算、重要进展或遇到的问题，然后继续执行。首次实际操作前先说明打算；无需用户回复。不是最终总结，不应复制思考过程。".into(),
            parameters: serde_json::json!({
                "type":"object", "properties":{
                    "message":{"type":"string","description":"面向用户的简短自然语言说明，使用用户的语言"}
                }, "required":["message"], "additionalProperties":false
            }),
            category: crate::permissions::ToolCategory::Core,
            executor: |_, _| Err("workflow_report_progress 仅由 WorkflowAgent 投递".into()),
            depends_on: vec![],
        });
        self.register_workflow_run();
        self.register_workflow_validate();
        self.register_schedule_cron();
        self.register_ui_maps_tools();
        self.register_experience_tools();
        self.register_workflow_memory_update();
        // wf_call — 调用已保存的子工作流（Executor 内部处理）
        self.register(ToolDef {
            name: "wf_call".to_string(),
            description: "调用已保存的子工作流模块。传入 workflow_id 和 inputs，子流程执行后 outputs 回写父流程变量。支持模块复用和嵌套。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "workflow_id": { "type": "string", "description": "已保存的子工作流 ID" },
                    "inputs": { "type": "object", "description": "传入子流程的变量（key=子变量名, value=值或 {{变量引用}}）" },
                    "outputs": { "type": "object", "description": "子流程产出回写的映射（key=子变量名, value=父变量名）" }
                },
                "required": ["workflow_id"]
            }),
            category: crate::permissions::ToolCategory::Core,
            executor: |_, _| Ok(crate::ToolResult::success("wf_call handled by executor")),
            depends_on: vec![],
        });
    }

    /// Leader + WorkflowAgent 共用工具（1 个）
    pub(crate) fn register_shared_tools(&mut self) {
        self.register_request_user_input();
    }

    // ── 公有构造函数 ──

    /// Create Leader Agent tool set (same as Exec tools + task_dispatch)
    ///
    /// Leader has full tool schema visibility, can accurately determine which tool is appropriate,
    /// either executes directly or dispatches to Exec via task_dispatch.
    pub fn leader() -> Self {
        let mut registry = Self::new();
        registry.register_base_tools();
        registry.register_shared_tools();
        registry.register_leader_only_tools();
        registry.register_workflow_run();
        registry.register_workflow_validate();
        registry.load_depends_from_file("config/tool_deps.toml");
        tracing::info!("Registered {} leader tools", registry.len());
        registry
    }

    /// Create Leader Agent tool set (with desktop control capability)
    ///
    /// Adds DesktopClient on top of leader(), enabling Leader to directly perform
    /// lightweight desktop operations (screenshots, mouse/keyboard, window management, clipboard),
    /// without dispatching via task_dispatch every time.
    pub fn leader_with_desktop(client: DesktopClient) -> Self {
        let mut registry = Self::leader();
        registry.set_desktop_client(client);
        registry.install_platform_semantic_desktop();
        tracing::info!(
            "Leader registry with desktop tools ({} tools)",
            registry.len()
        );
        registry
    }

    /// CLI 模式使用的内置工具集。
    pub fn builtin() -> Self {
        let mut registry = Self::new();
        registry.register_base_tools();
        registry.register_shared_tools();
        // leader_only 不含 register_leader_memory_update 和 register_annotation_*
        registry.register_timeline_stats();
        registry.register_task_dispatch();
        registry.register_planner_create();
        registry.register_planner_parse();
        registry.register_planner_archive();
        registry.register_planner_list();
        registry.register_tenet_add();
        // 额外: workflow_run + schedule_cron + wf_call
        registry.register_workflow_run();
        registry.register_workflow_validate();
        registry.register_schedule_cron();
        // wf_call — 调用已保存的子工作流（Executor 内部处理）
        registry.register(ToolDef {
            name: "wf_call".to_string(),
            description: "调用已保存的子工作流模块。传入 workflow_id 和 inputs，子流程执行后 outputs 回写父流程变量。支持模块复用和嵌套。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "workflow_id": { "type": "string", "description": "已保存的子工作流 ID" },
                    "inputs": { "type": "object", "description": "传入子流程的变量（key=子变量名, value=值或 {{变量引用}}）" },
                    "outputs": { "type": "object", "description": "子流程产出回写的映射（key=子变量名, value=父变量名）" }
                },
                "required": ["workflow_id"]
            }),
            category: crate::permissions::ToolCategory::Core,
            executor: |_, _| Ok(crate::ToolResult::success("wf_call handled by executor")),
            depends_on: vec![],
        });
        registry.load_depends_from_file("config/tool_deps.toml");
        tracing::info!("Registered {} builtin tools", registry.len());
        registry
    }

    /// CLI + 桌面。
    pub fn builtin_with_desktop() -> Self {
        let mut registry = Self::builtin();
        let client = DesktopClient::new();
        registry.set_desktop_client(client);
        registry.install_platform_semantic_desktop();
        tracing::info!(
            "Registered {} builtin tools + 24 desktop tools",
            registry.len()
        );
        registry
    }

    /// Create Exec Agent tool set (tools needed by Exec)
    ///
    /// ExecAgent is Leader's dispatch sub-agent. It only needs base tools
    /// (file, system, memory-read, web, process, skills). No desktop/browser
    /// capability — those are Leader-only.
    pub fn exec() -> Self {
        let mut registry = Self::new();
        registry.register_base_tools();
        // 约束：Exec 执行自动化操作属黑盒 → 关闭 desktop_*/browser_* 的暴露与执行。
        // 单靠 register_base_tools 挡不住 browser：它在 get_desktop_schemas 里是无条件
        // 附加的，故必须在 registry 层显式收口（见 automation_tools_enabled 文档）。
        registry.automation_tools_enabled = false;
        registry.load_depends_from_file("config/tool_deps.toml");
        tracing::info!("Registered {} exec tools", registry.len());
        registry
    }

    /// Create WorkflowAgent tool set
    ///
    /// WorkflowAgent manages workflow design and execution. It gets base tools +
    /// workflow-specific tools (workflow_run, schedule_cron, ui_maps, experience,
    /// workflow_memory_update, wf_call) + desktop automation.
    pub fn work_agent() -> Self {
        let mut registry = Self::new();
        registry.register_base_tools();
        registry.register_shared_tools();
        registry.register_workflow_only_tools();
        let client = DesktopClient::new();
        registry.set_desktop_client(client);
        registry.install_platform_semantic_desktop();
        registry.load_depends_from_file("config/tool_deps.toml");
        tracing::info!("Registered {} work_agent tools", registry.len());
        registry
    }

    fn install_platform_semantic_desktop(&mut self) {
        #[cfg(windows)]
        self.set_semantic_desktop_adapter(Arc::new(
            crate::desktop_automation::WindowsUiaAdapter::default(),
        ));
        #[cfg(target_os = "macos")]
        self.set_semantic_desktop_adapter(Arc::new(
            crate::desktop_automation::MacosAccessibilityAdapter::default(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 角色工具集分离回归：Exec 必须是 Leader 的子集且严格更小。
    ///
    /// 钉住两件事：
    ///   1. `ToolRegistry::exec()` 只注册 base tools，不夹带 shared / workflow-only；
    ///   2. Exec 的工具集不允许膨胀到与 Leader 等同——一旦有人把
    ///      `register_shared_tools()` 加进 `exec()`，Exec 就凭空多出
    ///      task_dispatch / planner_* / memory_* 等编排类工具（越权 + token 浪费）。
    #[test]
    fn test_exec_toolset_is_strict_subset_of_leader() {
        let leader: std::collections::HashSet<String> = ToolRegistry::builtin()
            .get_schemas()
            .into_iter()
            .map(|s| s.function.name)
            .collect();
        let exec: std::collections::HashSet<String> = ToolRegistry::exec()
            .get_schemas()
            .into_iter()
            .map(|s| s.function.name)
            .collect();

        let leaked: Vec<&String> = exec.difference(&leader).collect();
        assert!(
            leaked.is_empty(),
            "Exec 含 Leader 之外的工具（角色边界泄漏）: {leaked:?}"
        );
        assert!(
            exec.len() < leader.len(),
            "Exec 工具集({}) 应严格小于 Leader({})",
            exec.len(),
            leader.len()
        );
        // 编排类工具不得进入 Exec——它们是 Leader 的职责
        for forbidden in ["task_dispatch", "planner_create", "agent_dispatch"] {
            assert!(
                !exec.contains(forbidden),
                "编排类工具 '{forbidden}' 不应出现在 Exec 工具集中"
            );
        }
        // 约束：Exec 执行自动化操作属黑盒 → desktop_*/browser_* 必须完全缺席。
        // 这是安全边界而非 token 优化：browser 工具在 get_desktop_schemas 里是无条件
        // 附加的，靠 register_base_tools 挡不住，必须由 automation_tools_enabled 收口。
        for prefix in ["desktop_", "browser_"] {
            let leaked: Vec<&String> = exec.iter().filter(|n| n.starts_with(prefix)).collect();
            assert!(
                leaked.is_empty(),
                "自动化工具不得进入 Exec 工具集（{prefix}）: {leaked:?}"
            );
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn workflow_semantic_tools_are_available_in_normal_mode() {
        let registry = ToolRegistry::work_agent();
        let names: std::collections::HashSet<_> = registry
            .get_schemas()
            .into_iter()
            .map(|schema| schema.function.name)
            .collect();
        assert!(names.contains("desktop_semantic_observe"));
        assert!(names.contains("desktop_semantic_execute"));
        assert!(names.contains("desktop_semantic_action"));
        assert!(!names.contains("desktop_agent_step"));
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn enhanced_mode_only_adds_the_bounded_decision_tool() {
        let mut registry = ToolRegistry::work_agent();
        let normal: std::collections::HashSet<_> = registry
            .get_schemas()
            .into_iter()
            .map(|schema| schema.function.name)
            .collect();
        registry.set_enhanced_mode(true);
        let enhanced: std::collections::HashSet<_> = registry
            .get_schemas()
            .into_iter()
            .map(|schema| schema.function.name)
            .collect();
        let added: Vec<_> = enhanced.difference(&normal).cloned().collect();
        assert_eq!(added, vec!["desktop_agent_step".to_string()]);
        assert!(normal.is_subset(&enhanced));
    }

    /// 自动化开关必须在「存在性判定」与「执行」两端同时生效。
    ///
    /// schema 层不暴露只是第一道防线：`has_tool` 一旦返回 true，react_loop 的
    /// 「未知工具」守卫就不会触发，模型凭历史上下文臆造的工具名会被一路放行到执行层。
    #[test]
    fn test_exec_automation_gate_is_two_sided() {
        let exec = ToolRegistry::exec();
        assert!(
            !exec.has_tool("browser_navigate"),
            "Exec 不得持有 browser 工具"
        );
        assert!(
            !exec.has_tool("desktop_mouse"),
            "Exec 不得持有 desktop 工具"
        );
        assert!(exec.has_tool("Read"), "Exec 必须保留文件工具");

        // Leader 侧不受影响
        let leader = ToolRegistry::builtin();
        assert!(
            leader.has_tool("browser_navigate"),
            "Leader 应保留 browser 工具"
        );
        assert!(
            leader.has_tool("desktop_mouse"),
            "Leader 应保留 desktop 工具"
        );
    }

    #[tokio::test]
    async fn semantic_tool_reenters_the_current_execution_body_owner() {
        let gate = Arc::new(AutomationGate::new());
        let owner = "execution-body:test".to_string();
        let body = gate
            .try_acquire(
                ResourceClass::ExecutionBody,
                HoldKind::ExecutionBody,
                owner.clone(),
            )
            .unwrap();
        let mut registry = ToolRegistry::new();
        registry.set_automation_gate(gate.clone());

        let result = crate::automation_gate::with_execution_owner(owner, async {
            registry.acquire_semantic_desktop_lease()
        })
        .await;
        assert!(result.unwrap().expect("gate configured").is_reentrant());
        drop(body);
        assert!(gate.is_free());
    }

    #[tokio::test]
    async fn semantic_tool_is_rejected_for_a_different_owner() {
        let gate = Arc::new(AutomationGate::new());
        let _body = gate
            .try_acquire(
                ResourceClass::ExecutionBody,
                HoldKind::ExecutionBody,
                "execution-body:running",
            )
            .unwrap();
        let mut registry = ToolRegistry::new();
        registry.set_automation_gate(gate);

        let error = registry.acquire_semantic_desktop_lease().unwrap_err();
        assert!(error.starts_with(crate::automation_gate::CODE_BUSY));
    }

    #[tokio::test]
    async fn failed_semantic_lease_does_not_leak_the_gate() {
        let gate = Arc::new(AutomationGate::new());
        let mut registry = ToolRegistry::new();
        registry.set_automation_gate(gate.clone());
        let lease = registry
            .acquire_semantic_desktop_lease()
            .unwrap()
            .expect("gate configured");
        assert!(!gate.is_free());
        drop(lease);
        assert!(gate.is_free());
    }

    /// 真实执行路径验证：Exec 的自动化工具必须在「通用执行入口」上被拒。
    ///
    /// 前面的断言都停在元数据层（has_tool / get_schemas）。本用例走
    /// `execute_tool_only` —— 所有 Agent 共用的真实分发点（browser 走
    /// execute_browser_tool、其余走 execute），确保闸门挡在实际调用上：
    /// 即使参数为空、即使模型臆造了工具名，也不允许触达 DesktopClient 或 MCP 通道。
    #[test]
    fn test_exec_automation_blocked_at_real_execution_entry() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let exec = ToolRegistry::exec();
            for tool in ["browser_navigate", "desktop_mouse"] {
                let result = crate::agent::exec_tool::execute_tool_only(
                    &exec,
                    tool,
                    &serde_json::json!({}),
                    None,
                    None,
                )
                .await;
                assert!(!result.success, "Exec 调用 {tool} 必须被拒，实际却成功了");
                let msg = result.error.unwrap_or_default();
                assert!(
                    msg.contains("automation tools disabled"),
                    "{tool} 的拒绝原因应指向自动化开关，实际: {msg}"
                );
            }
        });
    }

    /// wf_tools 过滤谓词回归：agent 编排/记忆/工作流管理类被排除，wf_call 与普通工具保留
    #[test]
    fn test_workflow_step_tool_filter() {
        for excluded in [
            "task_dispatch",
            "planner_create",
            "memory_search",
            "memory_stats",
            "workflow_run",
            "workflow_validate",
            "schedule_cron",
            "request_user_input",
            "annotation_add",
            "leader::memory_update",
            "workflow_memory_update",
            "ui_maps_search",
            "experience_save",
        ] {
            assert!(
                !is_workflow_step_tool(excluded),
                "{excluded} should be excluded from workflow step tools"
            );
        }
        for kept in [
            "wf_call",
            "Read",
            "Write",
            "system_shell",
            "web_search",
            "image_generate",
            "video_generate",
            "desktop_semantic_action",
        ] {
            assert!(
                is_workflow_step_tool(kept),
                "{kept} should stay executable as a workflow step tool"
            );
        }
    }

    #[test]
    fn test_builtin_registry() {
        let registry = ToolRegistry::builtin();
        assert!(registry.len() >= 7);
        assert!(registry.get("Read").is_some());
        assert!(registry.get("Write").is_some());
        assert!(registry.get("Edit").is_some());
    }

    #[test]
    fn test_execute_unknown_tool() {
        let registry = ToolRegistry::builtin();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(registry.execute("unknown_tool", &serde_json::json!({})));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("未知工具"));
    }

    #[test]
    fn test_execute_with_permission_readonly_blocks_shell() {
        let registry = ToolRegistry::builtin();
        let policy = PermissionPolicy::new(crate::permissions::ToolPermissions::none());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(registry.execute_with_permission(
            "system_shell",
            &serde_json::json!({"command": "dir"}),
            &policy,
        ));
        assert!(
            result.is_ok(),
            "execute_with_permission returns Ok even when denied"
        );
        assert!(
            !result.unwrap().success,
            "no permissions should deny system_shell"
        );
    }

    #[test]
    fn test_execute_with_permission_danger_allows_shell() {
        let registry = ToolRegistry::builtin();
        let policy = PermissionPolicy::new(crate::permissions::ToolPermissions::all());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(registry.execute_with_permission(
            "system_shell",
            &serde_json::json!({"command": "dir"}),
            &policy,
        ));
        // Should succeed when permission allows — may return error for actual execution failure (not permission)
        assert!(result.is_ok());
    }

    #[test]
    fn test_render_tools_for_prompt_contains_builtins() {
        let registry = ToolRegistry::builtin();
        let rendered = registry.render_tools_for_prompt();
        // Tool names are flattened (:: → _) for DeepSeek API compatibility
        assert!(rendered.contains("Read"), "Read missing from prompt");
        assert!(rendered.contains("Write"), "Write missing from prompt");
        assert!(
            rendered.contains("system_shell"),
            "system_shell missing from prompt"
        );
    }

    /// 工具总览契约：分类 + 名称（最省渲染），不含任何 description / 参数。
    ///
    /// 钉子：完整参数由 API `tools` 字段下发；一旦有人把 description 重新
    /// 拼回 prompt 总览（历史上曾占 ~4.5K token 纯冗余），本断言立刻失败。
    #[test]
    fn test_render_tools_for_prompt_is_grouped_names_only() {
        let registry = ToolRegistry::builtin();
        let rendered = registry.render_tools_for_prompt();
        // 分组标题形如 `**文件**`，紧跟一行逗号分隔的工具名
        assert!(rendered.contains("**文件**"), "缺少文件分组标题");
        assert!(rendered.contains("**系统**"), "缺少系统分组标题");
        assert!(rendered.contains("Read"), "文件组缺少 Read");
        // 不得含 description（`name: desc` 形态）与截断省略号
        assert!(!rendered.contains('…'), "总览不得含截断省略号");
        assert!(rendered.starts_with("**"), "应以分组标题开头");
        // 分组标题行不得带 `:`（那是 description 残留）
        for line in rendered.lines() {
            if line.starts_with("**") {
                assert!(line.ends_with("**"), "分组标题格式异常: {line}");
                assert!(!line.contains(':'), "分组标题不得含 ':'：{line}");
            }
        }
        // 每个已注册工具名都应出现（不因归类丢失）
        for s in registry.get_schemas() {
            let name = s.function.name.replace("::", "_");
            assert!(rendered.contains(&name), "工具 '{name}' 未出现在总览中");
        }
    }

    /// 分组判定：各族的代表工具归入预期分组，未知工具兜底「其它」。
    #[test]
    fn test_prompt_tool_group_mapping() {
        assert_eq!(prompt_tool_group("Read"), "文件");
        assert_eq!(prompt_tool_group("Grep"), "文件");
        assert_eq!(prompt_tool_group("system_shell"), "系统");
        assert_eq!(prompt_tool_group("process_kill"), "系统");
        assert_eq!(prompt_tool_group("web_search"), "搜索");
        assert_eq!(prompt_tool_group("http_request"), "搜索");
        assert_eq!(prompt_tool_group("desktop_mouse"), "桌面");
        assert_eq!(prompt_tool_group("browser_click"), "浏览器");
        assert_eq!(prompt_tool_group("memory_search"), "记忆");
        assert_eq!(prompt_tool_group("annotation_add"), "记忆");
        // `::` 归一后应归入记忆（leader_memory_update），不得落入「其它」
        assert_eq!(prompt_tool_group("leader_memory_update"), "记忆");
        assert_eq!(prompt_tool_group("skill_read"), "技能");
        assert_eq!(prompt_tool_group("knowledge_search"), "技能");
        assert_eq!(prompt_tool_group("image_generate"), "生成");
        assert_eq!(prompt_tool_group("task_dispatch"), "协作");
        assert_eq!(prompt_tool_group("planner_create"), "编排");
        assert_eq!(prompt_tool_group("workflow_run"), "工作流");
        // 未知工具兜底，不消失
        assert_eq!(prompt_tool_group("totally_new_tool"), "其它");
    }

    #[test]
    fn test_get_schemas_all_have_names() {
        let registry = ToolRegistry::builtin();
        let schemas = registry.get_schemas();
        assert!(schemas.len() >= 7);
        let re = regex::Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap();
        for s in &schemas {
            assert!(!s.function.name.is_empty(), "tool schema missing name");
            assert!(
                re.is_match(&s.function.name),
                "tool name '{}' does not match pattern ^[a-zA-Z0-9_-]+$",
                s.function.name
            );
        }
    }

    #[test]
    fn test_get_schemas_no_permission_field() {
        let registry = ToolRegistry::builtin();
        let schemas = registry.get_schemas();
        let json = serde_json::to_string_pretty(&schemas).unwrap();
        // DeepSeek API rejects extra fields in function object
        assert!(
            !json.contains("\"permission\""),
            "function object should not contain permission field for API compatibility"
        );
    }

    #[test]
    fn test_tool_names_includes_builtins() {
        let registry = ToolRegistry::builtin();
        let names = registry.tool_names();
        assert!(names.contains(&"Read".to_string()));
        assert!(names.contains(&"Write".to_string()));
    }

    #[test]
    fn test_workflow_tool_group() {
        // file：注册表真实名（大小写敏感）
        assert_eq!(workflow_tool_group("Read"), "file");
        assert_eq!(workflow_tool_group("Diff"), "file");
        assert_eq!(workflow_tool_group("ListDir"), "file");
        // desktop / browser：前缀族
        assert_eq!(workflow_tool_group("desktop_screenshot"), "desktop");
        assert_eq!(workflow_tool_group("browser_navigate"), "browser");
        // browser：网络访问三件套并入（原 web 组撤销）
        assert_eq!(workflow_tool_group("web_search"), "browser");
        assert_eq!(workflow_tool_group("web_extract"), "browser");
        assert_eq!(workflow_tool_group("http_request"), "browser");
        // system：system_ 与 process_ 前缀
        assert_eq!(workflow_tool_group("system_shell"), "system");
        assert_eq!(workflow_tool_group("system_info"), "system");
        assert_eq!(workflow_tool_group("process_list"), "system");
        assert_eq!(workflow_tool_group("process_kill"), "system");
        // generation
        assert_eq!(workflow_tool_group("image_generate"), "generation");
        assert_eq!(workflow_tool_group("video_generate"), "generation");
        // misc 兜底（原 media 组并入；含 wf_tools 内可见但无专属分组的工具）
        assert_eq!(workflow_tool_group("video_subtitle_extract"), "misc");
        assert_eq!(workflow_tool_group("wf_call"), "misc");
        assert_eq!(workflow_tool_group("skill_query"), "misc");
        assert_eq!(workflow_tool_group("knowledge_search"), "misc");
        assert_eq!(workflow_tool_group("nonexistent_tool"), "misc");
    }

    #[tokio::test]
    async fn test_execute_missing_params() {
        let registry = ToolRegistry::builtin();
        // Read requires "path" param
        let result = registry.execute("Read", &serde_json::json!({})).await;
        // Should return an error result, not panic
        assert!(result.is_ok() || result.is_err());
        if let Ok(tool_result) = result {
            assert!(!tool_result.success, "read_file without path should fail");
        }
    }

    /// issue #69 方向 3 回归：agent_dispatch 必须落在专属超时桶（180s，复用
    /// Read 档位），不得落「其余工具 15s」兜底桶。默认 team.toml 四步虽打不穿
    /// 15s，但长 message override（逐字注入 5ms/字符，每千字符约 5s）能打穿——
    /// 误报会诱导 Leader 判投递中断而双投递。
    #[test]
    fn test_agent_dispatch_has_dedicated_timeout_bucket() {
        assert_eq!(
            ToolRegistry::tool_timeout("agent_dispatch"),
            Duration::from_secs(180),
            "agent_dispatch 必须有独立超时桶（复用 Read 的 180s 档），不得落 15s 兜底桶"
        );
        // 档位表其余部分不被顺手改宽：Read 保持 180s，兜底桶保持 15s
        assert_eq!(ToolRegistry::tool_timeout("Read"), Duration::from_secs(180));
        assert_eq!(ToolRegistry::tool_timeout("Write"), Duration::from_secs(15));
    }

    /// issue #69 方向 3 回归：超时文案必须讲实情——spawn_blocking 不可取消，
    /// 禁止出现「已取消」这类与实现矛盾的表述（诱导安全重试 ⇒ 双投递/双执行）。
    ///
    /// 第二个断言只要求「不断言进程已被取消」：system_shell 自超时分层后，
    /// 普通命令（timeout ≤ 300s）确实是**会被终止**的，说「仍在后台运行」反而
    /// 与实现矛盾。故改为按工具分辨：dispatch/其余工具仍是「不可取消」口径，
    /// system_shell 只要不断言「未取消」即可（它按任务类型区分）。
    #[test]
    fn test_timeout_message_tells_the_truth() {
        for tool in ["agent_dispatch", "Read", "Write"] {
            let msg = ToolRegistry::timeout_message(tool, Duration::from_secs(180));
            assert!(
                !msg.contains("已取消"),
                "{tool} 的超时文案不得声称「已取消」（同步执行不可被取消）: {msg}"
            );
            assert!(
                msg.contains("非取消") && msg.contains("后台"),
                "{tool} 的超时文案必须说明「未取消 + 可能仍在后台运行」: {msg}"
            );
        }

        // system_shell：分层措辞，两种结局都要说明，且都不得声称「已取消」
        let shell_msg = ToolRegistry::timeout_message("system_shell", Duration::from_secs(180));
        assert!(
            !shell_msg.contains("已取消"),
            "system_shell 不得声称「已取消」: {shell_msg}"
        );
        assert!(
            shell_msg.contains("未被终止") && shell_msg.contains("已被终止"),
            "system_shell 文案必须同时说明长任务未终止与普通命令已终止两种结局: {shell_msg}"
        );
        // agent_dispatch 专属文案必须带「先核对实况、已进终端勿重投」指引
        let msg = ToolRegistry::timeout_message("agent_dispatch", Duration::from_secs(180));
        assert!(
            msg.contains("勿重复投递") && msg.contains("确认指令是否已进入终端"),
            "agent_dispatch 超时文案必须给出核对实况与勿重投指引: {msg}"
        );
    }
}
