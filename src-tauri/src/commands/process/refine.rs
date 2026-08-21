//! Session refine — Leader + Workflow dual-slot dispatch

use crate::emitter::CompoundEmitter;
use crate::state::AppState;
use nuphus::agent::events::{EventEmitter, NuphusEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub async fn execute_session_refine<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    // CompoundEmitter 双推：RefineExecuting/SessionRefined 同时到桌面 Tauri 与手机 WS，
    // 手机端 refine 弹窗状态同步（桌面端零回归：mobile 为 None 时退化为纯 Tauri）。
    let emitter = CompoundEmitter::new(app.clone(), &state);

    let refine_active = state.refine_active.clone();
    refine_active.store(true, Ordering::SeqCst);
    struct RefineGuard {
        flag: Arc<AtomicBool>,
    }
    impl Drop for RefineGuard {
        fn drop(&mut self) {
            self.flag.store(false, Ordering::SeqCst);
        }
    }
    let _refine_guard = RefineGuard {
        flag: refine_active,
    };

    let refine_prompt = nuphus::agent::distill::REFINE_PROMPT;
    let cancel_flag = state.cancel_flag.clone();
    cancel_flag.store(false, Ordering::SeqCst);
    state.pause_flag.store(false, Ordering::SeqCst);

    let is_workflow = {
        let guard = state.runtime.lock().map_err(|e| e.to_string())?;
        guard.workflow_agent.is_some()
    };

    if !is_workflow {
        // Verify Leader agent exists before emitting RefineExecuting
        let guard = state.runtime.lock().map_err(|e| e.to_string())?;
        if guard.leader_agent.is_none() {
            return Err("No active agent — refine requires an active session.".to_string());
        }
    }

    emitter.emit(NuphusEvent::RefineExecuting);

    if is_workflow {
        return execute_workflow_refine(app, state, emitter, &cancel_flag).await;
    }

    // Leader refine
    let (total_msgs, session_id) = {
        let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
        let leader = guard
            .leader_agent
            .as_mut()
            .ok_or_else(|| "No active agent".to_string())?;
        (
            leader.session().messages().len(),
            leader.session().id.clone(),
        )
    };

    let refine_output = {
        let mut rt_owned = {
            let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
            guard
                .leader_agent
                .take()
                .ok_or_else(|| "No active agent".to_string())?
        };
        // REFINE_PROMPT 属系统提示词：以 internal user 消息入 session（LLM 上下文可见、
        // to_api_messages 不过滤 internal → 缓存不裂；extract_history 过滤 → 前端不显示）。
        // 走 resume（不 emit UserMessageReceived / 不重复 push_user / 不 advance_turn）：
        // 提炼是内部流程，不应以「用户消息」形式出现在桌面聊天与手机界面。
        rt_owned
            .session_mut()
            .push_user_internal(refine_prompt.to_string());
        // 静默 resume 内的 ExecutionStarted（goal=REFINE_PROMPT 开头，也不应显示在前端）。
        let saved_emitter = rt_owned.take_emitter();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(90),
            rt_owned.resume(refine_prompt, &cancel_flag),
        )
        .await
        .map_err(|_| "提炼超时（90s）。".to_string())
        .and_then(|r| r.map_err(|e| format!("提炼失败：{}", e)));
        rt_owned.restore_emitter(saved_emitter);
        let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
        guard.leader_agent = Some(rt_owned);
        result?
    };

    let distill = refine_output.message.trim().to_string();
    if distill.is_empty() || !refine_output.success {
        emitter.emit(NuphusEvent::SessionRefined {
            summary: String::new(),
            message_count: total_msgs,
            session_id: session_id.clone(),
        });
        emitter.emit(NuphusEvent::DirectResponse {
            message: "提炼失败，会话保持不变。".to_string(),
        });
        return Err("提炼失败：未产出有效摘要。".to_string());
    }

    {
        let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
        let leader = guard
            .leader_agent
            .as_mut()
            .ok_or_else(|| "No active agent".to_string())?;
        let _ = leader.save_refine_entry(&distill, "user_session_refine");
        if leader.session().is_refined() {
            leader.session_mut().accumulate_distill(&distill);
        } else {
            leader.session_mut().replace_with_distill(&distill);
        }
        leader.agent_mut().refine_count += 1;
    }

    emitter.emit(NuphusEvent::SessionRefined {
        summary: distill.clone(),
        message_count: total_msgs,
        session_id: session_id.clone(),
    });
    emitter.emit(NuphusEvent::DirectResponse {
        message: format!("上下文已提炼（原始 {} 条已存档）。", total_msgs),
    });
    Ok(format!("上下文已提炼（原始 {} 条已存档）。", total_msgs))
}

