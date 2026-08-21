//! Tools module - pluggable tool system

pub mod browser_tools;
pub mod builtin;
pub mod definitions;
pub mod desktop_executors;
pub mod desktop_schemas;
pub mod registry;

pub use registry::ToolRegistry;

// Re-export ToolCall from crate root
pub use crate::ToolCall;
