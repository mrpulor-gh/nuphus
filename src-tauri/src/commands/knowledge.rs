//! 知识库管理命令
//!
//! 提供 search_knowledge / list_knowledge / delete_knowledge 三个 Tauri 命令，
//! 与 Agent 端 knowledge_search 工具共享同一索引引擎实例。

use nuphus_index::{IndexConfig, IndexEngine, KnowledgeHit, QueryRequest};
use tauri::State;

use crate::state::AppState;

// ── 索引引擎初始化 ──

/// 获取或初始化知识库索引引擎（lazy init）
fn ensure_knowledge_engine(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, crate::state::ExecutionState>, String> {
    let mut guard = state
        .execution
        .lock()
        .map_err(|e| format!("锁异常: {}", e))?;
    if guard.knowledge_engine.is_none() {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let docs_root = find_plugin_knowledge(&exe_dir)
            .or_else(|| find_plugin_knowledge(&std::env::current_dir().unwrap_or_default()))
            .unwrap_or_else(|| {
                let path = std::env::current_dir()
                    .unwrap_or_default()
                    .join("plugin")
                    .join("knowledge");
                if path.exists() {
                    path
                } else {
                    exe_dir.join("plugin").join("knowledge")
                }
            });

        let index_dir = docs_root
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join(".nuphus").join("index"))
            .unwrap_or_else(|| {
                let mut p = std::env::current_dir().unwrap_or_default();
                p.push(".nuphus");
                p.push("index");
                p
            });

        let index_path = index_dir.join("knowledge_index.json");

        tracing::info!(
            "[knowledge] Initializing IndexEngine: docs_root={:?}, index_path={:?}",
            docs_root,
            index_path
        );

        guard.knowledge_engine = Some(IndexEngine::new(IndexConfig {
            docs_root: docs_root.to_string_lossy().to_string(),
            index_path: index_path.to_string_lossy().to_string(),
        }));
    }
    Ok(guard)
}

fn find_plugin_knowledge(start: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        let candidate = dir.join("plugin").join("knowledge");
        if candidate.exists() {
            return Some(candidate);
        }
        current = dir.parent();
    }
    None
}

// ── Tauri 命令 ──

#[tauri::command]
pub fn search_knowledge(
    state: State<'_, AppState>,
    query: String,
    tags: Option<Vec<String>>,
    max_results: Option<usize>,
) -> Result<Vec<KnowledgeHit>, String> {
    let engine_guard = ensure_knowledge_engine(state.inner())?;
    let engine = engine_guard
        .knowledge_engine
        .as_ref()
        .ok_or_else(|| "知识引擎未初始化".to_string())?;
    // 搜索前增量扫描，感知新增/修改文件
    engine.rescan_modified();
    let max = max_results.unwrap_or(10);
    let req = QueryRequest {
        query,
        tags: tags.unwrap_or_default(),
        max_results: max,
    };
    Ok(engine.search(&req))
}

#[tauri::command]
pub fn list_knowledge(state: State<'_, AppState>) -> Result<Vec<KnowledgeHit>, String> {
    let engine_guard = ensure_knowledge_engine(state.inner())?;
    let engine = engine_guard
        .knowledge_engine
        .as_ref()
        .ok_or_else(|| "知识引擎未初始化".to_string())?;
    // 每次列表查询前增量扫描，确保新增文件可见
    engine.rescan_modified();
    let req = QueryRequest {
        query: String::new(),
        tags: Vec::new(),
        max_results: 9999,
    };
    Ok(engine.search(&req))
}

#[tauri::command]
pub fn list_knowledge_tags(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let engine_guard = ensure_knowledge_engine(state.inner())?;
    let engine = engine_guard
        .knowledge_engine
        .as_ref()
        .ok_or_else(|| "知识引擎未初始化".to_string())?;
    // 每次标签列表查询前增量扫描，确保新增文件的标签可见
    engine.rescan_modified();
    Ok(engine.all_tags())
}

#[tauri::command]
pub fn delete_knowledge(state: State<'_, AppState>, rel_path: String) -> Result<bool, String> {
    let engine_guard = ensure_knowledge_engine(state.inner())?;
    let engine = engine_guard
        .knowledge_engine
        .as_ref()
        .ok_or_else(|| "知识引擎未初始化".to_string())?;
    let docs_root = std::path::PathBuf::from(&engine.config().docs_root);
    let abs_path = docs_root.join(&rel_path);
    let canonical = abs_path
        .canonicalize()
        .map_err(|e| format!("无法解析路径: {}", e))?;
    let root_canonical = docs_root
        .canonicalize()
        .map_err(|e| format!("无法解析根目录: {}", e))?;
    if !canonical.starts_with(&root_canonical) {
        return Err("路径越权拒绝".to_string());
    }
    std::fs::remove_file(&abs_path).map_err(|e| format!("删除失败: {}", e))?;
    engine.rescan();
    Ok(true)
}
