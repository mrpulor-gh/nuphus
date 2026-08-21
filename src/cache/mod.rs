//! 持久化只读工具结果缓存
//!
//! 在 ExecuteAgent 的 tool_cache（单次循环内存）基础上，
//! 提供跨会话的持久化缓存，配合 mtime 校验保证一致性。
//!
//! # 缓存策略
//!
//! | 工具类型 | 校验方式 | 说明 |
//! |----------|----------|------|
//! | file::read / file::stat / file::list_dir | mtime + size | 先 stat 再返回，文件变了自动失效 |
//! | search::glob / search::grep | mtime（目录） | 目录 mtime 变了重新搜索 |
//! | web_extract / web_search | TTL 60s | 短时缓存，过时自动失效 |
//! | planner::* / memory::* | 无校验 | 仅 LRU 淘汰，会话级一致性由调用方保证 |
//!
//! 写入时失效：file::write / file::edit 后调用 `invalidate(path)` 清除相关缓存。

pub mod tool_cache;

pub use tool_cache::global_cache;
pub use tool_cache::ToolCache;
