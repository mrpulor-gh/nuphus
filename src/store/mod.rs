//! 持久化存储层
//!
//! 使用 SQLite (rusqlite) 替代 JSONL 文件存储，提供关系型查询和 FTS5 全文检索。
//!
//! 架构：
//! - `db` — 数据库连接管理（单例路径+连接池）
//! - `memory` — 记忆条目存储（FTS5 全文搜索）
//! - `session` — 会话记录存储

pub mod db;
pub mod memory;
pub mod session;
