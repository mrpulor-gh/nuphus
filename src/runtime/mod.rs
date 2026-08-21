//! Runtime — Unified runtime
//!
//! Replaces the old dual-Agent architecture (Leader + Exec) with a single main loop + three modes.
//!
//! ## Architecture
//!
//! The ReAct loop is inlined from ReactAgent::run() into Runtime::react_loop().
//! Runtime truly owns the execution entry point; ReactAgent degrades to a pure state container.
//! Security chain is unified into exec_tool::check_tool_security().

pub mod dispatch;
pub mod r#loop;
pub mod mode;
pub mod protection;
pub mod react_loop;
pub mod sub_task;
pub mod sub_task_loop;
pub mod sub_task_shell;
pub mod workflow_agent;

pub use mode::Mode;
pub use r#loop::{Runtime, RuntimeBuilder, RuntimeConfig, RuntimeEvent};
pub use sub_task::SubTaskRunner;
pub use workflow_agent::WorkflowAgent;

#[cfg(test)]
mod tests;
