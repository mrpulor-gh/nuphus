//! Read-only AI proposal command: uses the current workflow model binding.
use crate::state::AppState;
use nuphus::workflow::scoped_edit::{ScopedEditProposal, ScopedEditRequest};
use tauri::State;

#[tauri::command]
pub async fn wf_propose_scoped_edit(
    state: State<'_, AppState>,
    request: ScopedEditRequest,
) -> Result<ScopedEditProposal, String> {
    let factory = nuphus::llm::ClientFactory::live();
    let registry = factory.registry().map_err(|error| error.to_string())?;
    let (provider, model) = crate::commands::config::llm::effective_model_binding(
        &state.llm_config_path,
        &registry,
        "workflow",
    )?;
    let client = factory
        .create_client_for(&provider, &model)
        .map_err(|error| error.to_string())?;
    let schemas = state.tools.get_schemas();
    nuphus::workflow::scoped_edit::propose(&request, &schemas, client.as_ref()).await
}
