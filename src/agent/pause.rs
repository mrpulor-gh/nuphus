//! pause — Global pause decision registry
//!
//! ExecuteAgent can suspend before tool execution to wait for user decision (Continue/Append/Terminate).
//! Decisions are passed via a global HashMap between Tauri commands (continue/append/terminate) and Exec polling points.
//!
//! ## 状态存储
//!
//! - PAUSE_DECISIONS / PAUSE_ACTION_ID → `crate::state::SignalState`（SharedSignals 显式注入）
//! - PENDING_APPEND — 跨 session 追加指令队列（保留独立 static）
//!
//! ## 注意
//!
//! 审计确认 sub_task_loop 和 react_loop 不会同时运行（sub_task_loop 在 react_loop 内 await），无需额外的互斥机制。

#[derive(Debug, Clone, PartialEq)]
pub enum PauseDecision {
    Continue,
    Append(String),
    Terminate,
}

// ════════════════════════════════════════════════════════════════
// PAUSE_DECISIONS — 通过 AppState 管理
// ════════════════════════════════════════════════════════════════

/// Set pause decision result (called by Tauri continue/append/terminate commands)
/// Append 追加到已有队列而非覆盖，支持同 action_id 多次追加
pub fn set_pause_decision(
    signals: &crate::state::SharedSignals,
    action_id: &str,
    decision: PauseDecision,
) {
    let mut state = crate::state::SignalState::write(signals);
    match decision {
        PauseDecision::Append(instr) => {
            state
                .pause_decisions
                .entry(action_id.to_string())
                .and_modify(|existing| {
                    if let PauseDecision::Append(ref mut prev) = existing {
                        prev.push('\n');
                        prev.push_str(&instr);
                    }
                })
                .or_insert(PauseDecision::Append(instr));
        }
        _ => {
            state
                .pause_decisions
                .insert(action_id.to_string(), decision);
        }
    }
}

/// Check if a pause decision exists without removing it (for pre-check before emitting ExecutionPaused)
pub fn peek_pause_decision(
    signals: &crate::state::SharedSignals,
    action_id: &str,
) -> Option<PauseDecision> {
    crate::state::SignalState::read(signals)
        .pause_decisions
        .get(action_id)
        .cloned()
}

/// Query pause decision result (polled by ExecuteAgent) — 消费一次
pub fn check_pause_decision(
    signals: &crate::state::SharedSignals,
    action_id: &str,
) -> Option<PauseDecision> {
    crate::state::SignalState::write(signals)
        .pause_decisions
        .remove(action_id)
}

// ════════════════════════════════════════════════════════════════
// PAUSE_ACTION_ID — 通过 AppState 管理
// ════════════════════════════════════════════════════════════════

/// Set current pause action_id (called by pause_execution)
pub fn set_pause_action_id(signals: &crate::state::SharedSignals, action_id: &str) {
    crate::state::SignalState::write(signals).pause_action_id = Some(action_id.to_string());
}

/// Get current pause action_id (polled by Agent)
pub fn get_pause_action_id(signals: &crate::state::SharedSignals) -> Option<String> {
    crate::state::SignalState::read(signals)
        .pause_action_id
        .clone()
}

/// Clear pause action_id (called by continue/append/terminate)
pub fn clear_pause_action_id(signals: &crate::state::SharedSignals) {
    crate::state::SignalState::write(signals).pause_action_id = None;
}

// ════════════════════════════════════════════════════════════════
// PENDING_APPEND — 跨 session 追加指令队列（保留全局 static）
// ════════════════════════════════════════════════════════════════

use std::sync::Mutex;

/// Global append instruction queue — persistent across sessions and runs.
/// Pause handler pushes here, after run()/dispatch returns, drain and merge into target session.
static PENDING_APPEND: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Push append instruction into global queue
pub fn push_pending_append(instr: String) {
    let mut guard = PENDING_APPEND.lock().unwrap_or_else(|e| e.into_inner());
    guard.push(instr);
}

/// Drain all append instructions from global queue (clears queue)
pub fn drain_pending_append() -> Vec<String> {
    let mut guard = PENDING_APPEND.lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *guard)
}

/// Check if global queue is empty
pub fn has_pending_append() -> bool {
    let guard = PENDING_APPEND.lock().unwrap_or_else(|e| e.into_inner());
    !guard.is_empty()
}

// ════════════════════════════════════════════════════════════════
// Pause polling: wait for user decision
// ════════════════════════════════════════════════════════════════

/// Pause polling: wait for user to make Continue/Append/Terminate decision via frontend dialog
/// Uses async sleep to avoid blocking tokio worker threads
pub async fn wait_for_pause_decision_global(
    signals: &crate::state::SharedSignals,
    action_id: &str,
    cancel_flag: &std::sync::atomic::AtomicBool,
) -> PauseDecision {
    for _ in 0..600 {
        if cancel_flag.load(std::sync::atomic::Ordering::SeqCst) {
            return PauseDecision::Terminate;
        }
        if let Some(decision) = check_pause_decision(signals, action_id) {
            return decision;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    PauseDecision::Continue
}
