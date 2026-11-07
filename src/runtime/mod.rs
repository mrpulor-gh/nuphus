//! Runtime — Unified runtime
//!
//! Replaces the old dual-Agent architecture (Leader + Exec) with a single main loop + three modes.
//!
//! ## Architecture
//!
//! The ReAct loop is inlined from ReactAgent::run() into Runtime::react_loop().
//! Runtime truly owns the execution entry point; ReactAgent degrades to a pure state container.
//! Security chain is unified into exec_tool::check_tool_security().

/// 执行身份：区分「用户任务轮」与「提炼轮」。
///
/// 提炼（refine）复用 react_loop 执行，但它**不是任务执行**：
/// - 不参与上下文预算检查（尤其 85% 硬止损）。否则提炼轮会再次触发提炼，
///   形成 `refine_in_loop → resume → react_loop → refine_in_loop` 的无界嵌套；
///   `Box::pin` 只固定 future 大小、不消除栈帧，每层实占一个完整栈帧，
///   最终撑爆 tokio worker 2MB 栈（STATUS_STACK_OVERFLOW / 0xc00000fd）。
/// - 不触发 `session_end` 生命周期 hook（会话并未结束）。
///
/// 任务轮（Task）走完整逻辑；提炼落盘后由**同一个任务轮迭代**继续执行用户任务，
/// 这才是「正常继续」，与「嵌套提炼」在结构上分开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    Task,
    Refine,
}

pub mod dispatch;
pub mod r#loop;
pub mod mode;
pub mod protection;
pub mod react_loop;
pub mod sub_task;
pub mod sub_task_loop;
pub mod sub_task_shell;
pub mod tool_lifecycle;
pub mod workflow_agent;

pub use mode::Mode;
pub use r#loop::{Runtime, RuntimeBuilder, RuntimeConfig, RuntimeEvent};
pub use sub_task::SubTaskRunner;
pub use workflow_agent::WorkflowAgent;

#[cfg(test)]
mod tests;
