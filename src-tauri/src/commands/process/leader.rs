//! leader — Leader Agent startup + model fallback chain
//!
//! build_runtime: Build Runtime instance from LLM config
//! run_runtime_with_config: Runtime entry point
//! execute_fallback_chain: When primary model fails, iterate other models in config.toml for fallback

use crate::state::HistoryMessage;
use nuphus::agent::events::EventEmitter;
use nuphus::agent::goal_types::RelationConfig;
use nuphus::agent::AgentConfig;
use nuphus::permissions::ToolPermissions;
use nuphus::runtime::{Mode, Runtime, RuntimeBuilder, RuntimeConfig};
use nuphus::session::Session;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

// ── Runtime ──

/// Build Runtime
pub(crate) fn build_runtime<E: EventEmitter + Clone>(
    llm: Arc<dyn nuphus::api::ApiClient>,
    tools: nuphus::ToolRegistry,
    config: &crate::state::LlamaConfig,
    tool_permissions: ToolPermissions,
    tool_permissions_ref: Arc<std::sync::Mutex<ToolPermissions>>,
    emitter: &E,
    pause_flag: &Arc<AtomicBool>,
    refine_threshold: f64,
) -> std::result::Result<Runtime, String> {
    let leader_registry = if let Some(dc) = tools.desktop_client() {
        nuphus::ToolRegistry::leader_with_desktop(dc)
    } else {
        nuphus::ToolRegistry::leader()
    };
    // 与 AppState 持有的全局唯一信号实例对齐（leader()/leader_with_desktop() 新建 registry 默认独立实例）
    let mut leader_registry = leader_registry;
    leader_registry.set_signals(tools.signals().clone());

    let builder = RuntimeBuilder::new()
        .llm(llm.clone())
        .tools(leader_registry)
        .config(RuntimeConfig {
            mode: Mode::Leader,
            agent_config: AgentConfig {
                model: config.model.clone(),
                provider: config.provider.clone(),
                tool_permissions,
                refine_threshold,
                reasoning_effort: config.reasoning_effort.clone(),
                ..Default::default()
            },
            refine_threshold,
            tool_permissions: tool_permissions_ref,
        })
        .emitter(Arc::new(emitter.clone()))
        .pause_flag(pause_flag.clone());

    let runtime = builder.build()?;

    Ok(runtime)
}

