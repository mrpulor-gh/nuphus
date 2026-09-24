//! Tools module - pluggable tool system

pub mod browser_tools;
pub mod builtin;
pub mod definitions;
pub mod desktop_approval;
pub mod desktop_executors;
pub mod desktop_schemas;
mod desktop_verification;
pub mod registry;
mod semantic_desktop;

pub use registry::ToolRegistry;

// Re-export ToolCall from crate root
pub use crate::ToolCall;
