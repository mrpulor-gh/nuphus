//! 工具定义分类模块
//!
//! 按功能类别拆分 registry.rs 中的 ToolDef 注册方法。

/// system_shell 的取消标志判定（供流式 shell 路径复用，见 `system::is_cancelled`）
pub use system::is_cancelled;

pub mod annotation;
pub mod file;
pub mod knowledge;
pub mod memory;
pub mod process;
pub mod schedule;
pub mod skill;
pub mod system;
