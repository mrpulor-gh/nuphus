//! Tool registry — enhanced tool definitions and permission binding

use crate::browser::BrowserClient;
use crate::desktop::DesktopClient;
use crate::permissions::{PermissionOutcome, PermissionPolicy, ToolCategory};
use crate::ToolResult;
use serde::Deserialize;
use std::collections::HashMap;
use std::string::String;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Tool execution context — bundle of injected handles passed to every executor.
///
/// PR-2 (AppState 合并): 携带 `SharedSignals`（pause/security/workflow 信号句柄），
/// 供 tenet_add / request_user_input 等需要写信号状态的工具使用。
/// 设计见 docs/internal/2026-08-06-appstate-merge-design.md §2.3 方案 A。
#[derive(Debug, Clone, Default)]
pub struct ToolCtx {
    /// 共享信号状态句柄（由 ToolRegistry 在 execute() 注入）
    pub signals: crate::state::SharedSignals,
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
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self {
            tools: HashMap::new(),
            desktop_client: Arc::new(RwLock::new(None)),
            browser_client: crate::browser::shared_client(),
            prompt_cache: Arc::new(RwLock::new(None)),
            canonical_map: HashMap::new(),
            signals: crate::state::new_shared_signals(),
        }
    }
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        Self {
            tools: self.tools.clone(),
            desktop_client: self.desktop_client.clone(),
            browser_client: self.browser_client.clone(),
            prompt_cache: self.prompt_cache.clone(),
            canonical_map: self.canonical_map.clone(),
            signals: self.signals.clone(),
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
        self.get(name).is_some() || Self::is_desktop_tool(name) || Self::is_browser_tool(name)
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
            // 双通道（dogfooding）：MCP 优先，失败回退直连
            match crate::mcp::dual::route_tool(tool_name, params).await {
                crate::mcp::dual::RouteOutcome::Handled(result) => return Ok(result),
                crate::mcp::dual::RouteOutcome::Fallback(reason) => {
                    tracing::debug!(
                        "[dual] desktop '{}' falls back to direct: {}",
                        tool_name,
                        reason
                    );
                }
            }
            // 跨进程自动化锁：MCP 通道不可用而回退直连时，与各 nuphus-mcp 实例
            // （其他 Agent）通过同一锁文件互斥。MCP 可用时锁已在 nuphus-mcp 进程内
            // 获取/释放，这里不重复获取（否则会与 MCP 进程的锁自锁）。
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
        };
        // 超时分级：
        // - system_shell/system_sleep 自带超时机制，给 600s 容纳默认 180s + 余量
        // - web_search/web_extract/http_request 走 reqwest::blocking 慢抓取，给 120s 防误杀
        // - video_subtitle_extract 含 yt-dlp 下载 + ffmpeg 转码 + 本地 ASR，
        //   长视频兜底链路给 900s（15min）
        // - image_generate 同步出图走 web 桶；video_generate 异步轮询（默认
        //   300s 上限 600s + 下载），给 900s 桶与 video_subtitle_extract 同级
        // - Read 读扫描件 PDF 时走「前端渲染(≤60s) + 逐页 OCR」兜底链路，
        //   普通文件读取仍是毫秒级，仅上限放宽
        // - memory_search semantic=true 首次调用需冷加载本地 candle embed 模型
        //   （debug 构建下可达数十秒），默认 15s 桶会误杀，放宽到 60s
        // - 其余工具 15s 防文件系统卡死
        // 注：desktop_/browser_ 工具在上方分支已提前返回，不经过此处
        let timeout = if tool_name == "system_shell" || tool_name == "system_sleep" {
            Duration::from_secs(600) // 足够容纳默认 180s + 余量
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
        } else if tool_name == "memory_search" {
            Duration::from_secs(60) // 容纳 semantic 路径 embed 模型冷加载（debug 数十秒）
        } else {
            Duration::from_secs(15)
        };
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
                let msg = format!(
                    "工具 '{}' 执行超时（{}秒），已取消",
                    tool_name,
                    timeout.as_secs()
                );
                tracing::error!("[TIMEOUT] {}", msg);
                Ok(ToolResult::failure(msg))
            }
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
        let simplified: Vec<String> = schemas
            .iter()
            .map(|s| {
                // Tool names in prompt must match API schemas (:: → _)
                let normalized_name = s.function.name.replace("::", "_");
                let desc = s.function.description.as_deref().unwrap_or("");
                format!("- {}: {}", normalized_name, desc)
            })
            .collect();
        let result = simplified.join("\n");

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
        schemas
            .iter()
            .map(|s| {
                let normalized_name = s.function.name.replace("::", "_");
                let desc = s.function.description.as_deref().unwrap_or("");
                format!("- {}: {}", normalized_name, desc)
            })
            .collect::<Vec<_>>()
            .join("\n")
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
        let mut guard = self
            .desktop_client
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = Some(client);
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
    "planner_complete",
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
    "schedule_cron",
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

    /// Leader 独占工具（11 个）
    pub(crate) fn register_leader_only_tools(&mut self) {
        self.register_leader_memory_update();
        self.register_timeline_stats();
        self.register_task_dispatch();
        self.register_planner_create();
        self.register_planner_parse();
        self.register_planner_complete();
        self.register_planner_list();
        self.register_tenet_add();
        self.register_annotation_add();
        self.register_annotation_remove();
        self.register_annotation_search();
    }

    /// WorkflowAgent 独占工具
    pub(crate) fn register_workflow_only_tools(&mut self) {
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
        let registry = Self::leader();
        registry.set_desktop_client(client);
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
        registry.register_planner_complete();
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
        let registry = Self::builtin();
        let client = DesktopClient::new();
        registry.set_desktop_client(client);
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
        registry.load_depends_from_file("config/tool_deps.toml");
        tracing::info!("Registered {} work_agent tools", registry.len());
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
