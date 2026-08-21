//! lifecycle — 执行生命周期控制命令
//!
//! interrupt / pause / continue / append / terminate / graceful_stop / force_reset / is_busy
//! 操作 AppState 的 cancel_flag / pause_flag / busy 等标志。

use crate::emitter::TauriEventEmitter;
use crate::state::AppState;
use nuphus::agent::events::{EventEmitter, NuphusEvent};
use std::sync::atomic::Ordering;
use tauri::State;

#[tauri::command]
pub fn interrupt(state: State<'_, AppState>) -> Result<String, String> {
    state.cancel_flag.store(true, Ordering::SeqCst);
    nuphus::agent::pause::clear_pause_action_id(&state.signals);
    tracing::info!("[INTERRUPT] cancel_flag set to true");
    Ok("Task interrupted".to_string())
}

/// 暂停执行(弹出中断菜单:继续/追加/终止)
/// 立即发射 ExecutionPaused 事件让前端弹出暂停菜单，不需要等 Agent 循环到检查点
#[tauri::command]
pub fn pause_execution(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let action_id = uuid::Uuid::new_v4().to_string();
    nuphus::agent::pause::set_pause_action_id(&state.signals, &action_id);
    state.pause_flag.store(true, Ordering::SeqCst);

    // 立即通知前端显示暂停菜单，Agent 循环到检查点时会复用同一个 action_id
    let emitter = TauriEventEmitter {
        app,
        seq: state.event_seq.clone(),
    };
    emitter.emit(NuphusEvent::ExecutionPaused {
        action_id: action_id.clone(),
    });

    tracing::info!("[PAUSE] pause_flag set to true, action_id: {}", action_id);
    Ok("Task paused".to_string())
}

/// 继续执行(用户点击"继续"按钮)
/// 注：不清 pause_flag 也不清 action_id，由运行时循环消费决策后自行清除。
/// 循环进入暂停等待前会先用 action_id 查决策，已存在则直接处理不弹窗。
#[tauri::command]
pub fn continue_execution(state: State<'_, AppState>, action_id: String) -> Result<String, String> {
    nuphus::agent::pause::set_pause_decision(
        &state.signals,
        &action_id,
        nuphus::agent::pause::PauseDecision::Continue,
    );
    // PAUSE_CLAIMED 由 Agent 循环统一释放，不在 Tauri 命令中释放
    tracing::info!("[PAUSE] Continue execution: {}", action_id);
    Ok("continued".to_string())
}

/// 追加指令后继续执行(用户输入新指令后点击发送)
#[tauri::command]
pub fn append_instruction(
    state: State<'_, AppState>,
    action_id: String,
    instruction: String,
) -> Result<String, String> {
    nuphus::agent::pause::set_pause_decision(
        &state.signals,
        &action_id,
        nuphus::agent::pause::PauseDecision::Append(instruction),
    );
    // PAUSE_CLAIMED 由 Agent 循环统一释放，不在 Tauri 命令中释放
    tracing::info!("[PAUSE] Append instruction: {}", action_id);
    Ok("appended".to_string())
}

/// 终止执行(用户点击"终止"按钮)
/// 不设 cancel_flag，由循环的 pause check 读到 Terminate 决策后
/// 注入系统提示词并走 leader_should_stop 优雅退出。
#[tauri::command]
pub fn terminate_execution(
    state: State<'_, AppState>,
    action_id: String,
) -> Result<String, String> {
    nuphus::agent::pause::set_pause_decision(
        &state.signals,
        &action_id,
        nuphus::agent::pause::PauseDecision::Terminate,
    );
    // PAUSE_CLAIMED 由 Agent 循环统一释放，不在 Tauri 命令中释放
    tracing::info!("[PAUSE] Terminate execution: {}", action_id);
    Ok("terminated".to_string())
}

/// 优雅停止：设置 pause_flag + 预置 Terminate 决策，不弹暂停菜单。
/// Agent 循环检测到 pause_flag 后，直接走 Terminate 路径：
///   → LLM 整理输出 → 保存结果 → 返回
/// 与用户点暂停菜单「终止」等价，但跳过前端弹窗。
#[tauri::command]
pub fn graceful_stop(state: State<'_, AppState>) -> Result<String, String> {
    let action_id = uuid::Uuid::new_v4().to_string();
    nuphus::agent::pause::set_pause_action_id(&state.signals, &action_id);
    // Pre-set Terminate decision so agent loop skips ExecutionPaused emit
    nuphus::agent::pause::set_pause_decision(
        &state.signals,
        &action_id,
        nuphus::agent::pause::PauseDecision::Terminate,
    );
    state.pause_flag.store(true, Ordering::SeqCst);
    tracing::info!(
        "[GRACEFUL-STOP] pause_flag + Terminate pre-set, action_id: {}",
        action_id
    );
    Ok(action_id)
}

#[tauri::command]
pub fn force_reset(state: State<'_, AppState>) -> Result<String, String> {
    let was_busy = state.busy.swap(false, Ordering::SeqCst);
    state.cancel_flag.store(true, Ordering::SeqCst);
    state.pause_flag.store(false, Ordering::SeqCst);
    nuphus::agent::pause::clear_pause_action_id(&state.signals);
    tracing::warn!("[FORCE-RESET] busy={}, forced by user", was_busy);
    Ok(format!("forced reset (was busy: {})", was_busy))
}

#[tauri::command]
pub fn is_busy(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.busy.load(Ordering::SeqCst))
}
