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
pub async fn interrupt(state: State<'_, AppState>) -> Result<String, String> {
    state.cancel_flag.store(true, Ordering::SeqCst);
    nuphus::agent::pause::clear_pause_action_id(&state.signals);

    // 工作流是由 WorkflowAgent 在一次 `workflow_run` 工具调用内部执行的：执行期间
    // agent 循环**阻塞在这个工具调用里**，`cancel_flag` 要等工具返回后才被检查 ——
    // 也就是说只置 cancel_flag，正在跑（或正停在 wait 步骤）的工作流根本停不下来，
    // 而本命令仍返回 Ok → 用户侧表现就是"点了中断毫无反应"。
    //
    // 用户点「中断」的意图是"停掉当前执行"，所以这里一并取消活动工作流。
    // `active_id` 由执行器 set_active/clear_active 维护，wait 步骤轮询期间也一直有值。
    // mark_user_cancelled 是既有机制：工具返回后 react_loop 据此不再重启工作流。
    if let Some(wf_id) = nuphus::workflow::hud_control::active_id(&state.signals) {
        let engine = state.workflow_engine.read().await;
        engine.cancel_workflow(&wf_id).await;
        nuphus::workflow::hud_control::mark_user_cancelled();
        tracing::info!(
            "[INTERRUPT] cancel_flag set + cancelled active workflow: {}",
            wf_id
        );
    } else {
        tracing::info!("[INTERRUPT] cancel_flag set (no active workflow)");
    }

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
    let instruction = instruction.trim().to_string();
    if instruction.is_empty() {
        return Err("追加消息不能为空".to_string());
    }

    // 与本轮主指令同内容 = 重复提交（界面重载 / 前端热更新 / 重试是常见来源）：
    // 整轮拒绝，不做二次注入——执行中提交同一内容没有合法语义（要追加新内容，内容必不同）。
    // 判据唯一真源：`nuphus::mobile_append::is_duplicate_of_last`（不设时间窗口）。
    let duplicate = state
        .session
        .lock()
        .ok()
        .map(|guard| nuphus::mobile_append::is_duplicate_of_last(&guard.last_message, &instruction))
        .unwrap_or(false);
    if duplicate {
        tracing::info!(
            "[APPEND] 丢弃与本轮主指令重复的追加: action_id={}",
            action_id
        );
        return Ok("duplicate".to_string());
    }

    // 暂停弹窗仍由 action_id 决策表消费；执行中追加则立即进入唯一真实队列，
    // 由当前 agent 的迭代边界消费，避免等待 Leader dispatch 完成。
    if state.pause_flag.load(Ordering::SeqCst) {
        nuphus::agent::pause::set_pause_decision(
            &state.signals,
            &action_id,
            nuphus::agent::pause::PauseDecision::Append(instruction.clone()),
        );
    } else if state.busy.load(Ordering::SeqCst) {
        // 走共享 enqueue：队列内同内容不再重复入队（与整轮判据同源，两道防线）
        if !nuphus::mobile_append::enqueue(&state.signals, instruction) {
            tracing::info!(
                "[APPEND] 丢弃重复追加（已在队列中）: action_id={}",
                action_id
            );
            return Ok("duplicate".to_string());
        }
    } else {
        return Err("当前没有正在执行的任务".to_string());
    }
    tracing::info!("[APPEND] instruction accepted: {}", action_id);
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

/// 强制重置的等待上限：旧执行体收到 `cancel_flag` 后收敛到 Idle 的时间预算。
/// 覆盖「迭代边界检查 + 流式 LLM 收尾 + 运行态回写」；超时即如实报告未收敛。
const FORCE_RESET_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// 等待轮询间隔（50ms：用户点「强制重置」的感知延迟要小，且不空转 CPU）
const FORCE_RESET_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// 强制重置 —— **真终止**，不做「假解锁」。
///
/// 旧实现直接 `busy.swap(false)`（经视图映射 → stage=Idle）并清追加队列，只置
/// `cancel_flag` **请求**停止就立刻返回：用户随即能开出新轮次，而旧执行体仍在跑
/// —— 两个执行体并存（双跑）会并发操作进程级浏览器单例 → 死锁 / panic 全崩。
///
/// 现在的语义（与 `execution_stage` 单一真相源一致）：
/// 1. 置 `cancel_flag` 请求旧执行体优雅退出（不写阶段）；
/// 2. **等待**阶段真正回到 `Idle`（旧执行体的轮次所有者 guard drop 时写入），
///    上限 [`FORCE_RESET_WAIT_TIMEOUT`]；
/// 3. 超时 → **不写 Idle**，返回稳定错误码并记 error 日志：宁可用户看到「还在收敛」，
///    也绝不放出「Idle 但旧任务仍在跑」这个双跑入口；
/// 4. 确认退出后才清残留追加队列（与轮次所有者 guard 的清理同源，此处兜底）。
#[tauri::command]
pub async fn force_reset(state: State<'_, AppState>) -> Result<String, String> {
    let was_busy = state.busy.load(Ordering::SeqCst);
    state.cancel_flag.store(true, Ordering::SeqCst);
    state.pause_flag.store(false, Ordering::SeqCst);
    nuphus::agent::pause::clear_pause_action_id(&state.signals);

    if was_busy {
        tracing::warn!(
            "[FORCE-RESET] 已请求旧执行体退出，等待其真正收敛（≤{}s）",
            FORCE_RESET_WAIT_TIMEOUT.as_secs()
        );
        let exited = nuphus::state::SignalState::wait_until_idle(
            &state.signals,
            FORCE_RESET_WAIT_TIMEOUT,
            FORCE_RESET_POLL_INTERVAL,
        )
        .await;
        if !exited {
            // 关键：此处**绝不**无条件写 Idle（旧实现的假解锁点）
            let held_stage = state.busy.stage();
            tracing::error!(
                "[FORCE-RESET] 旧执行体 {}s 内未退出（stage={}）：拒绝假解锁，保持占用态以免双跑",
                FORCE_RESET_WAIT_TIMEOUT.as_secs(),
                held_stage.as_str()
            );
            return Err(format!(
                "{}: 旧任务在 {} 秒内未退出（当前 stage={}），已请求终止但尚未收敛；请稍后再试或等待其自行结束",
                nuphus::automation_gate::CODE_BUSY,
                FORCE_RESET_WAIT_TIMEOUT.as_secs(),
                held_stage.as_str()
            ));
        }
    }

    // 旧执行体已确认退出（或本就空闲）：清残留追加队列再做状态自洽性收口。
    let cleared = {
        let mut signals = nuphus::state::SignalState::write(&state.signals);
        std::mem::take(&mut signals.append_queue).len()
    };
    tracing::warn!(
        "[FORCE-RESET] 重置完成：was_busy={}, 清空残留追加 {} 条（旧执行体已退出）",
        was_busy,
        cleared
    );
    Ok(format!("forced reset (was busy: {was_busy})"))
}

/// 兼容壳：只回答「后端是否仍被占用」。新代码请用 [`get_execution_state`]——
/// 追加判定需要区分 Running / Finalizing，单一布尔无法表达（见 ExecutionStage）。
#[tauri::command]
pub fn is_busy(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.busy.load(Ordering::SeqCst))
}

