//! Editor debug and execution evidence. Uses the same runtime and automation gate as wf_run.
use crate::state::AppState;
use nuphus::workflow::{
    debug::{DebugRequest, CURRENT},
    trace::{InvocationTrace, RunTrace},
};
use tauri::State;

#[tauri::command]
pub async fn wf_debug_run(
    state: State<'_, AppState>,
    request: DebugRequest,
) -> Result<serde_json::Value, String> {
    {
        let engine = state.workflow_engine.read().await;
        if super::workflow::workflow_execution_locked(&state, &engine).is_some() {
            return Err("当前有任务执行中，暂不可用！".into());
        }
    }
    let (gate_lease, execution_owner) = crate::resource_gate::acquire_execution_body_with_owner(
        &state.automation_gate,
        "wf_debug_run",
    )?;
    {
        let mut engine = state.workflow_engine.write().await;
        super::workflow::inject_workflow_runtime(&state, &mut engine);
    }
    let session = state
        .workflow_engine
        .read()
        .await
        .prepare_debug(request)
        .await?;
    let run_id = session.run_id.clone();
    let engine = state.workflow_engine.clone();
    let tools = state.tools.clone();
    tauri::async_runtime::spawn(async move {
        nuphus::automation_gate::with_execution_owner(execution_owner, async move {
            let _lease = gate_lease;
            let engine = engine.read().await;
            let tool_exec = move |tool: String, params: serde_json::Value| {
                let tools = tools.clone();
                async move {
                    let result = if tool.starts_with("browser_") {
                        tools.execute_browser_tool(&tool, &params).await
                    } else {
                        tools.execute(&tool, &params).await
                    }
                    .map_err(|e| e.to_string())?;
                    result.into_exec_result()
                }
            };
            let result = CURRENT
                .scope(
                    session.clone(),
                    engine.execute_workflow(
                        &session.workflow_id,
                        tool_exec,
                        engine.tools().map(|tools| tools.get_schemas()),
                        None,
                        Some(session.runtime_inputs.clone()),
                        true,
                        nuphus::workflow::WorkflowRunSource::Ui,
                    ),
                )
                .await;
            // Also closes the record if validation/gate failed before execute_v2 started.
            let cancelled = session.cancelled.load(std::sync::atomic::Ordering::Relaxed);
            if let Err(error) = &result {
                session.recorder.error(error.to_string()).await;
            }
            session
                .recorder
                .complete(if cancelled {
                    "cancelled"
                } else if result.is_ok() {
                    "success"
                } else {
                    "error"
                })
                .await;
            engine.debug_sessions.write().await.remove(&session.run_id);
        })
        .await;
    });
    Ok(serde_json::json!({"run_id":run_id}))
}

#[tauri::command]
pub async fn wf_debug_control(
    state: State<'_, AppState>,
    workflow_id: String,
    run_id: String,
    action: String,
) -> Result<(), String> {
    state
        .workflow_engine
        .read()
        .await
        .debug_control(&workflow_id, &run_id, &action)
        .await
}

#[tauri::command]
pub async fn wf_trace_list(
    state: State<'_, AppState>,
    workflow_id: String,
    debug: bool,
) -> Result<Vec<RunTrace>, String> {
    let root = state
        .workflow_engine
        .read()
        .await
        .store
        .root()
        .to_path_buf();
    nuphus::workflow::trace::list(&root, &workflow_id, debug).await
}

#[tauri::command]
pub async fn wf_trace_read(
    state: State<'_, AppState>,
    workflow_id: String,
    run_id: String,
    debug: bool,
    invocation_id: u64,
) -> Result<InvocationTrace, String> {
    let root = state
        .workflow_engine
        .read()
        .await
        .store
        .root()
        .to_path_buf();
    nuphus::workflow::trace::read(&root, &workflow_id, &run_id, debug, invocation_id).await
}
