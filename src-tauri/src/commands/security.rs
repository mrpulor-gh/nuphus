use crate::state::AppState;
use nuphus::permissions::ToolPermissions;
use tauri::State;

/// 批准一次
#[tauri::command]
pub async fn approve_once_security(
    state: State<'_, AppState>,
    action_id: String,
) -> Result<String, String> {
    nuphus::security::set_security_result(&state.signals, &action_id, true);
    let mut pending = state.execution.lock().map_err(|e| e.to_string())?;
    if let Some(entry) = pending.pending_security.get_mut(&action_id) {
        entry.approved = Some(true);
    }
    tracing::info!("[SECURITY] Approve once: {}", action_id);
    Ok("approved".to_string())
}

/// 对话级授权：批准本次 + 此工具此对话不再弹窗
#[tauri::command]
pub async fn approve_session_security(
    state: State<'_, AppState>,
    action_id: String,
    tool: String,
) -> Result<String, String> {
    nuphus::security::approve_session_tool(&state.signals, &tool);
    nuphus::security::set_security_result(&state.signals, &action_id, true);
    let mut pending = state.execution.lock().map_err(|e| e.to_string())?;
    if let Some(entry) = pending.pending_security.get_mut(&action_id) {
        entry.approved = Some(true);
    }
    tracing::info!(
        "[SECURITY] Approve session: tool={}, action={}",
        tool,
        action_id
    );
    Ok("session_approved".to_string())
}

#[tauri::command]
pub async fn reject_security(
    state: State<'_, AppState>,
    action_id: String,
) -> Result<String, String> {
    nuphus::security::set_security_result(&state.signals, &action_id, false);
    let mut pending = state.execution.lock().map_err(|e| e.to_string())?;
    if let Some(entry) = pending.pending_security.get_mut(&action_id) {
        entry.approved = Some(false);
    }
    tracing::info!("[SECURITY] Rejected action {}", action_id);
    Ok("rejected".to_string())
}

#[tauri::command]
pub fn set_tool_permissions(
    state: State<'_, AppState>,
    file_access: bool,
    web_search: bool,
    system_automation: bool,
) -> Result<String, String> {
    let perms = ToolPermissions {
        file_access,
        web_search,
        system_automation,
    };

    // 先持久化到磁盘（成功才更新内存，避免状态不一致）
    let path = &state.tool_permissions_path;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(&perms).unwrap_or_default();
    std::fs::write(path, &json).map_err(|e| format!("保存权限设置失败: {}", e))?;

    // 磁盘写入成功后才更新内存
    {
        let mut current = state.runtime.lock().map_err(|e| e.to_string())?;
        current.tool_permissions = perms;
    }
    // Sync shared reference for Runtime real-time policy updates
    if let Ok(mut shared) = state.tool_permissions_ref.lock() {
        *shared = perms;
    }
    tracing::info!(
        "[PERM] Tool permissions set to: file={}, web={}, system={}",
        file_access,
        web_search,
        system_automation
    );
    Ok(json)
}
