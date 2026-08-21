//! Session management
//!
//! Reference session.rs design, simplified implementation
//! Provides structured message session management
//!
//! ## 文件拆分
//!
//! - `types.rs` — 核心类型：MessageRole, ContentBlock, Message, RefineStrategy, TokenUsage
//! - `session.rs` — Session 结构体 + 生命周期（创建/推送/查询/归档）
//! - `transform.rs` — API 格式转换 + 工具对清理 + 提炼标记
//! - `image.rs` — 图片处理策略（三态能力矩阵）+ 描述缓存

pub mod image;
pub mod session;
pub mod transform;
pub mod types;

#[cfg(test)]
mod tests;

// ── 向后兼容导出 ──
pub use session::*;
pub use types::*;