/// Execute Leader via Runtime (replaces legacy run_leader_with_config)
pub(crate) async fn run_runtime_with_config<E: EventEmitter + Clone>(
    llm: Arc<dyn nuphus::api::ApiClient>,
    exec_llm: Arc<dyn nuphus::api::ApiClient>,
    tools: nuphus::ToolRegistry,
    config: &crate::state::LlamaConfig,
    message: &str,
    images: &Option<Vec<String>>,
    history: &Option<Vec<HistoryMessage>>,
    relation: &Option<RelationConfig>,
    soul_content: &str,
    source: &str,
    tool_permissions: ToolPermissions,
    tool_permissions_ref: Arc<std::sync::Mutex<ToolPermissions>>,
    cancel_flag: &Arc<AtomicBool>,
    pause_flag: &Arc<AtomicBool>,
    emitter: &E,
    existing_runtime: Option<Runtime>,
    session_backup_json: Option<String>,
    refine_threshold: f64,
    mode: Option<nuphus::runtime::Mode>,
    workflow_engine: Arc<tokio::sync::RwLock<nuphus::workflow::WorkflowEngine>>,
    resume: bool,
) -> std::result::Result<(nuphus::AgentOutput, Runtime), String> {
    // ── Capture session from existing runtime before it's consumed ──
    // When config changes and a new Runtime is built, this backup preserves
    // the full session (including ToolUse/ToolResult blocks) that would be
    // lost if we fell through to Session::from_history (text-only).
    let backup_session = existing_runtime.as_ref().map(|rt| rt.session().clone());

    let mut runtime = if let Some(rt) = existing_runtime {
        let config_match = rt.config().model == config.model
            && rt.config().provider == config.provider
            && rt.config().reasoning_effort == config.reasoning_effort
            // Agent 级 exec 模型变化也触发 Runtime 重建（exec_llm 在构建时注入）
            && rt.exec_model() == exec_llm.model_name();
        if config_match {
            rt
        } else {
            build_runtime(
                llm.clone(),
                tools.clone(),
                config,
                tool_permissions,
                tool_permissions_ref.clone(),
                emitter,
                pause_flag,
                refine_threshold,
            )?
        }
    } else {
        build_runtime(
            llm.clone(),
            tools.clone(),
            config,
            tool_permissions,
            tool_permissions_ref.clone(),
            emitter,
            pause_flag,
            refine_threshold,
        )?
    };

    // Set execution resources — Exec 子任务使用独立 exec_llm（Agent 级配置，可不同于 Leader）
    runtime.set_exec_resources(
        nuphus::ToolRegistry::exec(),
        exec_llm.clone(),
        emitter.clone(),
    );

    // ── Apply mode: preserve mode from frontend (e.g., 'workflow') across Runtime rebuild ──
    if let Some(m) = mode {
        runtime.set_mode(m);
    }

    // Inject workflow engine (required before runtime.run() for workflow_run tool)
    runtime.set_workflow_engine(workflow_engine);

    // Set context
    runtime.set_context(soul_content, relation.clone());

    // ExecutionStarted is emitted by Runtime::run(), not duplicated here
    // Restore history (if any)
    // ── Fix: use full session backup (with ToolUse/ToolResult) instead of text-only from_history ──
    // from_history creates ContentBlock::Text only, losing all tool call/result blocks.
    // backup_session (captured before existing_runtime was consumed) preserves everything.
    if runtime.session().is_empty() {
        if let Some(ref backup) = backup_session {
            tracing::info!(
                "[LEADER] Restored full session from backup ({} msgs, id={})",
                backup.len(),
                backup.id
            );
            runtime.set_session(backup.clone());
        } else if let Some(ref json) = session_backup_json {
            // Try to restore full session from AppState backup (survives Tauri command cancellation)
            match serde_json::from_str::<nuphus::session::Session>(json) {
                Ok(session) if !session.is_empty() => {
                    tracing::info!(
                        "[LEADER] Restored full session from AppState backup ({} msgs, id={})",
                        session.len(),
                        session.id
                    );
                    runtime.set_session(session);
                }
                Ok(_) => {
                    tracing::warn!("[LEADER] AppState backup session was empty, falling through");
                }
                Err(e) => {
                    tracing::warn!(
                        "[LEADER] Failed to deserialize AppState backup session: {}",
                        e
                    );
                }
            }
        }
        if runtime.session().is_empty() {
            if let Some(ref history) = history {
                if !history.is_empty() {
                    tracing::warn!("[LEADER] No session backup, falling back to text-only from_history ({} msgs)", history.len());
                    let tuples: Vec<(String, String)> = history
                        .iter()
                        .map(|h| (h.role.clone(), h.content.clone()))
                        .collect();
                    let session = Session::from_history(tuples);
                    runtime.set_session(session);
                }
            }
        }
    }

    // Shelf 镜像恢复（优先于摘要注入）：完整装载最近一次会话，
    // 取代「[上次会话摘要]」降级体验——跨重启延续真实对话上下文
    if runtime.session().is_empty() {
        match crate::commands::process::shelf::load_latest_mirror() {
            Some((mode, sess)) if !sess.is_empty() => {
                // v1 仅 leader 侧走此恢复链；workflow 镜像留给展示台列表
                if mode == "workflow" {
                    tracing::info!("[LEADER] 最新镜像为 workflow 会话，跳过 leader 恢复");
                } else {
                    tracing::info!(
                        "[LEADER] Restored full session from shelf mirror ({} msgs, id={})",
                        sess.len(),
                        sess.id
                    );
                    runtime.set_session(sess);
                }
            }
            _ => {}
        }
    }

    // When no history, try to restore latest session from SQLite
    if runtime.session().is_empty() {
        if let Ok(Some(last_session)) = nuphus::store::session::latest_session() {
            let summary = if last_session.summary.is_empty() {
                format!(
                    "上次会话 (ID: {}) — {} 条消息",
                    &last_session.id[..8],
                    last_session.message_count
                )
            } else {
                last_session.summary.clone()
            };
            tracing::info!("[LEADER] Restored session {} from SQLite", last_session.id);
            runtime
                .session_mut()
                .push_user(format!("[上次会话摘要] {}", summary));
        }
    }

    // memory.md snapshot
    runtime.inject_memory_snapshot();

    // ── Apply mode before execution (covers new Runtime and config-changed paths) ──
    if let Some(m) = mode {
        runtime.set_mode(m);
        tracing::info!("[MODE] Mode applied before run: {:?}", m);
    }

    // ── Apply message source marker before execution (every round: the same
    // Runtime may alternate between desktop and mobile entries) ──
    runtime.set_source(source);

    // 执行（resume=true 走断点续跑：不 advance_turn、不 push_user，见 Runtime::resume）
    let output = if resume {
        runtime.resume(message, cancel_flag).await
    } else {
        runtime.run(message, images, cancel_flag).await
    }
    .map_err(|e| e.to_string())?;

    Ok((output, runtime))
}
