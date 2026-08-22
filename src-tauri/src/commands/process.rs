use crate::state::{AppState, HistoryMessage, ProcessInputResponse};
use nuphus::agent::events::{EventEmitter, NuphusEvent};
use nuphus::agent::goal_types::RelationConfig;
use serde::Deserialize;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::Manager;
use tauri::State;

// Submodule split
pub mod leader;
pub mod lifecycle;
pub mod mode;
pub mod refine;
pub mod retry;
pub mod session;

// Re-export public commands so commands::xxx remains accessible
pub use lifecycle::*;
pub use mode::*;
pub use retry::*;
pub use session::*;

// ============================================================================
// ChatReference — 前端传递的资源引用
// ============================================================================

/// 前端通过 ChatMessage.references 传递的资源引用
/// `type` 是 Rust 关键字，用 #[serde(rename)] 反序列化
#[derive(Debug, Clone, Deserialize)]
pub struct ChatReference {
    #[serde(rename = "type")]
    pub ref_type: String, // "skill" | "knowledge" | "workflow" | "capture"
    pub id: String,
    pub label: String,
}

/// 获取工作区根目录（CARGO_MANIFEST_DIR 的父目录）
pub(crate) fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 解析 references，读取对应资源内容，返回注入到 Leader 上下文的文本
async fn resolve_references(refs: &[ChatReference]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for r in refs {
        let content = match r.ref_type.as_str() {
            "skill" => {
                let skill_dir = workspace_root().join("plugin").join("skills").join(&r.id);
                let entry = skill_dir.join("SKILL.md");
                if entry.exists() {
                    tokio::fs::read_to_string(&entry).await.unwrap_or_default()
                } else {
                    format!("[Skill not found: {}]", r.id)
                }
            }
            "knowledge" => {
                let kd = workspace_root().join("plugin").join("knowledge");
                let entry = kd.join(format!("{}.md", r.id));
                if entry.exists() {
                    tokio::fs::read_to_string(&entry).await.unwrap_or_default()
                } else {
                    format!("[Knowledge not found: {}]", r.id)
                }
            }
            "workflow" => {
                let wf_dir = workspace_root()
                    .join("plugin")
                    .join("workflows")
                    .join(&r.id);
                let entry = wf_dir.join("workflow.json");
                if entry.exists() {
                    tokio::fs::read_to_string(&entry).await.unwrap_or_default()
                } else {
                    format!("[Workflow not found: {}]", r.id)
                }
            }
            // 截图引用：id 即本地图片绝对路径，注入带路径的图片提示，
            // Agent 据此用 desktop_vision(image_path=...) 查看（主模型不支持视觉）。
            "capture" => {
                format!("[📷 用户附带图片，已保存至: {}]", r.id)
            }
            _ => format!("[Unknown reference type: {}]", r.ref_type),
        };
        if !content.is_empty() {
            parts.push(format!("[{}] {}:\n{}", r.ref_type, r.label, content));
        }
    }
    parts.join("\n\n")
}

// ============================================================================
// send_message_cmd — Tauri 薄壳 / submit_user_message — 共享业务入口
// ============================================================================

/// 核心入口：发送消息（文本 + 可选图片 + 资源引用）
///
/// 薄壳：仅做 Tauri 参数适配（`State<'_, AppState>` → `&AppState`），
/// 业务逻辑全部委托给 [submit_user_message]，桌面端行为与原实现完全一致。
#[tauri::command]
pub async fn send_message_cmd(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    message: String,
    images: Option<Vec<String>>,
    history: Option<Vec<HistoryMessage>>,
    relation: Option<RelationConfig>,
    mode: Option<String>,
    references: Option<Vec<ChatReference>>,
    send_id: Option<String>,
) -> Result<ProcessInputResponse, String> {
    submit_user_message(
        app,
        state.inner(),
        message,
        images,
        history,
        relation,
        mode,
        references,
        send_id,
        None, // source 缺省 "desktop"，桌面行为与抽取前完全一致
    )
    .await
}

/// 判定是否重复提交（非 busy 受理路径）：同消息 + 同 send_id + 完成不足 10s。
/// 纯函数（可测）：dedup 防线核心，防刷新/重试导致的重复提交。
fn is_completion_duplicate(
    last_message: &str,
    message: &str,
    last_send_id: &Option<String>,
    send_id: &Option<String>,
    elapsed_since_completion_secs: u64,
) -> bool {
    last_message == message && last_send_id == send_id && elapsed_since_completion_secs < 10
}

/// 判定是否重复追加（busy 追加路径）：同消息 + 受理不足 30s。
/// 纯函数（可测）：busy 期间防刷新/重试导致追加指令重复注入。
fn is_append_duplicate(
    last_message: &str,
    message: &str,
    elapsed_since_process_start_secs: u64,
) -> bool {
    last_message == message && elapsed_since_process_start_secs < 30
}

