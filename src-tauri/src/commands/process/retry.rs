//! retry — ReactAgent 兜底重试
//!
//! retry_agent: LLM 失败后的人工触发重试。
//!
//! 设计原则：
//! - **断点续跑（resume）**：LLM 调用失败时错误内容从未进入 session
//!   （流式内容在缓冲区，成功才落库；strip_incomplete_tools 已清理悬挂工具对），
//!   session 末尾即断点。重试 = 不重发消息、不开新 turn，直接重做那次失败的
//!   LLM 调用——失败回合的全部进度（含已完成的工具调用与副作用）完整保留。
//!   turn id 不变 → 记忆持久化按 entry id INSERT OR REPLACE，成功覆盖失败记录，
//!   零噪声；继续失败继续覆盖，永不累积。0 进度失败时行为与重发等价。
//! - **统一路径**：与正常发送共用 leader::run_runtime_with_config（resume 分支），
//!   接线（exec_resources/context/workflow engine/mode）只有一个实现。
//! - **缓存第一**：优先复用 state 中留存的 Runtime（session、已构建 prompt 缓存、
//!   接线全部保留），重试发送的就是 session 开始时构建好的 system prompt。

use crate::emitter::TauriEventEmitter;
use crate::state::{AppState, ProcessInputResponse};
use nuphus::agent::events::{EventEmitter, NuphusEvent, StepOutput};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::State;

