//! tenet — 原则相关 Tauri 命令

use nuphus::memory::{EnforceLevel, Tenet, TenetPriority, TenetSource, TenetStore};
use serde_json::json;

/// 获取所有原则
#[tauri::command]
pub fn get_tenets() -> Result<serde_json::Value, String> {
    let store = TenetStore::new();
    let items: Vec<serde_json::Value> = store
        .active()
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "content": t.content,
                "priority": t.priority,
                "enforce": t.enforce_level,
                "active": t.active,
                "created_at": t.created_at,
            })
        })
        .collect();

    Ok(json!({
        "count": store.active_count(),
        "total": store.all().len(),
        "items": items,
    }))
}

/// 添加一条原则
#[tauri::command]
pub fn add_tenet(content: String, priority: Option<String>) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("原则内容不能为空".into());
    }
    let priority = priority
        .as_deref()
        .and_then(|s| match s {
            "critical" => Some(TenetPriority::Critical),
            "high" => Some(TenetPriority::High),
            "medium" => Some(TenetPriority::Medium),
            "low" => Some(TenetPriority::Low),
            _ => None,
        })
        .unwrap_or(TenetPriority::High);

    let now = chrono::Utc::now().to_rfc3339();
    let tenet = Tenet {
        id: uuid::Uuid::new_v4().to_string(),
        content: content.trim().to_string(),
        source: TenetSource::UserManual,
        priority,
        enforce_level: EnforceLevel::Warning,
        immutable: true,
        created_at: now.clone(),
        updated_at: now,
        active: true,
    };

    TenetStore::new()
        .add(tenet)
        .map_err(|e| format!("添加原则失败: {}", e))
}

/// 删除（软删除）一条原则
#[tauri::command]
pub fn delete_tenet(id: String) -> Result<bool, String> {
    let mut store = TenetStore::new();
    store
        .deactivate(&id)
        .map_err(|e| format!("删除原则失败: {}", e))
}
