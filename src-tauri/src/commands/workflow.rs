//! workflow.rs — Tauri commands for workflow management
//!
//! 精简后的命令列表：保留 CRUD + 规划 + 执行。
//! 画布命令（wf_validate / wf_save / wf_run）：IR 唯一真源，保存前强制权威校验。

use nuphus::workflow::compiler::{Compiler, ValidationReport};
use nuphus::workflow::scheduler::{has_frontend_step, ScheduleRunRecord, SchedulerEngine};
use nuphus::workflow::types::{InputSpec, ScheduleConfig, Workflow};
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

#[derive(Debug, Serialize)]
pub struct WfScheduleDetails {
    pub config: Option<ScheduleConfig>,
    pub inputs: std::collections::HashMap<String, serde_json::Value>,
    pub sensitive_inputs: Vec<String>,
    pub eligible: bool,
    pub ineligible_reason: Option<String>,
}

fn sanitized_schedule_inputs(
    specs: &[InputSpec],
    decoded: &std::collections::HashMap<String, serde_json::Value>,
) -> (
    std::collections::HashMap<String, serde_json::Value>,
    Vec<String>,
) {
    let sensitive_names: std::collections::HashSet<&str> = specs
        .iter()
        .filter(|spec| spec.sensitive)
        .map(|spec| spec.name.as_str())
        .collect();
    let inputs = decoded
        .iter()
        .filter(|(name, _)| !sensitive_names.contains(name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let mut sensitive_inputs: Vec<String> = decoded
        .keys()
        .filter(|name| sensitive_names.contains(name.as_str()))
        .cloned()
        .collect();
    sensitive_inputs.sort();
    (inputs, sensitive_inputs)
}

fn merge_preserved_sensitive(
    specs: &[InputSpec],
    previous: &std::collections::HashMap<String, serde_json::Value>,
    explicit: &mut std::collections::HashMap<String, serde_json::Value>,
    preserve: &[String],
) -> Result<(), String> {
    let sensitive_names: std::collections::HashSet<&str> = specs
        .iter()
        .filter(|spec| spec.sensitive)
        .map(|spec| spec.name.as_str())
        .collect();
    for name in preserve {
        if !sensitive_names.contains(name.as_str()) {
            return Err("preserve_sensitive 只能包含已声明的敏感输入".to_string());
        }
        if explicit.contains_key(name) {
            continue;
        }
        let value = previous
            .get(name)
            .ok_or_else(|| format!("敏感调度输入 '{name}' 没有可保留的旧值"))?;
        explicit.insert(name.clone(), value.clone());
    }
    Ok(())
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

// ── 定时调度编辑 ──

#[tauri::command]
pub async fn wf_schedule_get(
    state: State<'_, AppState>,
    id: String,
) -> Result<WfScheduleDetails, String> {
    let engine = state.workflow_engine.read().await;
    let workflow = engine
        .store
        .get(&id)
        .await
        .ok_or_else(|| format!("Workflow not found: {id}"))?;
    let decoded = engine
        .scheduler
        .get_decoded_inputs(&id, &workflow.inputs)
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let (inputs, sensitive_inputs) = sanitized_schedule_inputs(&workflow.inputs, &decoded);
    let ineligible_reason = has_frontend_step(&workflow.steps)
        .then(|| "包含桌面或浏览器前台步骤，需要用户在场，不能定时执行".to_string());
    let config = engine
        .scheduler
        .get_schedule(&id)
        .await
        .or(workflow.schedule);
    Ok(WfScheduleDetails {
        config,
        inputs,
        sensitive_inputs,
        eligible: ineligible_reason.is_none(),
        ineligible_reason,
    })
}

#[tauri::command]
pub fn wf_schedule_preview(config: ScheduleConfig) -> Result<Vec<String>, String> {
    SchedulerEngine::preview(&config, 3)
        .map(|dates| dates.into_iter().map(|date| date.to_rfc3339()).collect())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn wf_schedule_set(
    state: State<'_, AppState>,
    id: String,
    config: ScheduleConfig,
    inputs: serde_json::Value,
    preserve_sensitive: Vec<String>,
) -> Result<(), String> {
    let mut explicit: std::collections::HashMap<String, serde_json::Value> = inputs
        .as_object()
        .ok_or_else(|| "inputs 必须是 JSON 对象".to_string())?
        .clone()
        .into_iter()
        .collect();
    let engine = state.workflow_engine.read().await;
    let workflow = engine
        .store
        .get(&id)
        .await
        .ok_or_else(|| format!("Workflow not found: {id}"))?;
    if !preserve_sensitive.is_empty() {
        let previous = engine
            .scheduler
            .get_decoded_inputs(&id, &workflow.inputs)
            .await
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        merge_preserved_sensitive(
            &workflow.inputs,
            &previous,
            &mut explicit,
            &preserve_sensitive,
        )?;
    }
    engine
        .set_schedule_with_inputs(&id, config, explicit)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn wf_schedule_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let engine = state.workflow_engine.read().await;
    if engine.store.get(&id).await.is_none() {
        return Err(format!("Workflow not found: {id}"));
    }
    engine.remove_schedule(&id).await;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct WfScheduleHistoryFilter {
    pub workflow_id: Option<String>,
    pub status: Option<String>,
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct WfScheduleHistoryPage {
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
    pub runs: Vec<ScheduleRunRecord>,
}

fn schedule_status_matches(run: &ScheduleRunRecord, expected: &str) -> bool {
    matches!(
        (expected, &run.status),
        ("running", nuphus::workflow::types::RunStatus::Running)
            | ("success", nuphus::workflow::types::RunStatus::Success)
            | ("cancelled", nuphus::workflow::types::RunStatus::Cancelled)
            | ("paused", nuphus::workflow::types::RunStatus::Paused)
            | ("error", nuphus::workflow::types::RunStatus::Error(_))
    )
}

#[tauri::command]
pub async fn wf_schedule_history_list(
    state: State<'_, AppState>,
    filter: Option<WfScheduleHistoryFilter>,
) -> Result<WfScheduleHistoryPage, String> {
    let filter = filter.unwrap_or(WfScheduleHistoryFilter {
        workflow_id: None,
        status: None,
        from: None,
        to: None,
        page: None,
        page_size: None,
    });
    let page = filter.page.unwrap_or(0);
    let page_size = filter.page_size.unwrap_or(50).clamp(1, 200);
    let engine = state.workflow_engine.read().await;
    let mut runs: Vec<_> = engine
        .scheduler
        .list_schedule_runs()
        .runs
        .into_iter()
        .filter(|run| {
            filter
                .workflow_id
                .as_deref()
                .map(|id| run.workflow_id == id)
                .unwrap_or(true)
        })
        .filter(|run| {
            filter
                .status
                .as_deref()
                .map(|status| schedule_status_matches(run, status))
                .unwrap_or(true)
        })
        .filter(|run| {
            filter
                .from
                .map(|date| run.started_at >= date)
                .unwrap_or(true)
        })
        .filter(|run| filter.to.map(|date| run.started_at <= date).unwrap_or(true))
        .collect();
    runs.sort_by_key(|run| std::cmp::Reverse(run.started_at));
    let total = runs.len();
    let start = page.saturating_mul(page_size).min(total);
    let end = (start + page_size).min(total);
    Ok(WfScheduleHistoryPage {
        total,
        page,
        page_size,
        runs: runs[start..end].to_vec(),
    })
}

#[tauri::command]
pub async fn wf_schedule_history_get(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<ScheduleRunRecord, String> {
    let engine = state.workflow_engine.read().await;
    engine
        .scheduler
        .list_schedule_runs()
        .runs
        .into_iter()
        .find(|run| run.run_id == run_id)
        .ok_or_else(|| format!("Schedule run not found: {run_id}"))
}

#[tauri::command]
pub async fn wf_schedule_history_delete(
    state: State<'_, AppState>,
    workflow_id: Option<String>,
    before: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<usize, String> {
    let engine = state.workflow_engine.read().await;
    engine
        .scheduler
        .delete_schedule_runs(workflow_id.as_deref(), before)
        .map_err(|error| error.to_string())
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
    engine.set_tools(std::sync::Arc::new(state.tools.clone()));
    // Both the workflow default client and per-step factory use the complete
    // provider+model binding. This avoids model-only lookup when names collide.
    if let Ok(full_registry) = nuphus::config::load_registry() {
        let factory = nuphus::llm::ClientFactory::new(full_registry);
        engine.set_client_factory(factory.clone());
        match crate::commands::config::llm::effective_model_binding(
            &state.llm_config_path,
            factory.registry(),
            "workflow",
        ) {
            Ok((provider, model)) => match factory.create_client_for(&provider, &model) {
                Ok(client) => engine.set_llm_client(client),
                Err(e) => tracing::warn!(
                    "[workflow] Failed to create bound client ({provider}:{model}): {e}"
                ),
            },
            Err(e) => tracing::warn!("[workflow] No workflow provider+model binding: {e}"),
        }
    }
}

/// 从画布确定性触发执行（同 id 自动断点续连，execute.rs 语义）。
/// 与 workflow_run 工具同一条 execute_workflow 链路；前台交互运行，
/// 不施加 scheduler 的 has_frontend_step 后台限制（desktop_/browser_ 工具本就面向桌面前台）。
/// 异步 spawn：立即返回，进度经 workflow-event 事件流推送。
/// fresh=true：上次运行失败后画布「运行」从头执行（force_fresh，跳过逻辑失效）；
/// 缺省/续跑（fresh=false）保留断点续连语义。
/// inputs：启动表单收集的外部输入（JSON 对象；缺省 None）；声明解析/必填校验在 execute 层。
#[tauri::command]
pub async fn wf_run(
    state: State<'_, AppState>,
    id: String,
    fresh: Option<bool>,
    inputs: Option<serde_json::Value>,
) -> Result<String, String> {
    let force_fresh = fresh.unwrap_or(false);
    // 契约：inputs 为 JSON 对象或 None（非对象直接拒绝，避免类型错误静默丢失）
    let inputs: std::collections::HashMap<String, serde_json::Value> = match inputs {
        None => std::collections::HashMap::new(),
        Some(serde_json::Value::Object(map)) => map.into_iter().collect(),
        Some(_) => return Err("inputs 必须是 JSON 对象".to_string()),
    };
    // 只记键名（不记值）：声明项的敏感度与必填校验由 execute 层依据 IR 判定
    for key in inputs.keys() {
        tracing::debug!("[wf_run] 收到输入键: {}", key);
    }
    // 在异步 spawn 前执行与 executor 相同的输入契约预检，让 UI 直接收到必填/类型错误。
    // 未声明键仍允许存在，executor 会按兼容语义只注入顶层。
    {
        let engine = state.workflow_engine.read().await;
        let workflow = engine
            .store
            .get(&id)
            .await
            .ok_or_else(|| format!("Workflow not found: {id}"))?;
        nuphus::workflow::inputs::resolve_declared_inputs(&workflow.inputs, &inputs)
            .map_err(|e| e.to_string())?;
    }
    let inputs = (!inputs.is_empty()).then_some(inputs);
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
                inputs,
                force_fresh,
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

#[cfg(test)]
mod schedule_editor_tests {
    use super::*;
    use nuphus::workflow::types::InputKind;

    fn input(name: &str, sensitive: bool) -> InputSpec {
        InputSpec {
            name: name.into(),
            kind: InputKind::String,
            required: false,
            default: None,
            description: None,
            sensitive,
        }
    }

    #[test]
    fn schedule_details_never_return_sensitive_values() {
        let specs = vec![input("topic", false), input("token", true)];
        let decoded = std::collections::HashMap::from([
            ("topic".into(), serde_json::json!("news")),
            ("token".into(), serde_json::json!("secret")),
        ]);
        let (visible, sensitive) = sanitized_schedule_inputs(&specs, &decoded);
        assert_eq!(
            visible,
            std::collections::HashMap::from([("topic".into(), serde_json::json!("news"))])
        );
        assert_eq!(sensitive, vec!["token"]);
    }

    #[test]
    fn preserved_sensitive_value_is_merged_without_overwriting_reentry() {
        let specs = vec![input("token", true)];
        let previous =
            std::collections::HashMap::from([("token".into(), serde_json::json!("old"))]);
        let mut explicit = std::collections::HashMap::new();
        merge_preserved_sensitive(&specs, &previous, &mut explicit, &["token".into()]).unwrap();
        assert_eq!(explicit["token"], serde_json::json!("old"));

        explicit.insert("token".into(), serde_json::json!("new"));
        merge_preserved_sensitive(&specs, &previous, &mut explicit, &["token".into()]).unwrap();
        assert_eq!(explicit["token"], serde_json::json!("new"));
    }
}
