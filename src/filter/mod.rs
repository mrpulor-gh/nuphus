//! 工具输出过滤器 — 在工具结果进入 Session 前进行轻量预处理，从源头减少 token 消耗。

pub mod filter;
pub mod rule;

pub use filter::ToolOutputFilter;