#[tauri::command]
pub async fn retry_agent(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ProcessInputResponse, String> {
    // 1. 取出挂起的重试数据
    let (session_json, config, message) = {
        let mut pending = state.execution.lock().map_err(|e| e.to_string())?;
        pending
            .pending_retry
            .take()
            .ok_or_else(|| "没有可重试的会话".to_string())?
    };

    // 2. 防止并发执行
    if state.busy.swap(true, Ordering::SeqCst) {
        if let Ok(mut pending) = state.execution.lock() {
            pending.pending_retry = Some((session_json, config, message));
        }
        return Err("任务正在执行中,请等待当前任务完成".to_string());
    }
    struct BusyGuard<'a>(&'a AtomicBool);
    impl Drop for BusyGuard<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _guard = BusyGuard(&state.busy);

    let cancel_flag = state.cancel_flag.clone();
    cancel_flag.store(false, Ordering::SeqCst);

    let emitter = TauriEventEmitter {
        app: app.clone(),
        seq: state.event_seq.clone(),
    };

    // 3. 创建 LLM 客户端（与 process.rs 一致：providers.toml registry）
    //    复用留存 Runtime 时仅用于刷新 exec_resources；需要重建时作为主客户端。
    let factory = {
        nuphus::config::load_registry()
            .ok()
            .map(nuphus::llm::ClientFactory::new)
            .or_else(|| {
                let in_mem = state.runtime.lock().ok().and_then(|g| g.llm_config.clone());
                in_mem
                    .filter(|c| !c.model.is_empty() && !c.api_key.is_empty())
                    .map(|cfg| {
                        let registry = nuphus::config::ModelRegistry::from_single(
                            cfg.model.clone(),
                            cfg.provider.clone(),
                            cfg.api_key.clone(),
                            String::new(),
                            cfg.reasoning_effort.clone(),
                        );
                        nuphus::llm::ClientFactory::new(registry)
                    })
            })
    }
    .ok_or_else(|| "无法加载模型配置".to_string())?;
    let llm = factory
        .create_client(&config.model)
        .or_else(|_| factory.create_main_client())
        .map_err(|e| format!("创建 LLM 客户端失败: {}", e))?;

    // Agent 级 exec 模型（单一入口 effective_model）：exec → default → leader
    let exec_model = crate::commands::config::llm::effective_model(
        &state.llm_config_path,
        factory.registry(),
        "exec",
    );
    let exec_llm = factory
        .create_client(&exec_model)
        .map_err(|e| format!("创建 Exec LLM 客户端失败 ({exec_model}): {e}"))?;

    // 4. 断点续跑：失败时错误内容从未进入 session（流式缓冲成功才落库，
    //    strip_incomplete_tools 已清理悬挂工具对），session 末尾即断点——
    //    优先复用留存 Runtime（缓存第一），无留存时由统一路径按 session JSON 恢复，
    //    无需任何回滚/改写。
    let existing_runtime = state
        .runtime
        .lock()
        .ok()
        .and_then(|mut g| g.leader_agent.take());

    // 5. 统一执行路径（与正常发送同一接线函数；resume=true → 断点续跑）
    let start_time = std::time::Instant::now();
    let tool_permissions = state
        .runtime
        .lock()
        .map(|g| g.tool_permissions)
        .unwrap_or_default();
    let refine_threshold = state
        .runtime
        .lock()
        .map(|g| g.refine_threshold)
        .unwrap_or(0.5);

    let run_result = super::leader::run_runtime_with_config(
        llm,
        exec_llm,
        state.tools.clone(),
        &config,
        &message, // 作为 resume goal（不重新 push 进 session）
        &None,    // images
        &None,    // history（session 由留存 Runtime / JSON 恢复）
        &None,    // relation（复用已构建的 context，见缓存第一）
        "",       // soul（同正常路径，经 RelationConfig 传递）
        "desktop",
        tool_permissions,
        state.tool_permissions_ref.clone(),
        &cancel_flag,
        &state.pause_flag,
        &emitter,
        existing_runtime,
        Some(session_json), // 无留存 Runtime 时的 session 恢复来源（已是断点状态）
        refine_threshold,
        None, // mode（复用 Runtime 原 mode；新建则默认 Free）
        state.workflow_engine.clone(),
        true,  // resume：断点续跑
        false, // fresh：retry 是续跑既有会话，不按新建处理
    )
    .await;

    let (output, runtime) = match run_result {
        Ok(ok) => ok,
        Err(err_msg) => {
            tracing::error!("Retry agent error: {}", err_msg);
            emitter.emit(NuphusEvent::ExecutionError {
                step_index: 0,
                error: err_msg.clone(),
            });
            return Err(err_msg);
        }
    };

    // 6. Runtime 存回 state（与正常路径 process.rs 一致）
    if let Ok(mut guard) = state.runtime.lock() {
        guard.leader_agent = Some(runtime);
    }

    // 7. 持久化对话记忆（与正常路径共享同一实现）：
    //    重试复用失败回合的 turn id，INSERT OR REPLACE 覆盖失败记录。
    let response_message = if output.message.trim().is_empty() && output.success {
        "（模型未产出有效回复，可能需重试）".to_string()
    } else {
        output.message.clone()
    };
    super::persist_leader_turn(state.inner(), &message, &response_message, output.success);

    // 8. 仍失败则保存现场，供再次重试（同 turn id，下次重试继续覆盖）
    if let Some(retry_json) = &output.retry_session {
        if !retry_json.is_empty() {
            if let Ok(mut pending) = state.execution.lock() {
                pending.pending_retry = Some((retry_json.clone(), config, message.clone()));
            }
            tracing::info!("[RETRY] Re-saved session for further retry");
        }
    }

    // 9. 发射完成事件（ExecutionStarted/流式事件由统一路径在运行中发射，
    //    与正常路径一致，前端由 ExecutionCompleted 驱动收尾）
    let elapsed = start_time.elapsed().as_millis() as u64;
    let total_calls = output.steps.len();
    emitter.emit(NuphusEvent::ExecutionCompleted {
        step_index: 0,
        output: StepOutput {
            step_index: 0,
            result_message: response_message.clone(),
            artifacts: vec![],
            tool_calls_count: total_calls,
        },
        total_duration_ms: elapsed,
        total_calls,
    });

    Ok(ProcessInputResponse {
        success: output.success,
        message: response_message,
        appended: None,
        image_warning: None,
    })
}
