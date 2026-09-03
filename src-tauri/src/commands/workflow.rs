//! workflow.rs — Tauri commands for workflow management
//!
//! 精简后的命令列表：保留 CRUD + 规划 + 执行。
//! 画布命令（wf_validate / wf_save / wf_run）：IR 唯一真源，保存前强制权威校验。

use nuphus::workflow::compiler::{Compiler, ValidationReport};
use nuphus::workflow::types::Workflow;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::{AppState, ToolSchema};

// ── Response types ──

#[derive(Debug, Serialize, Deserialize)]
pub struct WfListResponse {
    pub workflows: Vec<Workflow>,
}

/// wf_save 响应：saved=false 时 report 携带阻断原因（前端回 ProblemsPanel）
#[derive(Debug, Serialize, Deserialize)]
pub struct WfSaveResponse {
    pub saved: bool,
    pub report: ValidationReport,
}

// ── CRUD ──

#[tauri::command]
pub async fn wf_list(state: State<'_, AppState>) -> Result<WfListResponse, String> {
    let engine = state.workflow_engine.read().await;
    let workflows = engine.list_workflows().await;
    Ok(WfListResponse { workflows })
}

#[tauri::command]
pub async fn wf_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let engine = state.workflow_engine.read().await;
    engine.delete_workflow(&id).await.map_err(|e| e.to_string())
}

// ── 执行 ──

#[tauri::command]
pub async fn wf_stop(
    _app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    let engine = state.workflow_engine.read().await;
    engine.cancel_workflow(&id).await;
    nuphus::workflow::hud_control::mark_user_cancelled();
    tracing::info!("[wf_stop] Cancelled workflow: {}", id);
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn wf_pause(
    _app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let engine = state.workflow_engine.read().await;
    engine.pause_workflow(&id).await;
    tracing::info!("[wf_pause] Paused workflow: {}", id);
    Ok(())
}

#[tauri::command]
pub async fn wf_resume(
    _app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let engine = state.workflow_engine.read().await;
    engine.resume_workflow(&id).await;
    tracing::info!("[wf_resume] Resumed workflow: {}", id);
    Ok(())
}
// ── 画布命令（校验 / 保存 / 运行）──

/// 工作流画布工具选择器数据源：全量注册表按 WORKFLOW_TOOL_EXCLUDE 过滤，
/// 仅保留 workflow tool 步骤可执行的工具（排除 agent 编排/记忆/工作流管理类）。
/// 仅用于选择器展示；wf_validate/wf_save 的权威校验仍使用全量注册表，不受其影响。
#[tauri::command]
pub fn wf_tools(state: State<'_, AppState>) -> Result<Vec<ToolSchema>, String> {
    let schemas = state.tools.get_schemas();
    Ok(schemas
        .into_iter()
        .filter(|s| nuphus::tools::registry::is_workflow_step_tool(&s.function.name))
        .map(|s| {
            let group = nuphus::tools::registry::workflow_tool_group(&s.function.name).to_string();
            ToolSchema {
                name: s.function.name,
                description: s
                    .function
                    .description
                    .as_deref()
                    .unwrap_or_default()
                    .to_string(),
                input_schema: s.function.parameters,
                group: Some(group),
            }
        })
        .collect())
}

/// 校验工作流（不修改数据）。
/// 与执行前校验同源：Compiler::validate_workflow（含工具注册表）+ validate_calls（循环链 DFS）。
#[tauri::command]
pub async fn wf_validate(
    state: State<'_, AppState>,
    workflow: Workflow,
) -> Result<ValidationReport, String> {
    let engine = state.workflow_engine.read().await;
    let tool_schemas = engine.tools().map(|t| t.get_schemas());
    let mut report = match tool_schemas.as_deref() {
        Some(schemas) => Compiler::validate_workflow_with_tools(&workflow, schemas),
        None => Compiler::validate_workflow(&workflow),
    };
    report
        .errors
        .extend(Compiler::validate_calls(&workflow, &engine.store).await);
    report.passed = report.errors.is_empty();
    Ok(report)
}

/// 保存工作流（画布唯一写回路径）。
/// 保存前强制权威校验：errors 非空 → 阻断并回传报告（saved=false），前端落 ProblemsPanel。
#[tauri::command]
pub async fn wf_save(
    state: State<'_, AppState>,
    mut workflow: Workflow,
) -> Result<WfSaveResponse, String> {
    let engine = state.workflow_engine.read().await;
    let tool_schemas = engine.tools().map(|t| t.get_schemas());
    let mut report = match tool_schemas.as_deref() {
        Some(schemas) => Compiler::validate_workflow_with_tools(&workflow, schemas),
        None => Compiler::validate_workflow(&workflow),
    };
    report
        .errors
        .extend(Compiler::validate_calls(&workflow, &engine.store).await);
    report.passed = report.errors.is_empty();

    if !report.passed {
        tracing::warn!(
            "[wf_save] Validation blocked save for '{}': {:?}",
            workflow.id,
            report.errors
        );
        return Ok(WfSaveResponse {
            saved: false,
            report,
        });
    }

    workflow.updated_at = Some(chrono::Utc::now());
    engine
        .store
        .save(&workflow)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!("[wf_save] Saved workflow: {}", workflow.id);
    Ok(WfSaveResponse {
        saved: true,
        report,
    })
}

/// 执行前注入 LLM client + ToolRegistry（ChatAgent 步骤依赖）。
/// wf_run 与 plugin_workflow_run 共用（对齐 workflow_agent 执行前注入逻辑）；
/// 调用方须已持有 workflow_engine 的写锁。
pub fn inject_workflow_runtime(state: &AppState, engine: &mut nuphus::workflow::WorkflowEngine) {
    let llm_config = state
        .runtime
        .lock()
        .ok()
        .and_then(|g| g.llm_config.clone())
        .filter(|c| !c.model.is_empty() && !c.api_key.is_empty());
    engine.set_tools(std::sync::Arc::new(state.tools.clone()));
    // 完整 registry 工厂：chat 步骤 with.model 按模型 ID 路由专属 provider
    if let Ok(full_registry) = nuphus::config::load_registry() {
        engine.set_client_factory(nuphus::llm::ClientFactory::new(full_registry));
    }
    if let Some(cfg) = llm_config {
        let registry = nuphus::config::ModelRegistry::from_single(
            cfg.model.clone(),
            cfg.provider.clone(),
            cfg.api_key.clone(),
            cfg.base_url.clone(),
            cfg.reasoning_effort.clone(),
        );
        let factory = nuphus::llm::ClientFactory::new(registry);
        match factory.create_main_client() {
            Ok(client) => engine.set_llm_client(client),
            Err(e) => tracing::warn!("[workflow] Failed to create LLM client: {}", e),
        }
    }
}

/// 从画布确定性触发执行（同 id 自动断点续连，execute.rs 语义）。
/// 与 workflow_run 工具同一条 execute_workflow 链路；前台交互运行，
/// 不施加 scheduler 的 has_frontend_step 后台限制（desktop_/browser_ 工具本就面向桌面前台）。
/// 异步 spawn：立即返回，进度经 workflow-event 事件流推送。
#[tauri::command]
pub async fn wf_run(state: State<'_, AppState>, id: String) -> Result<String, String> {
    // 注入 LLM client + ToolRegistry（ChatAgent 步骤依赖），与 plugin_workflow_run 共用公共函数
    {
        let mut engine = state.workflow_engine.write().await;
        inject_workflow_runtime(&state, &mut engine);
    }

    let engine = state.workflow_engine.clone();
    let tools = state.tools.clone();

    // ── 全局执行闸门前置（大王铁律）──
    // 让 UI 直接拿到拒绝（而非 spawn 内 execute_workflow 拒绝后只有服务端日志）；
    // 后端 execute_workflow 仍会二次校验（防 spawn 竞态窗口）。
    {
        let engine_r = engine.read().await;
        let active = engine_r.active_run_info();
        if active.is_some() || state.busy.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("当前有任务执行中，暂不可用！".to_string());
        }
    }

    tauri::async_runtime::spawn(async move {
        let tool_exec = move |tool: String, params: serde_json::Value| {
            let tools = tools.clone();
            async move {
                // browser_ 工具需走异步入口（ToolRegistry::execute 会拒绝）
                let result = if tool.starts_with("browser_") {
                    tools.execute_browser_tool(&tool, &params).await
                } else {
                    tools.execute(&tool, &params).await
                }
                .map_err(|e| e.to_string())?;
                result.into_exec_result()
            }
        };
        let engine_r = engine.read().await;
        let tool_schemas = engine_r.tools().map(|t| t.get_schemas());
        if let Err(e) = engine_r
            .execute_workflow(
                &id,
                tool_exec,
                tool_schemas,
                None,
                None,
                nuphus::workflow::WorkflowRunSource::Ui,
            )
            .await
        {
            tracing::error!("[wf_run] Workflow {} failed: {}", id, e);
        }
    });
    Ok("started".to_string())
}

