//! approval — 审批相关 Tauri 命令
//!
//! approve_pending:  用户批准 → 从 PendingApprovalStore 移除
//! reject_pending:   用户拒绝 → 从 PendingApprovalStore 移除
//! get_pending_details: 获取待审批项详情

use nuphus::memory::{EnforceLevel, Tenet, TenetPriority, TenetSource};
use nuphus::security::approval;
use tauri::State;

/// 批准待审批项
#[tauri::command]
pub async fn approve_pending(
    state: State<'_, crate::state::AppState>,
    action_id: String,
) -> Result<String, String> {
    let pending = approval::get(&state.signals, &action_id)
        .ok_or_else(|| format!("待审批项不存在或已过期: {}", action_id))?;

    let priority = pending
        .metadata
        .get("priority")
        .and_then(|v| v.as_str())
        .and_then(|s| match s {
            "critical" => Some(TenetPriority::Critical),
            "high" => Some(TenetPriority::High),
            "medium" => Some(TenetPriority::Medium),
            "low" => Some(TenetPriority::Low),
            _ => None,
        })
        .unwrap_or(TenetPriority::Medium);

    let tenet = Tenet {
        id: String::new(),
        content: pending.content.clone(),
        source: TenetSource::UserManual,
        priority,
        enforce_level: EnforceLevel::Suggestion,
        immutable: true,
        created_at: String::new(),
        updated_at: String::new(),
        active: true,
    };

    nuphus::memory::TenetStore::new()
        .add(tenet)
        .map_err(|e| format!("写入 TenetStore 失败: {}", e))?;

    approval::remove(&state.signals, &action_id)
        .ok_or_else(|| format!("待审批项不存在或已过期: {}", action_id))?;

    tracing::info!(
        "[APPROVAL] approved: title={}, kind={}",
        pending.title,
        pending.kind
    );
    Ok("approved".to_string())
}

/// 拒绝待审批项
#[tauri::command]
pub async fn reject_pending(
    state: State<'_, crate::state::AppState>,
    action_id: String,
) -> Result<String, String> {
    approval::remove(&state.signals, &action_id)
        .ok_or_else(|| format!("待审批项不存在或已过期: {}", action_id))?;

    tracing::info!("[APPROVAL] rejected: {}", action_id);
    Ok("rejected".to_string())
}

/// 获取待审批项详情
#[tauri::command]
pub async fn get_pending_details(
    state: State<'_, crate::state::AppState>,
    action_id: String,
) -> Result<serde_json::Value, String> {
    let pending = approval::get(&state.signals, &action_id)
        .ok_or_else(|| format!("待审批项不存在或已过期: {}", action_id))?;

    Ok(serde_json::json!({
        "action_id": pending.action_id,
        "title": pending.title,
        "content": pending.content,
        "kind": pending.kind,
        "metadata": pending.metadata,
    }))
}
