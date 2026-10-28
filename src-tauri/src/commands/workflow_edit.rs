//! Read-only AI proposal command: uses the current workflow model binding.
use crate::state::AppState;
use nuphus::workflow::scoped_edit::{ScopedEditProposal, ScopedEditRequest};
use tauri::{Emitter, State, WebviewWindow};

#[tauri::command]
pub async fn wf_propose_scoped_edit(
    window: WebviewWindow,
    state: State<'_, AppState>,
    request: ScopedEditRequest,
) -> Result<ScopedEditProposal, String> {
    let factory = nuphus::llm::ClientFactory::live();
    let registry = factory.registry().map_err(|error| error.to_string())?;
    // 绑定口径（2026-10-28）：workflow 优先自身声明；解析失败回落 leader
    // （与 process 主链同一口径），再交给统一构建路径处置。
    let (provider, model) = crate::commands::config::llm::effective_model_binding(
        &state.llm_config_path,
        &registry,
        "workflow",
    )
    .or_else(|e| {
        tracing::warn!("[workflow-edit] workflow 绑定不可用（{e}），回落 leader");
        crate::commands::config::llm::effective_model_binding(
            &state.llm_config_path,
            &registry,
            "leader",
        )
    })?;
    let client = factory
        .create_client_for(&provider, &model)
        .map_err(|error| error.to_string())
        .or_else(|e: String| {
            tracing::warn!("[workflow-edit] 客户端构建失败（{e}），回落 leader");
            crate::commands::config::llm::effective_model_binding(
                &state.llm_config_path,
                &registry,
                "leader",
            )
            .and_then(|(p, m)| factory.create_client_for(&p, &m).map_err(|e| e.to_string()))
        })?;
    let schemas = state.tools.get_schemas();
    nuphus::workflow::scoped_edit::propose_with_progress(
        &request,
        &schemas,
        client.as_ref(),
        || {
            if let Some(id) = &request.request_id {
                let _ = window.emit(
                    "workflow-edit-progress",
                    serde_json::json!({
                        "request_id": id, "phase": "correcting"
                    }),
                );
            }
        },
    )
    .await
}