// ── 画布布局 sidecar（canvas.layout.json，位置元数据不污染 IR）──

fn canvas_layout_path(id: &str) -> std::path::PathBuf {
    nuphus::utils::workspace_root()
        .join("plugin")
        .join("workflows")
        .join(id)
        .join("canvas.layout.json")
}

/// 读取画布布局 sidecar；文件不存在/损坏返回 None（前端回退全量自动布局）
#[tauri::command]
pub async fn wf_layout_get(id: String) -> Result<Option<serde_json::Value>, String> {
    let path = canvas_layout_path(&id);
    match tokio::fs::read_to_string(&path).await {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => Ok(Some(v)),
            Err(e) => {
                tracing::warn!("[wf_layout_get] Corrupt layout sidecar for {}: {}", id, e);
                Ok(None)
            }
        },
        Err(_) => Ok(None),
    }
}

/// 写入画布布局 sidecar（原子写：tmp + rename，对齐 store.save 落盘语义）
#[tauri::command]
pub async fn wf_layout_save(id: String, layout: serde_json::Value) -> Result<(), String> {
    let path = canvas_layout_path(&id);
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&layout).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, &json)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 全局执行闸门查询（WorkflowPage / CanvasPage 锁定态唯一权威源）
///
/// locked = 已有 active workflow run 或 Agent busy（state.busy）。
/// reason: "workflow"（工作流执行中）| "agent"（Agent 跑代码/跑任务）| "idle"。
#[tauri::command]
pub async fn wf_gate_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let engine = state.workflow_engine.read().await;
    let active = engine.active_run_info();
    let busy = state.busy.load(std::sync::atomic::Ordering::SeqCst);
    let (locked, reason) = if active.is_some() {
        (true, "workflow")
    } else if busy {
        (true, "agent")
    } else {
        (false, "idle")
    };
    Ok(serde_json::json!({
        "locked": locked,
        "reason": reason,
        "owner": active.as_ref().map(|a| a.owner.as_str()),
        "workflow_id": active.as_ref().map(|a| a.workflow_id.as_str()),
    }))
}
