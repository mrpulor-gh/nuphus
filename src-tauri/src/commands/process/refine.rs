//! Session refine — Leader + Workflow dual-slot dispatch

use crate::emitter::CompoundEmitter;
use crate::state::AppState;
use nuphus::agent::events::{EventEmitter, NuphusEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Refine 内部流程的过滤发射器：只放行 LlmTextDelta（前端提炼气泡流式渲染），
/// 静默其余事件（ExecutionStarted / UserMessageReceived / ExecutionCompleted /
/// DirectResponse / TokenUsage / ToolCall* 等）。
/// 此前 Leader/Workflow 两路径均 take_emitter 全静默：提炼期间前端收不到任何
/// delta，气泡空转直到 session_refined 一次性填 summary——双 mode 提炼均无
/// 流式输出的根因（2026-08-30 修复）。REFINE_PROMPT 是纯总结任务，正常不产生
/// 工具调用；即使 LLM 意外调工具，ToolCall* 也被静默（提炼不污染执行轨迹）。
struct RefineStreamFilter {
    inner: Arc<dyn EventEmitter>,
}

impl EventEmitter for RefineStreamFilter {
    fn emit(&self, event: NuphusEvent) {
        if matches!(event, NuphusEvent::LlmTextDelta { .. }) {
            self.inner.emit(event);
        }
    }
}

/// 提炼的墙钟预算（秒）。
///
/// 云端沿用原来的值（Leader 90s / Workflow 60s，**行为不变**）；**本地端点**
/// （本机/局域网推理服务）改用 provider 配置超时的下限语义：
/// `max(配置的 timeout_secs, LOCAL_TIMEOUT_FLOOR_SECS)`，再加 60s 余量，
/// 好让传输层自己的超时先报错（错误信息更准确：是"请求超时"而不是"提炼超时"）。
///
/// 依据（实测，192.168.5.150 llama-swap + Qwen3.8-27B-Q8 / 256K ctx）：
/// prefill ≈ 2,000 tok/s、decode ≈ 15 tok/s ⇒ 20 万 token 上下文提炼 ≈ 5.5 分钟，
/// 原 60/90s 差约 5 倍。详见 `nuphus::config::provider::LOCAL_TIMEOUT_FLOOR_SECS`。
fn refine_timeout_secs(state: &tauri::State<'_, AppState>, cloud_default: u64) -> u64 {
    let (base_url, provider) = {
        let Ok(guard) = state.runtime.lock() else {
            return cloud_default;
        };
        match guard.llm_config.as_ref() {
            Some(cfg) => (cfg.base_url.clone(), cfg.provider.clone()),
            None => return cloud_default,
        }
    };
    if !nuphus::config::provider::is_local_endpoint(&base_url, None) {
        return cloud_default;
    }
    let configured =
        crate::commands::config::read_provider_timeout_secs_from_config_toml(&provider)
            .unwrap_or(300);
    nuphus::config::provider::effective_timeout_secs(&base_url, None, configured) + 60
}

pub async fn execute_session_refine<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    // CompoundEmitter 双推：RefineExecuting/SessionRefined/RefineFailed 同时到桌面
    // Tauri 与手机 WS，手机端 refine 弹窗状态同步（桌面端零回归：mobile 为 None 时
    // 退化为纯 Tauri）。RefineFailed 与 RefineExecuting 必须成对——失败不广播结束
    // 事件会让双端提炼 UI 永久卡在 spinner。
    // ── refine 专属防重（同 session 双 refine 拒绝）──
    // refine_active 原子 compare_exchange：第一次 false→true 成功；第二次（提炼
    // 执行中，桌面/手机任一端再触发）读到已 true → 立即 Err「提炼进行中」，
    // 不 broadcast 任何事件（RefineExecuting 已由首次触发广播，双端 UI 已在锁）。
    // 此前只有 busy guard：busy 在普通执行中也为 true，无法区分「提炼进行中」。
    let refine_active = state.refine_active.clone();
    if refine_active
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("提炼进行中，请等待当前提炼完成后再试。".to_string());
    }

    let emitter = CompoundEmitter::new(app.clone(), &state);
    // ── busy 置位（强刷���因修复）── refine 期间 leader/workflow agent 被 take 移出
    // runtime，若不声明 busy：① guard_switch 放行 → can_switch=true，SessionRail 轮询
    // 看到 activeId 突变（active 条目消失）误判外部切换 → 前端整列重拉旧历史（实测
    // 「提炼前后对话窗口强制刷新」回归）；② refine 期间可切换会话，与 take/put 并发
    // 竞态。swap 记录旧值，Drop 恢复——forced 路径（主循环内 busy 本为 true）嵌套安全。
    let prev_busy = state.busy.swap(true, Ordering::SeqCst);
    struct RefineGuard {
        flag: Arc<AtomicBool>,
        busy: Arc<AtomicBool>,
        prev_busy: bool,
    }
    impl Drop for RefineGuard {
        fn drop(&mut self) {
            self.flag.store(false, Ordering::SeqCst);
            self.busy.store(self.prev_busy, Ordering::SeqCst);
        }
    }
    let _refine_guard = RefineGuard {
        flag: refine_active,
        busy: state.busy.clone(),
        prev_busy,
    };

    let refine_prompt = nuphus::agent::distill::REFINE_PROMPT;
    let cancel_flag = state.cancel_flag.clone();
    cancel_flag.store(false, Ordering::SeqCst);
    state.pause_flag.store(false, Ordering::SeqCst);

    // ── 模式判定：以 current_mode 为准（与 session.rs::chat_history 同一权威源）──
    // 旧写法 `guard.workflow_agent.is_some()` 是「实例存在性推断模式」：mode.rs 切到
    // Leader 时**保留** workflow_agent（session 不丢失，见 mode.rs:16 注释），因此只要
    // 进过 workflow 模式，该槽永久为 Some → is_some() 恒真 → 切回 leader/custom 后
    // refine 仍去提炼 workflow 会话（用户报告：「refine 内容是最早时的 mode Agent 内容，
    // 不是当前 mode Agent 的内容」）。
    // custom 模式走 leader 主循环（session 存于 leader_agent）→ 按 leader 处理，
    // 与 session.rs:269-276 的归一化语义一致。
    // 锁序：先取 runtime 锁、后读 current_mode（current_mode 是独立 RwLock；mode.rs 写入侧
    // 先 drop(guard) 再写，无反向嵌套，故本顺序无死锁环），与 session.rs::chat_history 相同。
    let (is_workflow, agent_ready) = {
        let guard = state.runtime.lock().map_err(|e| e.to_string())?;
        let current_mode = state
            .current_mode
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|_| "leader".to_string());
        let is_workflow = current_mode == "workflow";
        // 目标模式的 agent 可用性检查（「实例可用性」，与上面的模式判定严格分离）
        let agent_ready = if is_workflow {
            guard.workflow_agent.is_some()
        } else {
            guard.leader_agent.is_some()
        };
        tracing::info!(
            "[REFINE] mode={} is_workflow={} agent_ready={}",
            current_mode,
            is_workflow,
            agent_ready
        );
        (is_workflow, agent_ready)
    };

    // Verify the *target mode's* agent exists before emitting RefineExecuting：
    // 缺失时前置失败（不广播 RefineExecuting/RefineFailed），避免双端提炼 UI 进入 spinner
    // 后等不到结束事件；也避免旧行为下「mode=workflow 但 workflow_agent 未懒初始化」
    // 时静默回落到 leader 会话提炼（错误会话）。
    if !agent_ready {
        return Err(if is_workflow {
            "No active workflow agent — refine requires an active session.".to_string()
        } else {
            "No active agent — refine requires an active session.".to_string()
        });
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
        // 静默 resume 内的 ExecutionStarted（goal=REFINE_PROMPT 开头，也不应显示在前端）：
        // 用过滤器替换而非 take 成 None——LlmTextDelta 放行供前端提炼气泡流式渲染，
        // 生命周期事件仍被拦截（全静默会让提炼等 session_refined 才一次性出结果）。
        let saved_emitter = rt_owned.take_emitter();
        if let Some(ref e) = saved_emitter {
            rt_owned.restore_emitter(Some(Arc::new(RefineStreamFilter { inner: e.clone() })));
        }
        let budget = refine_timeout_secs(&state, 90);
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(budget),
            rt_owned.resume(refine_prompt, &cancel_flag),
        )
        .await
        .map_err(|_| format!("提炼超时（{budget}s）"))
        .and_then(|r| r.map_err(|e| e.to_string()));
        let _ = rt_owned.take_emitter(); // 丢弃过滤器
        rt_owned.restore_emitter(saved_emitter);
        let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
        guard.leader_agent = Some(rt_owned);
        match result {
            Ok(output) => output,
            Err(reason) => {
                // 失败也必须广播结束事件：RefineExecuting 已让双端进入「提炼中」UI，
                // 只 return Err 会让非发起方（forced 自动提炼弹窗 / 手机提炼卡片）
                // 永久 spinner——LLM key 失效/连不上时的假死根因。
                emitter.emit(NuphusEvent::RefineFailed {
                    message: format!("提炼失败：{reason}，会话保持不变。"),
                });
                return Err(format!("提炼失败：{reason}。"));
            }
        }
    };

    let distill = refine_output.message.trim().to_string();
    if distill.is_empty() || !refine_output.success {
        emitter.emit(NuphusEvent::RefineFailed {
            message: "提炼失败：未产出有效摘要，会话保持不变。".to_string(),
        });
        return Err("提炼失败：未产出有效摘要。".to_string());
    }

    // 快照保护名单先于 runtime 锁收集（protected_snapshot_ids 内部会取同一把
    // std Mutex，持锁时调用会死锁）
    let protected = crate::commands::process::shelf::protected_snapshot_ids(state.inner());
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
        // 提炼结果必须立刻落盘。否则 SQLite 快照/断点仍是提炼前的全量——任何非干净退出
        // （崩溃/强杀）都会把它恢复回来，重启后再次越过 force_limit →
        // 「提炼 → 丢失 → 重提炼」死循环（issue #9 RC2）。
        crate::commands::process::shelf::persist_and_mirror("leader", leader.session(), &protected);
    }
    crate::commands::process::shelf::apply_refined_title(state.inner(), &session_id, &distill);

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
        // 同 Leader 路径：过滤器替换而非全静默——放行 LlmTextDelta（提炼气泡流式），
        // 拦截 ExecutionStarted 等生命周期事件。llm_stream_with_streaming 每次 attempt
        // 从 self.emitter.clone() 取发射器（workflow_agent.rs），run 期间 emitter 为
        // 过滤器即生效；run 结束后 take 丢弃过滤器、还原原发射器。
        if let Some(ref e) = saved_emitter {
            wa_owned.set_emitter(Some(Arc::new(RefineStreamFilter { inner: e.clone() })));
        }
        let budget = refine_timeout_secs(&state, 60);
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(budget),
            wa_owned.run(refine_prompt, &None, cancel_flag),
        )
        .await
        .map_err(|_| format!("提炼超时（{budget}s）"))
        .and_then(|r| r.map_err(|e| e.to_string()));
        let _ = wa_owned.take_emitter(); // 丢弃过滤器
        wa_owned.set_emitter(saved_emitter);
        wa_owned.set_internal_input(false);
        let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
        guard.workflow_agent = Some(wa_owned);
        match result {
            Ok(output) => output,
            Err(reason) => {
                // 同 Leader 分支：失败广播 RefineFailed，双端提炼 UI 才能退出 spinner
                emitter.emit(NuphusEvent::RefineFailed {
                    message: format!("提炼失败：{reason}，会话保持不变。"),
                });
                return Err(format!("提炼失败：{reason}。"));
            }
        }
    };

    let distill = refine_output.message.trim().to_string();
    if distill.is_empty() || !refine_output.success {
        emitter.emit(NuphusEvent::RefineFailed {
            message: "提炼失败：未产出有效摘要，会话保持不变。".to_string(),
        });
        return Err("提炼失败：未产出有效摘要。".to_string());
    }

    let protected = crate::commands::process::shelf::protected_snapshot_ids(state.inner());
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
        // 同 leader 路径：提炼结果立即落盘，避免重启恢复全量后重炼（issue #9 RC2）
        crate::commands::process::shelf::persist_and_mirror("workflow", wa.session(), &protected);
    }
    crate::commands::process::shelf::apply_refined_title(state.inner(), &session_id, &distill);

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
