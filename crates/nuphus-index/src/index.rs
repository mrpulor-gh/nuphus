//! IndexEngine — 知识库索引引擎。
//!
//! 扫描 plugin/knowledge/ → 解析 frontmatter + 正文 → 内存 HashMap → JSON 持久化。
//! 使用 JSON 文件持久化，零外部依赖（serde_json 已有）。

use crate::query;
use crate::scanner;
use crate::types::{FileMeta, IndexConfig, KnowledgeHit, QueryRequest};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// 索引引擎
pub struct IndexEngine {
    config: IndexConfig,
    /// 文件索引（rel_path → FileMeta）
    data: RwLock<HashMap<String, FileMeta>>,
    /// 标签索引（tag → [rel_path, ...]）
    tag_index: RwLock<HashMap<String, Vec<String>>>,
    /// 索引持久化路径
    index_path: PathBuf,
}

impl IndexEngine {
    /// 创建并初始化索引引擎。
    ///
    /// 启动流程：
    /// 1. 如果存在持久化索引 → 加载
    /// 2. 否则全量扫描 plugin/knowledge/
    pub fn new(config: IndexConfig) -> Self {
        let docs_root = PathBuf::from(&config.docs_root);
        let index_path = PathBuf::from(&config.index_path);

        // 尝试从持久化加载
        let (files, tags) = Self::load_persisted(&index_path, &docs_root);

        let (files, tags) = if files.is_empty() {
            tracing::info!("[nuphus-index] Performing full scan of {:?}...", docs_root);
            let result = scanner::scan_directory(&docs_root);
            Self::persist(&index_path, &result.files, &result.tags);
            (result.files, result.tags)
        } else {
            // 启动时清理：移除磁盘上已不存在的文件条目
            let stale_count = files
                .keys()
                .filter(|rel_path| !docs_root.join(rel_path).exists())
                .count();
            if stale_count > 0 {
                let files: HashMap<String, FileMeta> = files
                    .into_iter()
                    .filter(|(rel_path, _)| docs_root.join(rel_path).exists())
                    .collect();
                // 从剩余文件重建标签索引
                let mut tags: HashMap<String, Vec<String>> = HashMap::new();
                for (rel_path, meta) in &files {
                    for tag in &meta.tags {
                        tags.entry(tag.clone()).or_default().push(rel_path.clone());
                    }
                }
                tracing::info!(
                    "[nuphus-index] Startup cleanup: removed {} stale entries, {} remain",
                    stale_count,
                    files.len()
                );
                Self::persist(&index_path, &files, &tags);
                (files, tags)
            } else {
                (files, tags)
            }
        };

        Self {
            config,
            data: RwLock::new(files),
            tag_index: RwLock::new(tags),
            index_path,
        }
    }

    /// 搜索知识文档
    pub fn search(&self, req: &QueryRequest) -> Vec<KnowledgeHit> {
        let files = self.data.read().expect("index lock poisoned");
        query::search_index(&files, req)
    }

    /// 获取所有标签
    pub fn all_tags(&self) -> Vec<String> {
        let tag_index = self.tag_index.read().expect("index lock poisoned");
        let mut tags: Vec<String> = tag_index.keys().cloned().collect();
        tags.sort();
        tags
    }

    /// 获取配置
    pub fn config(&self) -> &IndexConfig {
        &self.config
    }

    /// 强制重新扫描（感知文件变更）
    pub fn rescan(&self) {
        let docs_root = PathBuf::from(&self.config.docs_root);
        tracing::info!("[nuphus-index] Rescanning {:?}...", docs_root);

        let result = scanner::scan_directory(&docs_root);

        let mut files = self.data.write().expect("index lock poisoned");
        let mut tags = self.tag_index.write().expect("index lock poisoned");
        *files = result.files;
        *tags = result.tags;

        Self::persist(&self.index_path, &files, &tags);
        tracing::info!("[nuphus-index] Rescan complete: {} files", files.len());
    }

    /// 增量扫描（仅扫描变更的文件）
    pub fn rescan_modified(&self) {
        let docs_root = PathBuf::from(&self.config.docs_root);
        let known = self.data.read().expect("index lock poisoned").clone();

        let result = scanner::scan_modified(&docs_root, &known);

        let mut files = self.data.write().expect("index lock poisoned");
        let mut tags = self.tag_index.write().expect("index lock poisoned");
        *files = result.files;
        *tags = result.tags;

        Self::persist(&self.index_path, &files, &tags);
    }

    // ── 持久化 ──

    fn load_persisted(
        index_path: &Path,
        _docs_root: &Path,
    ) -> (HashMap<String, FileMeta>, HashMap<String, Vec<String>>) {
        if !index_path.exists() {
            return (HashMap::new(), HashMap::new());
        }

        let content = match fs::read_to_string(index_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("[nuphus-index] Failed to read index: {}", e);
                return (HashMap::new(), HashMap::new());
            }
        };

        let index: PersistentIndex = match serde_json::from_str(&content) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!("[nuphus-index] Failed to parse index: {}", e);
                return (HashMap::new(), HashMap::new());
            }
        };

        tracing::info!(
            "[nuphus-index] Loaded persisted index: {} files (version {})",
            index.files.len(),
            index.version
        );
        (index.files, index.tags)
    }

    fn persist(
        index_path: &Path,
        files: &HashMap<String, FileMeta>,
        tags: &HashMap<String, Vec<String>>,
    ) {
        if let Some(parent) = index_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let index = PersistentIndex {
            version: 1,
            files: files.clone(),
            tags: tags.clone(),
            indexed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };

        match serde_json::to_string(&index) {
            Ok(json) => {
                if let Err(e) = fs::write(index_path, &json) {
                    tracing::error!("[nuphus-index] Failed to write index: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("[nuphus-index] Failed to serialize index: {}", e);
            }
        }
    }
}

/// 持久化索引格式
#[derive(Serialize, Deserialize)]
struct PersistentIndex {
    version: u32,
    files: HashMap<String, FileMeta>,
    tags: HashMap<String, Vec<String>>,
    indexed_at: u64,
}
