//! nuphus-index — 知识库统一索引引擎。
//!
//! 零新增外部依赖，纯内存 HashMap + JSON 持久化。
//! 仅索引 plugin/knowledge/ 下的 .md 文件。
//!
//! # 用法
//!
//! ```rust,ignore
//! use nuphus_index::{IndexEngine, IndexConfig, QueryRequest};
//!
//! let engine = IndexEngine::new(IndexConfig {
//!     docs_root: "./plugin/knowledge".to_string(),
//!     index_path: "./.nuphus/index/knowledge_index.json".to_string(),
//!     ..Default::default()
//! });
//!
//! // 搜索
//! let results = engine.search(&QueryRequest {
//!     query: "PowerShell".to_string(),
//!     max_results: 5,
//!     ..Default::default()
//! });
//! ```

mod index;
mod parser;
mod query;
mod scanner;

// 重新导出
pub use index::IndexEngine;
pub use types::{FileMeta, IndexConfig, KnowledgeHit, QueryRequest};

mod types;