/// 共享业务入口：发送消息的完整处理逻辑（桌面 / 移动端共用）。
///
/// 当前由 [send_message_cmd]（Tauri 桌面入口）与 mobile_server 的
/// POST /message（HTTP 手机入口，source="mobile"）调用，两个入口走完全
/// 相同的业务路径。
///
/// 注意：事件发射经 [CompoundEmitter]（桌面 Tauri + 手机 WS 双推；
/// mobile_server 未启动时退化为纯 Tauri）。Runtime 泛化（默认 Wry）使
/// 集成测试可用 MockRuntime 驱动完整调用链；下方 spawn 内仍通过
/// `app_handle.state::<AppState>()` 重新获取状态（Runtime P0 保护模式，
/// commit 2fd603e），该模式原样保留。
pub async fn submit_user_message<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: &AppState,
    message: String,
    images: Option<Vec<String>>,
    history: Option<Vec<HistoryMessage>>,
    relation: Option<RelationConfig>,
    mode: Option<String>,
    references: Option<Vec<ChatReference>>,
    send_id: Option<String>,
    source: Option<String>,
) -> Result<ProcessInputResponse, String> {
    // Must contain at least text or images
    if message.trim().is_empty() && images.as_ref().map(|v| v.is_empty()).unwrap_or(true) {
        return Err("Message cannot be empty".to_string());
    }

    // 身份配置缓存：桌面端 soul 随消息传入（非 None）时更新 relation_cache，
    // 手机端（localStorage 隔离、无配置通道）发消息不传 relation 时由 mobile_server 用此兜底。
    if let Some(rel) = &relation {
        if let Ok(mut cache) = state.relation_cache.write() {
            *cache = Some(rel.clone());
        }
    }

    // Message source marker: desktop 入口缺省 "desktop"，mobile HTTP 入口传 "mobile"
    let source = source.unwrap_or_else(|| "desktop".to_string());

    // Duplicate prevention
    {
        let guard = state.session.lock().map_err(|e| e.to_string())?;
        if is_completion_duplicate(
            &guard.last_message,
            &message,
            &guard.last_send_id,
            &send_id,
            state.elapsed_since_completion(),
        ) {
            return Ok(ProcessInputResponse {
                success: true,
                message: String::new(),
                appended: None,
                image_warning: None,
            });
        }
        // Reserve slot, only add after completion (prevent concurrent misjudgment)
        drop(guard);
    }

    // 执行中（busy）：不拒绝、不丢弃——消息入队为追加指令（与移动端一致），
    // 由 react_loop 迭代边界 drain 注入；短时间多条合并不覆盖。
    // 去重防线：与最近受理/已注入内容相同且在 30s 内 → 丢弃（防刷新/重试导致的重复提交）。
    // 主指令受理时会写 guard.last_message（见下方非 busy 分支），此处直接复用该记录。
    if state.busy.load(Ordering::SeqCst) {
        if !message.trim().is_empty() {
            let duplicate = {
                let guard = state.session.lock().map_err(|e| e.to_string())?;
                is_append_duplicate(
                    &guard.last_message,
                    &message,
                    state.elapsed_since_process_start(),
                )
            };
            if duplicate {
                // 不记消息内容（日志导出会携带对话敏感片段）——长度 + send_id 足够定位
                tracing::info!(
                    "[Dedup] 丢弃重复追加指令（30s 内已受理）: len={} send_id={:?}",
                    message.chars().count(),
                    send_id
                );
            } else {
                nuphus::mobile_append::push(message.clone());
            }
        }
        return Ok(ProcessInputResponse {
            success: true,
            // message 回传原始追加内容，前端弹窗直接显示消息本身（不显示解释性文案）
            message: message.clone(),
            appended: Some(true),
            image_warning: None,
        });
    }

    // Prevent concurrent execution
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("Task is already running, please wait for completion".to_string());
    }
    // ⚠️ 不在此处创建函数级 BusyGuard：Tauri command future 可能因 IPC break
    // （页面刷新/导航/command 取消）被 drop，函数作用域 guard 会随之释放 busy，
    // 但下方 spawn 任务继续运行 → 执行中手机端追加会被误判为新任务（新 session）。
    // busy 锁生命周期必须与 spawn 任务严格绑定（任务内 TaskBusyGuard 持有并释放）。
    // 因此 spawn 之前的所有提前返回路径必须显式 state.busy.store(false)。

    // Message dedup (based on completion time + start time double check)
    {
        let mut guard = state.session.lock().map_err(|e| {
            state.busy.store(false, Ordering::SeqCst);
            e.to_string()
        })?;
        let elapsed_from_start = state.elapsed_since_process_start();
        let elapsed_from_completion = state.elapsed_since_completion();
        if guard.last_message == message
            && guard.last_send_id == send_id
            && (elapsed_from_start < 30 || elapsed_from_completion < 30)
        {
            // Dedup detected — return success instead of error so frontend retries stop harmlessly
            state.busy.store(false, Ordering::SeqCst);
            return Ok(ProcessInputResponse {
                success: true,
                message: String::new(),
                appended: None,
                image_warning: None,
            });
        }
        guard.last_message = message.clone();
        guard.last_send_id = send_id.clone();
        guard.last_message_images = images.clone().unwrap_or_default();
        state.record_process_start();
    }

    let cancel_flag = state.cancel_flag.clone();
    cancel_flag.store(false, Ordering::SeqCst);
    state.pause_flag.store(false, Ordering::SeqCst);

    let refine_threshold = state
        .runtime
        .lock()
        .map_err(|e| {
            state.busy.store(false, Ordering::SeqCst);
            e.to_string()
        })?
        .refine_threshold;

    // ── Create ClientFactory from full model registry (config.toml / env) ──
    // 完整 registry 含全部已配置 provider/model（api_key 存于 config.toml），
    // 使 create_client(任意模型) 可用——Leader/Workflow/Exec/Custom 各 agent 可独立模型。
    // Priority 1: 完整 registry；Priority 2: in-memory from_single（startup llm_config）。
    let factory = {
        nuphus::config::load_registry()
            .ok()
            .map(nuphus::llm::ClientFactory::new)
            .or_else(|| {
                let in_mem = state.runtime.lock().ok().and_then(|g| g.llm_config.clone());
                in_mem
                    .filter(|c| !c.model.is_empty() && !c.api_key.is_empty())
                    .map(|cfg| {
                        let registry = nuphus::config::ModelRegistry::from_single(
                            cfg.model.clone(),
                            cfg.provider.clone(),
                            cfg.api_key.clone(),
                            String::new(),
                            cfg.reasoning_effort.clone(),
                        );
                        nuphus::llm::ClientFactory::new(registry)
                    })
            })
    }
    .ok_or_else(|| "无法加载模型配置，请检查 config.toml".to_string())
    .inspect_err(|_| {
        state.busy.store(false, Ordering::SeqCst);
    })?;

    let model_name = factory.registry().model.clone();
    let tools = state.tools.clone();
    // CompoundEmitter: mobile_server 运行时事件双推（桌面 + 手机 WS），否则纯 Tauri
    let emitter = crate::emitter::CompoundEmitter::new(app.clone(), state);

    emitter.emit(NuphusEvent::SessionInfo {
        session_id: uuid::Uuid::new_v4().to_string(),
        model: model_name.clone(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    });

    // ════════════════════════════════════════════════════════════════
    // New architecture: Leader Agent (ReAct + read tools + task::dispatch)
    // ════════════════════════════════════════════════════════════════

    let mode_parsed = mode
        .as_deref()
        .and_then(|m| nuphus::runtime::Mode::from_str(m).ok());
    let is_workflow = mode_parsed == Some(nuphus::runtime::Mode::Workflow);

    // 发送确认当前模式：前端传的 mode 与后端权威一致（手机端发消息默认带 mode，
    // 桌面端 set_mode 已显式切换；None 兼容旧调用——保持 current_mode 不变）。
    // current_mode 是独立 RwLock，不持有 runtime 锁，无死锁风险。
    // 与 set_mode_impl 对齐：写入归一化值（未知/旧版残留 free/plan → leader），
    // 避免脏字符串污染权威状态（chat_history 按 current_mode 选择 agent 会话）。
    if mode.is_some() {
        if let Ok(mut cm) = state.current_mode.write() {
            *cm = mode_parsed.unwrap_or_default().as_str().to_string();
        }
    }

    // Backup current session (prevent session loss if agent is consumed on failure)
    // workflow 模式备份 workflow_agent.session()：workflow 消息存于 workflow_agent 独立
    // session（leader_agent.session() 不增长），且执行中 agent 被 take 后若只备份 leader
    // 会是旧/空数据——手机端执行中重进页面 session_backup 为空，拉不到 workflow 历史。
    let backup_session = state.runtime.lock().ok().and_then(|guard| {
        if is_workflow {
            guard
                .workflow_agent
                .as_ref()
                .map(|agent| agent.session().clone())
        } else {
            guard
                .leader_agent
                .as_ref()
                .map(|agent| agent.session().clone())
        }
    });

    // Persist session backup to AppState so it survives Tauri command cancellation (IPC break).
    // When IPC breaks mid-execution, the command's async task may be cancelled,
    // dropping local variables. Writing to AppState ensures the session survives.
    if let Some(ref session) = backup_session {
        if let Ok(json) = serde_json::to_string(session) {
            if let Ok(mut sb) = state.session.lock() {
                sb.session_backup = Some(json);
                tracing::info!(
                    "[BACKUP] Session persisted to AppState ({} bytes)",
                    sb.session_backup.as_ref().map(|s| s.len()).unwrap_or(0)
                );
            }
        }
    }

    let (existing_agent, existing_workflow_agent) = {
        let mut guard = state.runtime.lock().map_err(|e| {
            state.busy.store(false, Ordering::SeqCst);
            e.to_string()
        })?;
        if is_workflow {
            (None, guard.workflow_agent.take())
        } else {
            let mut agent = guard.leader_agent.take();
            // ── Apply mode BEFORE run, not after ──
            if let Some(ref m) = mode {
                if let Ok(parsed) = nuphus::runtime::Mode::from_str(m) {
                    if let Some(ref mut rt) = agent {
                        rt.set_mode(parsed);
                        tracing::info!("[MODE] Pre-applied mode from frontend: {}", m);
                    }
                }
            }
            (agent, None)
        }
    };

    // ═══════════════════════════════════════════════════════════════════════
    // Clone everything needed for spawned task (detached from Tauri Future)
    // If Tauri cancels this command (IPC break), the spawned task keeps
    // running and will store the Runtime back to state — preventing P0 loss.
    // ═══════════════════════════════════════════════════════════════════════
    let app_handle = app.clone();
    let cancel_flag2 = cancel_flag.clone();
    let pause_flag2 = state.pause_flag.clone();
    let factory2 = factory.clone();
    let tools2 = tools.clone();
    let message2 = message.clone();
    let images2 = images.clone();
    let references2 = references.clone();
    let history2 = history.clone();
    let relation2 = relation.clone();
    let mode2 = mode.clone();
    let _backup_session2 = backup_session.clone();
    let refine_threshold2 = refine_threshold;
    let existing_workflow_agent2 = existing_workflow_agent;
    let is_workflow2 = is_workflow;
    let source2 = source.clone();

    let join_handle = tokio::spawn(async move {
        let state = app_handle.state::<AppState>();
        // ⚠️ 任务生命周期持 busy 锁：Tauri command future 可能因 IPC break（页面刷新/
        // 导航/command 取消）被 drop，函数作用域 BusyGuard 会随之释放——但 spawn 任务
        // 继续运行。若锁被提前释放，执行中手机端追加会被误判为新任务（新 session）。
        // 因此任务内部重新持有锁：busy 状态与任务运行期严格绑定，IPC break 不再放锁。
        state.busy.store(true, Ordering::SeqCst);
        struct TaskBusyGuard<'a>(&'a AtomicBool);
        impl Drop for TaskBusyGuard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _task_guard = TaskBusyGuard(&state.busy);
        // CompoundEmitter: mobile_server 运行时事件双推（桌面 + 手机 WS），否则纯 Tauri
        let emitter = crate::emitter::CompoundEmitter::new(app_handle.clone(), state.inner());
        // ── Agent 级模型解析（单一入口 effective_model）：
        //    leader(锚点) → default → 各自 agent；「可用」= registry 命中。──
        let registry = factory2.registry();
        let leader_model = crate::commands::config::llm::effective_model(
            &state.llm_config_path,
            registry,
            "leader",
        );
        let workflow_model = crate::commands::config::llm::effective_model(
            &state.llm_config_path,
            registry,
            "workflow",
        );
        let exec_model =
            crate::commands::config::llm::effective_model(&state.llm_config_path, registry, "exec");
        let custom_model = crate::commands::config::llm::effective_model(
            &state.llm_config_path,
            registry,
            "custom",
        );

        // create_client：effective_model 已保证模型可用（registry 命中），此处仅创建客户端。
        let resolve_llm =
            |model: &str| -> Result<(Arc<dyn nuphus::api::ApiClient>, String), String> {
                let llm = factory2
                    .create_client(model)
                    .map_err(|e| format!("创建 LLM 客户端失败 ({model}): {e}"))?;
                Ok((llm, model.to_string()))
            };
        let (leader_llm, leader_model) = resolve_llm(&leader_model)?;
        let (workflow_llm, workflow_model) = resolve_llm(&workflow_model)?;
        let (exec_llm, _exec_model) = resolve_llm(&exec_model)?;
        let (custom_llm, custom_model) = resolve_llm(&custom_model)?;

        // 当前 mode 的活动模型：Custom 走 leader 路径但用 custom 专属模型
        let (active_model, active_llm) = if !is_workflow2 && mode2.as_deref() == Some("custom") {
            (custom_model.clone(), custom_llm.clone())
        } else {
            (leader_model.clone(), leader_llm.clone())
        };
        let active_provider = factory2
            .registry()
            .find_model(&active_model)
            .map(|(p, _)| p.name.clone())
            .unwrap_or_default();
        let main_config = crate::state::LlamaConfig {
            model: active_model.clone(),
            provider: active_provider,
            ..Default::default()
        };

        let (leader_config, leader_llm) = (main_config.clone(), active_llm);

        // Read session backup from AppState (survives Tauri command cancellation on IPC break)
        let session_backup_json = state
            .session
            .lock()
            .ok()
            .and_then(|g| g.session_backup.clone());

        // Clone for retry error handler (run_runtime_with_config takes ownership)
        let session_backup_json_retry = session_backup_json.clone();

        // ── Soul file ──
        let soul_content2 = String::new(); // Soul passed via RelationConfig, no longer read from file

        // ── Resolve references (skill/knowledge/workflow) and prepend to message ──
        let effective_message = if let Some(ref refs) = references2 {
            if !refs.is_empty() {
                let resolved = resolve_references(refs).await;
                if !resolved.is_empty() {
                    tracing::info!(
                        "[REFERENCES] Resolved {} reference(s), prepending to user message ({} chars)",
                        refs.len(),
                        resolved.len()
                    );
                    format!("{}\n\n---\n\n{}", resolved, &message2)
                } else {
                    message2.clone()
                }
            } else {
                message2.clone()
            }
        } else {
            message2.clone()
        };

        // ── Run Agent (main model, dual-slot: Leader or Workflow) ──
        let start = std::time::Instant::now();

        let (output, fallback_used, fallback_model) = if is_workflow2 {
            // ══════════════ WORKFLOW PATH ══════════════
            let (user_label, assistant_name) = relation2
                .as_ref()
                .map(|r| {
                    let ul = if r.user_label.is_empty() {
                        "用户"
                    } else {
                        &r.user_label
                    };
                    let an = if r.assistant_name.is_empty() {
                        "Nuphus"
                    } else {
                        &r.assistant_name
                    };
                    (ul.to_string(), an.to_string())
                })
                .unwrap_or_else(|| ("用户".to_string(), "Nuphus".to_string()));

            let mut wa = if let Some(mut wa) = existing_workflow_agent2 {
                // Agent 级模型变更：workflow 模型变化 → 换 llm + 更新 model_label（保留 session）
                if wa.model_label() != workflow_model {
                    tracing::info!(
                        "[WORKFLOW] model changed {} → {}, swapping llm (session preserved)",
                        wa.model_label(),
                        workflow_model
                    );
                    wa.set_llm(workflow_llm.clone(), workflow_model.clone());
                }
                wa
            } else {
                let mut tools = nuphus::ToolRegistry::work_agent();
                // 与 AppState 持有的全局唯一信号实例对齐
                tools.set_signals(state.signals.clone());
                let perms = state
                    .runtime
                    .lock()
                    .map(|g| g.tool_permissions)
                    .unwrap_or(nuphus::permissions::ToolPermissions::default());
                let mut new_wa = nuphus::runtime::WorkflowAgent::new(
                    workflow_llm.clone(),
                    tools,
                    Some(Arc::new(emitter.clone())),
                    Some(pause_flag2.clone()),
                    workflow_model.clone(),
                    user_label.clone(),
                    assistant_name.clone(),
                    perms,
                    refine_threshold2,
                );
                new_wa.set_workflow_engine(state.workflow_engine.clone());
                new_wa
            };

            wa.set_source(&source2);
            wa.sync_before_run(
                Some(Arc::new(emitter.clone())),
                &user_label,
                &assistant_name,
                state
                    .runtime
                    .lock()
                    .map(|g| g.tool_permissions)
                    .unwrap_or(nuphus::permissions::ToolPermissions::default()),
                Some(state.workflow_engine.clone()),
            );
            wa.inject_memory_snapshot();

            match wa.run(&effective_message, &images2, &cancel_flag2).await {
                Ok(output) => {
                    if let Ok(mut guard) = state.runtime.lock() {
                        guard.workflow_agent = Some(wa);
                    }
                    (output, false, Option::<String>::None)
                }
                Err(e) => {
                    let err_str = e.to_string();
                    // Put the agent back so session is preserved, then propagate error
                    if let Ok(mut guard) = state.runtime.lock() {
                        guard.workflow_agent = Some(wa);
                    }
                    if cancel_flag2.load(Ordering::SeqCst) {
                        return Err(err_str);
                    }
                    emitter.emit(NuphusEvent::DirectResponse {
                        message: format!("⚠ WorkflowAgent error: {}", err_str),
                    });
                    return Err(err_str);
                }
            }
        } else {
            // ══════════════ LEADER PATH ══════════════
            match leader::run_runtime_with_config(
                leader_llm.clone(),
                exec_llm.clone(),
                tools2.clone(),
                &leader_config,
                &effective_message,
                &images2,
                &history2,
                &relation2,
                &soul_content2,
                &source2,
                state
                    .runtime
                    .lock()
                    .map(|g| g.tool_permissions)
                    .unwrap_or(nuphus::permissions::ToolPermissions::default()),
                state.tool_permissions_ref.clone(),
                &cancel_flag2,
                &pause_flag2,
                &emitter,
                existing_agent,
                session_backup_json,
                refine_threshold2,
                mode_parsed,
                state.workflow_engine.clone(),
                false,
            )
            .await
            {
                Ok((output, mut runtime)) => {
                    // ── Apply mode from frontend ──
                    if let Some(ref m) = mode2 {
                        if let Ok(parsed) = nuphus::runtime::Mode::from_str(m) {
                            runtime.set_mode(parsed);
                            tracing::info!("[MODE] Applied mode from frontend: {}", m);
                        }
                    }
                    tracing::info!(
                        "[RUNTIME] run_runtime_with_config succeeded, saving runtime to state (session: {} msgs, id={})",
                        runtime.session().len(),
                        runtime.session().id
                    );
                    if let Ok(mut guard) = state.runtime.lock() {
                        guard.leader_agent = Some(runtime);
                    }
                    (output, false, Option::<String>::None)
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if cancel_flag2.load(Ordering::SeqCst) {
                        tracing::info!(
                            "[AGENT] Err branch with cancel_flag=true, skipping agent rebuild"
                        );
                        return Err(err_str);
                    }

                    // Retryable errors are handled by agent layer, return directly
                    if nuphus::agent::common::is_retryable_llm_error(&err_str) {
                        // Rebuild agent to prevent missing session context on next request
                        if let Ok(mut guard) = state.runtime.lock() {
                            if guard.leader_agent.is_none() {
                                if let Ok(mut fresh) =
                                    leader::build_runtime(
                                        leader_llm.clone(),
                                        tools2.clone(),
                                        &main_config,
                                        state.runtime.lock().map(|g| g.tool_permissions).unwrap_or(
                                            nuphus::permissions::ToolPermissions::default(),
                                        ),
                                        state.tool_permissions_ref.clone(),
                                        &emitter,
                                        &pause_flag2,
                                        refine_threshold2,
                                    )
                                {
                                    tracing::info!(
                                        "[RETRY] Rebuilt fresh agent for retryable error"
                                    );
                                    if let Some(ref json) = session_backup_json_retry {
                                        if let Ok(sess) = serde_json::from_str(json) {
                                            fresh.set_session(sess);
                                            tracing::info!("[RETRY] Restored session from backup");
                                        }
                                    }
                                    guard.leader_agent = Some(fresh);
                                }
                            }
                        }
                        if let Some(ref retry_json) = session_backup_json_retry {
                            if let Ok(mut pending) = state.execution.lock() {
                                pending.pending_retry = Some((
                                    retry_json.clone(),
                                    main_config.clone(),
                                    message2.clone(),
                                ));
                                tracing::info!("[RETRY] Saved retryable session to pending_retry");
                            }
                        }
                        (
                            nuphus::AgentOutput {
                                message: format!("[retryable] {}", err_str),
                                success: false,
                                steps: Vec::new(),
                                retry_session: session_backup_json_retry.clone(),
                            },
                            false,
                            Option::<String>::None,
                        )
                    } else {
                        // ── Non-retryable error: try fallback chain ──
                        let rebuild_result = {
                            let m2 = mode2
                                .as_deref()
                                .and_then(|m| nuphus::runtime::Mode::from_str(m).ok());
                            leader::run_runtime_with_config(
                                leader_llm.clone(),
                                exec_llm.clone(),
                                tools2.clone(),
                                &leader_config,
                                &effective_message,
                                &images2,
                                &history2,
                                &relation2,
                                &soul_content2,
                                &source2,
                                state
                                    .runtime
                                    .lock()
                                    .map(|g| g.tool_permissions)
                                    .unwrap_or(nuphus::permissions::ToolPermissions::default()),
                                state.tool_permissions_ref.clone(),
                                &cancel_flag2,
                                &pause_flag2,
                                &emitter,
                                None,
                                session_backup_json_retry.clone(),
                                refine_threshold2,
                                m2,
                                state.workflow_engine.clone(),
                                false,
                            )
                            .await
                        };

                        match rebuild_result {
                            Ok((output, runtime)) => {
                                if let Ok(mut guard) = state.runtime.lock() {
                                    guard.leader_agent = Some(runtime);
                                }
                                (output, false, Option::<String>::None)
                            }
                            Err(e2) => {
                                return Err(format!(
                                    "主模型 + 恢复全部失败 (model={}): {} | rebuild_err: {}",
                                    leader_config.model, err_str, e2
                                ));
                            }
                        }
                    }
                }
            }
        };

        // ── Response message (fallback annotation) ──
        let elapsed = start.elapsed().as_millis() as u64;
        let _goal_types: Vec<String> = output
            .steps
            .iter()
            .filter_map(|s| s.goal_type.clone())
            .collect();

        if let Ok(guard) = state.runtime.lock() {
            if let Some(ref _runtime) = guard.leader_agent {
                let total_tool_calls = output.steps.len();
                tracing::info!(
                    "[RUNTIME] Post-run diagnostics (elapsed={}ms): msg={}, tools={}, outlen={}",
                    elapsed,
                    output.message.len(),
                    total_tool_calls,
                    output.message.chars().count(),
                );
            }
        }

        // ExecutionCompleted 由各 Agent（Leader/WorkflowAgent）内部自行发射，
        // 此处不再重复推送，避免前端收到双次完成状态。

        // Track elapsed time
        let _elapsed = elapsed;

        // If failed but has recoverable session, save to pending_retry
        if !output.success {
            if let Some(ref retry_json) = output.retry_session {
                if !retry_json.is_empty() {
                    let saved_config = if fallback_used {
                        state.runtime.lock().ok().and_then(|g| g.llm_config.clone())
                    } else {
                        Some(main_config.clone())
                    };
                    if let Some(cfg) = saved_config {
                        if let Ok(mut pending) = state.execution.lock() {
                            pending.pending_retry =
                                Some((retry_json.clone(), cfg, message2.clone()));
                            tracing::info!(
                                "[RETRY] Saved failed session to pending_retry for user retry"
                            );
                        }
                    }
                }
            }
        }

        // If fallback model was used, annotate the message
        let mut response_message = if let Some(ref model_id) = fallback_model {
            format!("[Fell back to {}] {}", model_id, output.message)
        } else {
            output.message
        };

        // ── Empty response guard: avoid silent "断裂" when output is empty ──
        if response_message.trim().is_empty() && output.success {
            response_message = "（模型未产出有效回复，可能需重试）".to_string();
            tracing::warn!(
                "[EMPTY] Empty response with success=true, overridden with fallback message"
            );
        }

        // Update completion time for next dedup check
        state.record_completion();

        // Memory: persist Leader turn to SQLite — only for Leader mode
        if !is_workflow2 {
            persist_leader_turn(state.inner(), &message2, &response_message, output.success);
        }

        // ── Post-processing: refine (dual-slot) ──
        if is_workflow2 {
            let mut wa_opt = {
                let mut guard = state.runtime.lock().unwrap_or_else(|e| e.into_inner());
                guard.workflow_agent.take()
            };
            if let Some(ref mut wa) = wa_opt {
                emitter.emit(NuphusEvent::TokenUsage {
                    source: "workflow".to_string(),
                    input_tokens: wa.session().estimate_token_usage() as u32,
                    output_tokens: 0,
                    cache_hit_tokens: u32::MAX,
                });
                let cw = nuphus::agent::goal_types::get_context_window(&leader_config.model);
                wa.maybe_refine_session(cw, refine_threshold2, Some(&emitter))
                    .await;
                let mut guard = state.runtime.lock().unwrap_or_else(|e| e.into_inner());
                guard.workflow_agent = wa_opt.take();
            }
        } else {
            // Leader post-processing
            let mut runtime_opt = {
                let mut guard = state.runtime.lock().unwrap_or_else(|e| e.into_inner());
                guard.leader_agent.take()
            };
            if let Some(ref mut rt) = runtime_opt {
                let session_usage = rt.session().estimate_token_usage() as u32;
                emitter.emit(NuphusEvent::TokenUsage {
                    source: "leader".to_string(),
                    input_tokens: session_usage,
                    output_tokens: 0,
                    cache_hit_tokens: u32::MAX,
                });

                let cw = nuphus::agent::goal_types::get_context_window(&rt.config().model);
                let refine_threshold = rt.config().refine_threshold;
                rt.maybe_refine_session(&cancel_flag2, cw, refine_threshold)
                    .await;

                {
                    let mut guard = state.runtime.lock().unwrap_or_else(|e| e.into_inner());
                    guard.leader_agent = runtime_opt.take();
                }
            }
        }

        // 图片降级警告：主模型与 vision 模型都不支持视觉时，前端弹窗提示（图片仍降级发送，不阻塞）
        // 判定与 runtime build（loop.rs resolve_vision_strategy）同源：load_registry + 主模型 supports_vision
        let image_warning = if images2.as_ref().map(|v| !v.is_empty()).unwrap_or(false) {
            let registry = nuphus::config::load_registry().ok();
            let main_model = registry
                .as_ref()
                .map(|r| r.model.clone())
                .unwrap_or_default();
            let strategy = nuphus::session::image::resolve_image_strategy(
                registry
                    .as_ref()
                    .and_then(|r| r.find_model(&main_model).map(|(_, m)| m.supports_vision))
                    .unwrap_or(false),
                match nuphus::config::resolve_vision_strategy() {
                    nuphus::config::VisionStrategy::Capability(m) => Some(m),
                    nuphus::config::VisionStrategy::Main => Some(main_model),
                    nuphus::config::VisionStrategy::None => None,
                }
                .as_deref(),
            );
            if strategy == nuphus::session::image::ImageStrategy::None {
                Some(
                    "当前模型不支持图片理解，且未配置视觉模型（capabilities.vision）。\
                     图片已保存发送，但 AI 无法查看图片内容——请在 设置→模型→自定义配置 中选择视觉模型。"
                        .to_string(),
                )
            } else {
                None
            }
        } else {
            None
        };

        Ok::<_, String>(ProcessInputResponse {
            success: output.success,
            message: response_message,
            appended: None,
            image_warning,
        })
    });

    let result = join_handle
        .await
        .map_err(|e| format!("Task panicked: {}", e))??;

    // Record completed send_id (fixed 256-entry ring buffer, prevent IPC retry)
    if let Some(ref sid) = send_id {
        if result.success {
            if let Ok(mut guard) = state.execution.lock() {
                if guard.completed_send_ids.len() >= 256 {
                    guard.completed_send_ids.pop_front();
                }
                guard.completed_send_ids.push_back(sid.clone());
            }
        }
    }

    Ok(result)
}