/// 后端唯一权威执行态快照 —— 前端执行态的**单一来源**（拉通道；推通道 = nuphus-event）。
///
/// - `stage`：`"idle"` / `"running"` / `"finalizing"`
/// - `busy`：`stage != "idle"`（= 旧 `is_busy`，终止按钮 / 会话锁定用）
/// - `append_accepting`：当前提交会不会被当作追加指令受理（仅 `"running"`；
///   `"finalizing"` 会返回 `rejected: "finalizing"`，前端须把输入退回输入框）
///
/// 收敛前执行态分散在 5 个来源（前端 isProcessing / 前端 completed / 后端 is_busy /
/// 后端 can_switch / rail 的 OR 派生），同一时刻各方判断可以不一致。现在：
/// 后端只有 `SignalState::execution_stage` 一个存储，前端只有本命令 + 事件两条通道。
#[derive(serde::Serialize)]
pub struct ExecutionStateSnapshot {
    pub stage: String,
    pub busy: bool,
    pub append_accepting: bool,
}

#[tauri::command]
pub fn get_execution_state(state: State<'_, AppState>) -> Result<ExecutionStateSnapshot, String> {
    let stage = state.busy.stage();
    Ok(ExecutionStateSnapshot {
        stage: stage.as_str().to_string(),
        busy: stage.is_busy(),
        append_accepting: stage.accepts_append(),
    })
}

/// Read the backend-owned append queue for the execution control UI.
#[tauri::command]
pub fn get_append_queue(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let signals = nuphus::state::SignalState::read(&state.signals);
    Ok(signals.append_queue.clone())
}

/// Remove one still-pending append instruction by its queue index.
/// The active agent consumes from this same vector, so an already-consumed
/// instruction cannot be deleted through this command.
#[tauri::command]
pub fn remove_append_queue_item(
    state: State<'_, AppState>,
    index: usize,
) -> Result<Vec<String>, String> {
    let mut signals = nuphus::state::SignalState::write(&state.signals);
    if index >= signals.append_queue.len() {
        return Err("追加消息已被执行或不存在".to_string());
    }
    signals.append_queue.remove(index);
    Ok(signals.append_queue.clone())
}
