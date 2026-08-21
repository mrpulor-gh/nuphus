//! nuphus-index 类型定义
//!
//! 精简后的知识库索引类型：只索引 plugin/knowledge/ 下的 .md 文件。

use serde::{Deserialize, Serialize};

// ════════════════════════════════════════════════════════════════════
// 文件元数据
// ════════════════════════════════════════════════════════════════════

/// 单个知识文档的索引元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMeta {
    /// 相对 plugin/knowledge 的路径（如 "powershell/基础操作.md"）
    pub rel_path: String,
    /// 标题（frontmatter title 或文件名）
    pub title: String,
    /// 标签列表（frontmatter tags，可选）
    pub tags: Vec<String>,
    /// 文件最后修改时间（UNIX 秒）
    pub file_mtime: u64,
    /// 文件大小（字节）
    pub file_size: u64,
    /// 正文全文（去除 frontmatter 区域）
    pub body_text: String,
    /// 语义向量（512-dim，由上层计算注入，不持久化）
    #[serde(skip)]
    pub embedding: Option<Vec<f32>>,
}

// ════════════════════════════════════════════════════════════════════
// 查询
// ════════════════════════════════════════════════════════════════════

/// 查询请求
#[derive(Debug, Clone)]
pub struct QueryRequest {
    /// 搜索关键词
    pub query: String,
    /// 按标签过滤
    pub tags: Vec<String>,
    /// 最大返回数
    pub max_results: usize,
}

impl Default for QueryRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            tags: Vec::new(),
            max_results: 10,
        }
    }
}

/// 查询命中条目（返回给调用方的精简信息）
#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeHit {
    /// 相对路径
    pub rel_path: String,
    /// 文档标题
    pub title: String,
    /// 标签列表
    pub tags: Vec<String>,
    /// 正文摘要（前 200 字，用于 UI 预览）
    pub snippet: String,
    /// 文件修改时间
    pub file_mtime: u64,
    /// 相关性分数
    pub score: f32,
}

impl From<&FileMeta> for KnowledgeHit {
    fn from(f: &FileMeta) -> Self {
        let snippet = f
            .body_text
            .chars()
            .take(200)
            .collect::<String>()
            .trim()
            .to_string();
        Self {
            rel_path: f.rel_path.clone(),
            title: f.title.clone(),
            tags: f.tags.clone(),
            snippet,
            file_mtime: f.file_mtime,
            score: 0.0,
        }
    }
}

// ════════════════════════════════════════════════════════════════════
// 配置
// ════════════════════════════════════════════════════════════════════

/// 索引引擎配置
#[derive(Debug, Clone, Default)]
pub struct IndexConfig {
    /// plugin/knowledge 目录的绝对路径
    pub docs_root: String,
    /// 索引持久化文件路径
    pub index_path: String,
}
