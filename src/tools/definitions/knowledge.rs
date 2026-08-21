//! 知识库工具定义
//! knowledge_search — 搜索 plugin/knowledge/ 下的用户知识文档

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;
use nuphus_index::{IndexConfig, IndexEngine, QueryRequest};
use std::sync::Mutex;

/// 全局 IndexEngine 单例（与 Tauri 端共享同一索引文件，避免每次重建）
static KNOWLEDGE_ENGINE: Mutex<Option<IndexEngine>> = Mutex::new(None);

fn get_or_init_engine() -> Result<std::sync::MutexGuard<'static, Option<IndexEngine>>, String> {
    let mut guard = KNOWLEDGE_ENGINE
        .lock()
        .map_err(|e| format!("锁异常: {}", e))?;
    if guard.is_none() {
        let docs_root = find_plugin_knowledge().ok_or_else(|| {
            "知识库目录未找到: plugin/knowledge/。请确认知识库目录存在。".to_string()
        })?;

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

        if let Err(e) = std::fs::create_dir_all(&index_dir) {
            tracing::warn!("[knowledge_search] 无法创建索引目录 {:?}: {}", index_dir, e);
        }

        *guard = Some(IndexEngine::new(IndexConfig {
            docs_root: docs_root.to_string_lossy().to_string(),
            index_path: index_path.to_string_lossy().to_string(),
        }));
    }
    Ok(guard)
}

impl ToolRegistry {
    /// knowledge_search — 搜索知识库文档，返回标题、摘要、路径（供后续 Read 深入）
    pub(crate) fn register_knowledge_search(&mut self) {
        self.register(ToolDef {
            name: "knowledge_search".to_string(),
            description: "搜索知识库文档（plugin/knowledge/ 下的 .md 文件）。返回标题、摘要、路径，不返回全文 — 用 Read 工具读取具体文档。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "搜索关键词或问题描述" },
                    "max_results": { "type": "integer", "description": "最大返回数（默认 5）" }
                },
                "required": ["query"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let max_results = params.get("max_results")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5) as usize;

                let engine_guard = get_or_init_engine()?;
                let engine = engine_guard
                    .as_ref()
                    .ok_or_else(|| "知识库引擎初始化失败".to_string())?;
                // 增量同步索引（感知前端删除/新增/修改的知识文件，
                // 避免与 Tauri 端引擎实例的状态分裂）
                engine.rescan_modified();
                let req = QueryRequest {
                    query: query.to_string(),
                    tags: vec![],
                    max_results,
                };
                let hits = engine.search(&req);
                let json = serde_json::to_string_pretty(&hits)
                    .map_err(|e| format!("序列化搜索结果失败: {}", e))?;

                if hits.is_empty() {
                    Ok(ToolResult::success(format!(
                        "知识库中未找到与「{}」相关的文档。\n可用知识库目录: plugin/knowledge/",
                        query
                    )))
                } else {
                    Ok(ToolResult::success(json))
                }
            },
            depends_on: vec![],
        });
    }
}

/// 查找 plugin/knowledge 目录（绝对路径）
fn find_plugin_knowledge() -> Option<std::path::PathBuf> {
    let candidates: Vec<std::path::PathBuf> = vec![
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf())),
        std::env::current_dir().ok(),
    ]
    .into_iter()
    .flatten()
    .collect();

    for base in &candidates {
        let path = base.join("plugin").join("knowledge");
        if path.exists() {
            tracing::debug!("[knowledge_search] 找到知识库: {:?}", path);
            return Some(path);
        }
    }

    // 兜底：以 current_dir 为准（即使不存在也返回，让 IndexEngine 报友好错误）
    let fallback = std::env::current_dir()
        .unwrap_or_default()
        .join("plugin")
        .join("knowledge");
    if fallback.exists() {
        Some(fallback)
    } else {
        tracing::warn!("[knowledge_search] 知识库目录不存在: {:?}", fallback);
        None
    }
}