async fn execute_workflow_refine<R: tauri::Runtime, E: EventEmitter>(
    _app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    emitter: E,
    cancel_flag: &AtomicBool,
) -> Result<String, String> {
    let refine_prompt = nuphus::agent::distill::REFINE_PROMPT;

    let (total_msgs, session_id) = {
        let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
        let wa = guard
            .workflow_agent
            .as_mut()
            .ok_or_else(|| "No active workflow agent".to_string())?;
        (wa.session().len(), wa.session().id.clone())
    };

    let refine_output = {
        let mut wa_owned = {
            let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
            guard
                .workflow_agent
                .take()
                .ok_or_else(|| "No active workflow agent".to_string())?
        };
        // REFINE_PROMPT 属系统提示词：以 internal user 消息入 session（LLM 可见、前端不显示）。
        // internal_input=true 让 run 跳过重复 push_user（否则会以普通 user 消息入 session，
        // 提炼失败时残留并被 extract_history 显示）；静默 ExecutionStarted（goal 不应显示）。
        wa_owned.set_internal_input(true);
        wa_owned
            .session_mut()
            .push_user_internal(refine_prompt.to_string());
        let saved_emitter = wa_owned.take_emitter();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            wa_owned.run(refine_prompt, &None, cancel_flag),
        )
        .await
        .map_err(|_| "提炼超时（60s）。".to_string())
        .and_then(|r| r.map_err(|e| format!("Workflow refine failed: {}", e)));
        wa_owned.set_emitter(saved_emitter);
        wa_owned.set_internal_input(false);
        let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
        guard.workflow_agent = Some(wa_owned);
        result?
    };

    let distill = refine_output.message.trim().to_string();
    if distill.is_empty() || !refine_output.success {
        emitter.emit(NuphusEvent::SessionRefined {
            summary: String::new(),
            message_count: total_msgs,
            session_id: session_id.clone(),
        });
        emitter.emit(NuphusEvent::DirectResponse {
            message: "提炼失败，会话保持不变。".to_string(),
        });
        return Err("提炼失败：未产出有效摘要。".to_string());
    }

    {
        let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
        let wa = guard
            .workflow_agent
            .as_mut()
            .ok_or_else(|| "No active workflow agent".to_string())?;
        let _ = nuphus::agent::distill::save_refine_entry(
            &wa.session().id,
            &wa.session().current_turn_id(),
            &distill,
            "user_session_refine",
            nuphus::memory::entry::AgentType::WorkAgent,
        );
        if wa.session().is_refined() {
            wa.session_mut().accumulate_distill(&distill);
        } else {
            wa.session_mut().replace_with_distill(&distill);
        }
        wa.inc_refine_count();
    }

    emitter.emit(NuphusEvent::SessionRefined {
        summary: distill,
        message_count: total_msgs,
        session_id: session_id.clone(),
    });
    emitter.emit(NuphusEvent::DirectResponse {
        message: format!("上下文已提炼（原始 {} 条已存档）。", total_msgs),
    });
    Ok(format!("上下文已提炼（原始 {} 条已存档）。", total_msgs))
}

/// 广播「用户跳过提炼」：一端跳过 → 双端（桌面 Tauri + 手机 WS）同步关闭 refine 弹窗，
/// 避免「手机点了跳过、电脑端弹窗还在」的状态残留。
pub fn broadcast_refine_skip<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: &AppState) {
    let emitter = CompoundEmitter::new(app, state);
    emitter.emit(NuphusEvent::RefineSkipped);
    tracing::info!("[REFINE] RefineSkipped broadcast");
}

/// 桌面端「跳过提炼」纯函数：广播 RefineSkipped 让另一端同步关闭弹窗。
/// #[tauri::command] 由 process.rs 的 thin wrapper 提供（避免宏生成函数重复）。
pub fn refine_skip(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    broadcast_refine_skip(app, state.inner());
    Ok("refine skipped".to_string())
}
