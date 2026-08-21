//! Workflow HUD control — shared state between executor and Tauri commands
//!
//! Enables the HUD overlay to pause/resume/stop active workflows.
//!
//! ## 状态存储
//!
//! - ACTIVE_WORKFLOW_ID → `crate::state::SignalState`（SharedSignals 显式注入）
//! - WORKFLOW_USER_CANCELLED — Tauri 命令层 → Core 单向信号（保留独立 AtomicBool）

use std::sync::atomic::{AtomicBool, Ordering};

/// Set the currently active workflow. Called by executor at start of execution.
pub fn set_active(signals: &crate::state::SharedSignals, workflow_id: &str) {
    crate::state::SignalState::write(signals).active_workflow_id = Some(workflow_id.to_string());
}

/// Clear the active workflow ID. Called by executor on completion/cancellation.
pub fn clear_active(signals: &crate::state::SharedSignals) {
    crate::state::SignalState::write(signals).active_workflow_id = None;
}

/// Get the current active workflow ID (clone)
pub fn active_id(signals: &crate::state::SharedSignals) -> Option<String> {
    crate::state::SignalState::read(signals)
        .active_workflow_id
        .clone()
}

// ── User cancellation signal — wf_stop → react_loop ──

/// Set to true when user explicitly stops a workflow via wf_stop / hud_stop.
/// Consumed by react_loop after workflow_run's execute_v2 returns.
static WORKFLOW_USER_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Mark that the user has explicitly cancelled the active workflow.
/// Called by wf_stop / hud_stop Tauri commands.
pub fn mark_user_cancelled() {
    WORKFLOW_USER_CANCELLED.store(true, Ordering::SeqCst);
}

/// Read and clear the user-cancelled flag (one-shot consumption).
/// Returns true if user explicitly stopped the workflow since last check.
pub fn take_user_cancelled() -> bool {
    WORKFLOW_USER_CANCELLED.swap(false, Ordering::SeqCst)
}