// ============================================================================
// execute_session_refine — thin wrapper delegating to refine module
// ============================================================================

/// After user confirms refine via frontend, delegate to refine.rs dual-slot logic.
#[tauri::command]
pub async fn execute_session_refine(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    refine::execute_session_refine(app, state).await
}

/// 桌面端「跳过提炼」：本地关闭弹窗 + 广播 RefineSkipped（双端同步关闭）。
#[tauri::command]
pub fn refine_skip(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    refine::refine_skip(app, state)
}

// ============================================================================
// persist_leader_turn — shared with retry_agent
// ============================================================================
fn persist_leader_turn(
    state: &AppState,
    user_message: &str,
    assistant_message: &str,
    success: bool,
) {
    use nuphus::memory::entry::{AgentType, MemoryEntry, MemoryKind};
    use nuphus::store::memory;

    let (entry_id, session_id, turn_id) = {
        let guard = match state.runtime.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let rt = match guard.leader_agent.as_ref() {
            Some(rt) => rt,
            None => return,
        };
        let session = rt.session();
        let turn_id = session.current_turn_id();
        // ⚠️ 主键必须含 session 维度：旧格式 `leader-{turn}-000` 被跨会话同 turn
        // REPLACE 覆盖（历史对话静默丢失）。session_id 取前 8 字符保持可读。
        let sid: String = session.id.chars().take(8).collect();
        let entry_id = format!("leader-{}-{}-000", sid, turn_id);
        (entry_id, session.id.clone(), turn_id)
    };

    let now = chrono::Utc::now();
    let entry = MemoryEntry {
        id: entry_id,
        session_id,
        turn_id,
        sequence: 0,
        created_at: now.to_rfc3339(),
        wall_clock_ms: now.timestamp_millis() as u64,
        agent_type: AgentType::Leader,
        kind: MemoryKind::Conversation,
        task_chain_id: None,
        chain_step: None,
        goal_type: None,
        tags: Vec::new(),
        // ⚠️ intent/summary 是 FTS + embedding 的唯一索引字段（FTS 不含
        // user_message/assistant_message 列），留空 = 对话全文存了但检索不到。
        intent: user_message.chars().take(100).collect(),
        summary: assistant_message.chars().take(300).collect(),
        user_message: user_message.to_string(),
        assistant_message: assistant_message.to_string(),
        tools_used: Vec::new(),
        success,
        output: None,
        artifacts: Vec::new(),
        is_marked: false,
        execution_steps: Vec::new(),
        parent_id: None,
        children_ids: Vec::new(),
        pattern: None,
        custom_agent_id: None,
    };
    let _ = memory::insert_entry(&entry);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── dedup 防线纯函数回归 ──
    // 历史事故：追加指令重复注入（busy 路径）与刷新重试重复提交（非 busy 路径）。
    // 这两个判定已抽为纯函数，以下测试固化边界语义。

    #[test]
    fn completion_duplicate_blocks_same_message_and_send_id_within_10s() {
        assert!(is_completion_duplicate(
            "你好",
            "你好",
            &Some("s1".into()),
            &Some("s1".into()),
            5
        ));
        // 边界：刚好 10s → 不拦截（放行，避免永久卡死重试）
        assert!(!is_completion_duplicate(
            "你好",
            "你好",
            &Some("s1".into()),
            &Some("s1".into()),
            10
        ));
        // 超过 10s → 放行
        assert!(!is_completion_duplicate(
            "你好",
            "你好",
            &Some("s1".into()),
            &Some("s1".into()),
            15
        ));
    }

    #[test]
    fn completion_duplicate_distinguishes_message_and_send_id() {
        // 不同消息 → 放行
        assert!(!is_completion_duplicate(
            "你好",
            "再见",
            &Some("s1".into()),
            &Some("s1".into()),
            1
        ));
        // 同消息但 send_id 不同（新的一次显式提交）→ 放行
        assert!(!is_completion_duplicate(
            "你好",
            "你好",
            &Some("s1".into()),
            &Some("s2".into()),
            1
        ));
        // send_id 全 None（移动端/历史入口）→ 同消息 10s 内仍拦截
        assert!(is_completion_duplicate("你好", "你好", &None, &None, 1));
        // 空消息 → 不构成重复
        assert!(!is_completion_duplicate(
            "你好",
            "",
            &Some("s1".into()),
            &Some("s1".into()),
            1
        ));
    }

    #[test]
    fn append_duplicate_blocks_same_message_within_30s() {
        assert!(is_append_duplicate("继续", "继续", 5));
        // 边界：刚好 30s → 放行（允许 30s 后的同文追加为有意重发）
        assert!(!is_append_duplicate("继续", "继续", 30));
        assert!(!is_append_duplicate("继续", "继续", 35));
        // 不同消息 → 放行（多条追加不合并）
        assert!(!is_append_duplicate("继续", "换个话题", 1));
        assert!(!is_append_duplicate("继续", "", 1));
    }
}