//! Relation（身份关系）配置命令：称呼持久化到 relation.json，手机端 /identity 经 relation_cache 下发。
use crate::state::AppState;
use nuphus::agent::goal_types::RelationConfig;
use tauri::State;

/// 持久化身份配置并更新 relation_cache（桌面端 SoulPage 保存 / 启动迁移时调用）。
#[tauri::command]
pub fn set_relation(state: State<'_, AppState>, relation: RelationConfig) -> Result<(), String> {
    let config_dir = state
        .llm_config_path
        .parent()
        .ok_or_else(|| "配置目录解析失败".to_string())?;
    let path = config_dir.join("relation.json");
    let json = serde_json::to_string_pretty(&relation)
        .map_err(|e| format!("序列化 relation 失败: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("写入 relation.json 失败: {e}"))?;
    let mut cache = state.relation_cache.write().map_err(|e| e.to_string())?;
    *cache = Some(relation);
    tracing::info!("[relation] 身份配置已持久化并更新缓存");
    Ok(())
}
