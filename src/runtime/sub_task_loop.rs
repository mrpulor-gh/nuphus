//! SubTaskRunner ReAct main loop — extracted from sub_task.rs
//!
//! Contains run_free() and its private helper methods.
//! Pattern matches react_loop.rs: `impl super::SubTaskRunner`

use crate::agent::events::{NuphusEvent, StepOutput};
use crate::agent::exec_tool;
use crate::agent::pause::PauseDecision;
use crate::agent::reminders::{ReminderCategory, ReminderPriority};
use crate::runtime::protection::ProtectionAlert;
use crate::{
    api::{AssistantEvent, MessageRequest},
    session::ContentBlock,
    ToolCall,
};

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

impl super::SubTaskRunner {
    /// Iteration progressive warning threshold constants
    const WARN_REMIND: f64 = 0.60;
    const WARN_EMPHASIS: f64 = 0.75;
    const WARN_REDLINE: f64 = 0.88;
    const WARN_FORBID: f64 = 0.95;

    /// Free ReACT mode — LLM autonomously plans tool usage, no step constraints
    ///
    /// Returns: (success, result_message, artifacts)
    pub async fn run_free(
        &mut self,
        cancel_flag: &AtomicBool,
    ) -> crate::Result<(bool, String, Vec<crate::ExecutionStep>)> {
        // ── Subtask span ──
        let sub_span = tracing::info_span!(
            "subtask",
            goal = %self.goal.chars().take(80).collect::<String>(),
            goal_type = %self.goal_type.as_ref().map(|g| g.id()).unwrap_or("unknown"),
        );
        let _sub_enter = sub_span.enter();

        tracing::info!("SubTaskRunner run_free started");

        if !self.suppress_lifecycle_events {
            self.emit(NuphusEvent::ExecutionStarted {
                step_index: 0,
                goal: self.goal.chars().take(120).collect(),
                tools: self.tools.tool_names(),
                source: "cli".to_string(),
                mode: "leader".to_string(),
            });
        }

        let mut iter_timer = std::time::Instant::now();
        let mut user_requested_stop = false;
        let mut reasoning_followup_done = false;
        let mut consecutive_empty = 0u32;
        for iteration in 0..self.config.max_iterations {
            if cancel_flag.load(Ordering::SeqCst) {
                if !self.suppress_error_events {
                    self.emit(NuphusEvent::Error {
                        code: "cancelled".to_string(),
                        message: "任务已被用户中断".to_string(),
                        retryable: false,
                        from_subtask: self.suppress_lifecycle_events,
                    });
                }
                let total_duration = self.execution_started_at.elapsed().as_millis() as u64;
                // Record memory before cancellation return
                if let Err(e) = self
                    .learn_from_success(total_duration, "任务已被用户中断")
                    .await
                {
                    tracing::warn!("Failed to learn from cancellation: {}", e);
                }
                return Ok((false, "任务已被用户中断".to_string(), vec![]));
            }

            // ── Pause check ──
            if let Some(ref pause_flag) = self.pause_flag {
                if pause_flag.load(Ordering::SeqCst) {
                    let action_id = crate::agent::pause::get_pause_action_id(self.tools.signals())
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    // Check if decision already pre-set (continue/append/terminate via frontend).
                    // Skip ExecutionPaused emit to prevent re-popup after user has already dismissed the pause UI.
                    let skip_emit =
                        crate::agent::pause::peek_pause_decision(self.tools.signals(), &action_id)
                            .is_some();
                    if !skip_emit {
                        self.emit(NuphusEvent::ExecutionPaused {
                            action_id: action_id.clone(),
                        });
                    }
                    let decision = self.wait_for_pause_decision(&action_id, cancel_flag).await;
                    pause_flag.store(false, Ordering::SeqCst);
                    match decision {
                        PauseDecision::Continue => {
                            tracing::info!("[PAUSE] User chose to continue");
                        }
                        PauseDecision::Append(instr) => {
                            tracing::info!(
                                "[PAUSE] User appended instruction: {}",
                                instr.chars().take(80).collect::<String>()
                            );
                            self.session.push_user(instr);
                        }
                        PauseDecision::Terminate => {
                            tracing::info!("[PAUSE] User chose to terminate — graceful stop");
                            self.session.push_user(
                            "⚠ 用户要求立即停止当前操作。请立即整理已有成果，输出你已完成的内容和当前状态，并说明哪些尚未完成。不要继续执行任何工具调用。".to_string()
                        );
                            user_requested_stop = true;
                            self.user_terminated = true;
                        }
                    }
                }
            }

            self.session.strip_incomplete_tools();

            // ── Iteration progressive warning ──
            if self.inject_iteration_warning(iteration) {
                // Just injected "forbidden" warning, give LLM one last chance this round
            } else if self.max_warning_injected >= 4 {
                tracing::warn!("[FORBID] Force stop at iteration {}", iteration);
                break;
            }

            let llm_start = std::time::Instant::now();
            let events = self.llm_stream_retry(cancel_flag).await?;
            let llm_duration = llm_start.elapsed().as_millis();
            tracing::debug!(
                "[TIMING] iteration {}: llm_stream_retry = {}ms",
                iteration,
                llm_duration
            );

            let assistant_blocks = self.process_events(events);
            let process_duration = llm_start.elapsed().as_millis() - llm_duration;
            if process_duration > 50 {
                tracing::debug!(
                    "[TIMING] iteration {}: process_events = {}ms",
                    iteration,
                    process_duration
                );
            }

            self.emit(NuphusEvent::ExecutionProgress {
                iteration: iteration as u32 + 1,
                max_iterations: self.config.max_iterations as u32,
                tool_calls_so_far: self.tool_call_total_count as usize,
            });

            if assistant_blocks.is_empty() {
                // Empty response guard (mirrors react_loop.rs): a stream that
                // yields zero blocks repeatedly means the model is stuck (e.g.
                // transport produced MessageStop without content). Bound it —
                // otherwise the loop spins until max_iterations with no progress.
                consecutive_empty += 1;
                if consecutive_empty >= 3 {
                    tracing::warn!(
                        "[Exec] {} consecutive empty responses, aborting task",
                        consecutive_empty
                    );
                    let total_duration = self.execution_started_at.elapsed().as_millis() as u64;
                    let result_msg = "模型连续返回空响应（无文本无工具调用），任务中止".to_string();
                    if let Err(e) = self.learn_from_success(total_duration, &result_msg).await {
                        tracing::warn!("Failed to learn from empty stall: {}", e);
                    }
                    return Ok((false, result_msg, vec![]));
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                continue;
            }
            consecutive_empty = 0;

            let tool_calls = crate::agent::common::extract_tool_calls(&assistant_blocks);
            tracing::debug!(
                "[SubTaskRunner] extract_tool_calls found {} tool calls",
                tool_calls.len()
            );

            let valid_ids: std::collections::HashSet<&str> =
                tool_calls.iter().map(|c| c.id.as_str()).collect();
            let filtered_blocks: Vec<ContentBlock> = assistant_blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, .. } if !valid_ids.contains(id.as_str()) => None,
                    _ => Some(b.clone()),
                })
                .collect();

            // ── No tool calls → end directly ──
            if tool_calls.is_empty() {
                self.session.push_assistant(filtered_blocks);
                let text = assistant_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text, .. } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let result_msg = text.trim().to_string();

                // ── Reasoning-only guard ──
                // Model produced only thinking (reasoning) but no text and no tool
                // call — typically the stream was truncated (thinking exhausted the
                // output budget) and text/tool_call never materialised. Treating
                // this as "task complete" is a false success (empty delivery).
                // Ask once for a formal text reply, mirroring react_loop.rs.
                if result_msg.is_empty() {
                    let has_reasoning = assistant_blocks.iter().any(|b| {
                        matches!(
                            b,
                            ContentBlock::Text {
                                reasoning: Some(_),
                                ..
                            }
                        )
                    });
                    if has_reasoning && !reasoning_followup_done {
                        reasoning_followup_done = true;
                        self.session.push_user_internal(
                            "【系统】你刚才只输出了思考过程，没有给出正式交付。请直接输出面向 Leader 的正式回复（任务结果摘要）；如仍需执行工具，请继续调用。不要重复思考过程。".to_string(),
                        );
                        tracing::warn!(
                            "[Exec] reasoning-only turn without text/tool — requesting formal reply"
                        );
                        continue;
                    }
                    // Follow-up still produced only reasoning → explicit failure,
                    // never report a fake success for an empty delivery.
                    if has_reasoning {
                        let fail_msg = "模型连续两轮仅输出思考、未产出正式回复或工具调用（疑似输出被长度限制截断）。任务未完成，无交付物。".to_string();
                        let total_duration = self.execution_started_at.elapsed().as_millis() as u64;
                        if let Err(e) = self.learn_from_success(total_duration, &fail_msg).await {
                            tracing::warn!("Failed to learn from reasoning stall: {}", e);
                        }
                        if !self.suppress_lifecycle_events {
                            self.emit(NuphusEvent::ExecutionError {
                                step_index: 0,
                                error: fail_msg.clone(),
                            });
                        }
                        return Ok((false, fail_msg, vec![]));
                    }
                }

                // When only XML tags were present (e.g. <invoke>/<parameter> that got
                // stripped), fall back to a sensible summary so the caller never receives
                // an empty string.
                let result_msg = if result_msg.is_empty() && self.tool_call_total_count > 0 {
                    format!("任务完成，共执行 {} 次工具调用", self.tool_call_total_count)
                } else {
                    result_msg
                };
                let total_duration = self.execution_started_at.elapsed().as_millis() as u64;
                // learn_from_success (含 StateChecker) 必须在 ExecutionCompleted 之前完成，
                // 否则前端提前释放 isProcessing 后用户发消息会被 busy 标志拦截
                if let Err(e) = self.learn_from_success(total_duration, &result_msg).await {
                    tracing::warn!("Failed to learn from success: {}", e);
                }
                if !self.suppress_lifecycle_events {
                    let tool_count = self.tool_call_total_count as usize;
                    self.emit(NuphusEvent::ExecutionCompleted {
                        step_index: 0,
                        output: StepOutput {
                            step_index: 0,
                            result_message: result_msg.clone(),
                            artifacts: vec![],
                            tool_calls_count: tool_count,
                        },
                        total_duration_ms: total_duration,
                        total_calls: tool_count,
                    });
                }
                return Ok((true, result_msg, vec![]));
            }

            // User requested stop + LLM still returns tool calls → skip execution
            if user_requested_stop {
                self.session.push_assistant(filtered_blocks);
                let text = assistant_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text, .. } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let result_msg = if text.trim().is_empty() {
                    "操作已停止".to_string()
                } else {
                    text.trim().to_string()
                };
                let total_duration = self.execution_started_at.elapsed().as_millis() as u64;
                if let Err(e) = self.learn_from_success(total_duration, &result_msg).await {
                    tracing::warn!("Failed to learn from stop: {}", e);
                }
                if !self.suppress_lifecycle_events {
                    self.emit(NuphusEvent::ExecutionCompleted {
                        step_index: 0,
                        output: StepOutput {
                            step_index: 0,
                            result_message: result_msg.clone(),
                            artifacts: vec![],
                            tool_calls_count: self.tool_call_total_count as usize,
                        },
                        total_duration_ms: total_duration,
                        total_calls: self.tool_call_total_count as usize,
                    });
                }
                return Ok((true, result_msg, vec![]));
            }

            // ── Has tool calls ──
            self.session.push_assistant(assistant_blocks);

            // ── Tool execution (concurrent batch, default 3 per batch) ──
            const BATCH_SIZE: usize = 3;
            let mut checked_calls: Vec<(ToolCall, Option<ProtectionAlert>)> = Vec::new();
            let mut protection_warnings: Vec<String> = Vec::new();

            for call in &tool_calls {
                self.tool_call_total_count += 1;
                self.emit(NuphusEvent::ToolCallStart {
                    call_id: call.id.clone(),
                    tool_name: call.tool.clone(),
                    params: call.params.clone(),
                    iteration: iteration as u32,
                    from_task: self.is_task,
                });

                // System tool HUD hint
                let system_hint = match call.tool.as_str() {
                    "system_shell" => Some("执行系统命令"),
                    "system_sleep" => Some("等待中"),
                    "system_info" => Some("获取系统信息"),
                    "system_env_get" => Some("读取环境变量"),
                    "process_list" => Some("查看进程"),
                    "process_kill" => Some("终止进程"),
                    _ => None,
                };
                if let Some(hint) = system_hint {
                    self.emit(NuphusEvent::HudUpdate {
                        text: hint.to_string(),
                        phase: "running".into(),
                        step_kind: None,
                    });
                }

                // Protection check
                let alert = self.protection.check_pre_call(call);
                if let Some(ref a) = alert {
                    tracing::warn!("[PROTECT] {}: tool={}", a.label(), call.tool);
                    protection_warnings.push(a.to_session_warning());
                }

                // Dependency check
                let mut called_tools: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for msg in self.session.messages() {
                    for block in &msg.content {
                        if let crate::session::ContentBlock::ToolUse { name, .. } = block {
                            called_tools.insert(name.clone());
                            called_tools.insert(name.clone());
                        }
                    }
                }
                let missing_deps = self.tools.check_dependencies(&call.tool, &called_tools);
                if !missing_deps.is_empty() {
                    tracing::warn!(
                        "[DEP] tool '{}' missing dependencies: {:?}",
                        call.tool,
                        missing_deps
                    );
                    protection_warnings.push(format!(
                        "依赖提示: 工具 '{}' 建议先调用 {} 再执行当前操作。",
                        call.tool,
                        missing_deps.join(", ")
                    ));
                }

                // Safety check
                if let Some(err_result) = self.check_tool_safety(call, cancel_flag).await {
                    self.emit(NuphusEvent::ToolCallEnd {
                        call_id: call.id.clone(),
                        tool_name: call.tool.clone(),
                        success: false,
                        duration_ms: 0,
                        output_preview: String::new(),
                        output_full_size: 0,
                        is_truncated: false,
                        error: err_result.error.clone(),
                        from_task: self.is_task,
                    });

                    match exec_tool::breaker_check(self.safety_consecutive_failures) {
                        exec_tool::BreakerAction::Halt => {
                            tracing::error!(
                                "[CIRCUIT-BREAKER] {} consecutive safety failures, aborting",
                                self.safety_consecutive_failures
                            );
                            self.session.strip_incomplete_tools();
                            let total_duration =
                                self.execution_started_at.elapsed().as_millis() as u64;
                            let result_msg = format!(
                                "执行中止:连续 {} 次安全检查未通过",
                                self.safety_consecutive_failures
                            );
                            if let Err(e) =
                                self.learn_from_success(total_duration, &result_msg).await
                            {
                                tracing::warn!("Failed to learn from circuit-breaker: {}", e);
                            }
                            return Ok((false, result_msg, vec![]));
                        }
                        exec_tool::BreakerAction::Warn | exec_tool::BreakerAction::Restrict => {
                            if let Some(msg) =
                                exec_tool::breaker_message(self.safety_consecutive_failures)
                            {
                                self.session.push_user(msg);
                            }
                        }
                        exec_tool::BreakerAction::None => {}
                    }

                    if let Some(ref a) = self.protection.check_post_call(call) {
                        protection_warnings.push(a.to_session_warning());
                    }
                    self.session.push_tool_result(
                        call.id.clone(),
                        err_result.error.clone().unwrap_or_default(),
                        true,
                    );
                    continue;
                }

                checked_calls.push((call.clone(), alert));
            }

            // ── Tool cache separation ──
            let mut cached_indices: std::collections::HashSet<usize> =
                std::collections::HashSet::new();
            let mut calls_to_execute: Vec<&(ToolCall, Option<ProtectionAlert>)> = Vec::new();
            for (idx, pair) in checked_calls.iter().enumerate() {
                let (call, _alert) = pair;
                let tool_name = call.tool.as_str();
                let is_cacheable = matches!(
                    tool_name,
                    "Read" | "Glob" | "Grep" | "Diff" | "get_system_info" | "get_env" | "think"
                );
                if is_cacheable {
                    let cache_hit = {
                        let mut cache = crate::cache::global_cache().lock().await;
                        cache.get(&call.tool, &call.params).is_some()
                    };
                    if cache_hit {
                        tracing::info!("[TOOL-CACHE] hit: {} params={:?}", call.tool, call.params);
                        cached_indices.insert(idx);
                        continue;
                    }
                }
                calls_to_execute.push(pair);
            }

            // Concurrent execution batch processing
            for chunk in calls_to_execute.chunks(BATCH_SIZE) {
                let mut batch_results = Vec::new();
                for (call, _) in chunk.iter() {
                    let tools = self.tools.clone();
                    let call = call.clone();
                    let start = std::time::Instant::now();
                    let result = if call.tool == "system_shell" {
                        match &self.event_emitter {
                            Some(emitter) => {
                                crate::runtime::sub_task_shell::execute_shell_streaming(
                                    &call, emitter,
                                )
                                .await
                            }
                            None => {
                                crate::runtime::sub_task_shell::execute_tool_only(
                                    &tools, &call, None,
                                )
                                .await
                            }
                        }
                    } else {
                        crate::runtime::sub_task_shell::execute_tool_only(
                            &tools,
                            &call,
                            self.event_emitter.as_deref(),
                        )
                        .await
                    };
                    let duration_ms = start.elapsed().as_millis() as u64;
                    batch_results.push((call, result, duration_ms));
                }

                for (call, result, duration_ms) in batch_results {
                    let tool_name = call.tool.as_str();
                    let is_cacheable = matches!(
                        tool_name,
                        "Read" | "Glob" | "Grep" | "Diff" | "get_system_info" | "get_env" | "think"
                    );
                    if is_cacheable && result.success {
                        let mut cache = crate::cache::global_cache().lock().await;
                        cache.set(&call.tool, &call.params, result.clone());
                    }

                    let output_str = result.output.as_deref().unwrap_or("");
                    // ToolResult::failure 只设 error、output 为空——失败时必须用 error
                    // 作为会话内容，否则 LLM 收到空错误结果，无法诊断只能盲重试
                    let output_str = if output_str.is_empty() && !result.success {
                        result.error.as_deref().unwrap_or("unknown error")
                    } else {
                        output_str
                    };
                    tracing::debug!("[SubTaskRunner] tool result: tool={}, success={}, error={:?}, output_preview={}",
                        call.tool, result.success, result.error,
                        output_str.chars().take(100).collect::<String>());

                    let preview_limit = if call.tool.starts_with("planner_") {
                        5000
                    } else {
                        2000
                    };
                    self.emit(NuphusEvent::ToolCallEnd {
                        call_id: call.id.clone(),
                        tool_name: call.tool.clone(),
                        success: result.success,
                        duration_ms,
                        output_preview: output_str.chars().take(preview_limit).collect(),
                        output_full_size: output_str.len(),
                        is_truncated: output_str.chars().count() > preview_limit,
                        error: result.error.clone(),
                        from_task: self.is_task,
                    });

                    // External content: sanitize + injection scan + untrusted boundary (unified entry)
                    let filtered = crate::filter::ToolOutputFilter::apply(&call.tool, output_str);
                    let filtered = crate::security::injection::process_external_output(
                        &call.tool,
                        Some(&call.params),
                        &filtered,
                    );
                    let truncated = crate::utils::truncate_tool_output(&filtered, 8000, &call.tool);
                    self.session
                        .push_tool_result(call.id.clone(), truncated, !result.success);

                    if !result.success {
                        if let Some(ref a) = self.protection.check_post_call(&call) {
                            protection_warnings.push(a.to_session_warning());
                        }
                    } else {
                        self.safety_consecutive_failures = 0;
                        self.protection.reset_consecutive_errors();

                        if call.tool == "Write" {
                            self.reminders.clear_by_prefix("Write");
                            if let Some(path) = call.params.get("path").and_then(|v| v.as_str()) {
                                let mut cache = crate::cache::global_cache().lock().await;
                                cache.invalidate(path);
                            }
                        }
                    }
                }
            }

            // ── Handle cache hit results ──
            for (idx, (call, alert)) in checked_calls.iter().enumerate() {
                if !cached_indices.contains(&idx) {
                    continue;
                }
                if let Some(ref a) = alert {
                    protection_warnings.push(a.to_session_warning());
                }
                let mut cache = crate::cache::global_cache().lock().await;
                let result = match cache.get(&call.tool, &call.params) {
                    Some(r) => r.clone(),
                    None => continue,
                };
                drop(cache);
                let output_str = result.output.as_deref().unwrap_or("");
                // 缓存重放的失败结果同样 output 为空——回退 error（见主路径同注）
                let output_str = if output_str.is_empty() && !result.success {
                    result.error.as_deref().unwrap_or("unknown error")
                } else {
                    output_str
                };
                self.emit(NuphusEvent::ToolCallEnd {
                    call_id: call.id.clone(),
                    tool_name: call.tool.clone(),
                    success: result.success,
                    duration_ms: 0,
                    output_preview: output_str.chars().take(2000).collect(),
                    output_full_size: output_str.len(),
                    is_truncated: output_str.len() > 2000,
                    error: result.error.clone(),
                    from_task: self.is_task,
                });

                // External content: sanitize + injection scan + untrusted boundary (unified entry)
                let output_with_warning = crate::security::injection::process_external_output(
                    &call.tool,
                    Some(&call.params),
                    output_str,
                );
                let truncated =
                    crate::utils::truncate_tool_output(&output_with_warning, 8000, &call.tool);
                self.session
                    .push_tool_result(call.id.clone(), truncated, !result.success);
                if !result.success {
                    if let Some(ref pp) = self.protection.check_post_call(call) {
                        protection_warnings.push(pp.to_session_warning());
                    }
                } else {
                    self.safety_consecutive_failures = 0;
                    self.protection.reset_consecutive_errors();
                }
            }

            // Flush protection warnings into ReminderQueue (persistent, multi-turn)
            for w in protection_warnings.drain(..) {
                self.reminders.enqueue(
                    w,
                    3,
                    ReminderPriority::High,
                    ReminderCategory::DeviationCorrect,
                );
            }
            for w in self.pending_warnings.drain(..) {
                self.reminders.enqueue(
                    w,
                    3,
                    ReminderPriority::High,
                    ReminderCategory::DeviationCorrect,
                );
            }

            // Context water level monitor (no distillation, just reminder)
            let usage = self.session.estimate_token_usage();
            let ratio = usage as f64 / self.config.context_window as f64;
            if ratio > 0.70 {
                tracing::warn!(
                    "[EXEC CONTEXT] High context usage: {}/{} ({:.1}%). \
                    Consider breaking task into smaller subtasks.",
                    usage,
                    self.config.context_window,
                    ratio * 100.0
                );
                self.reminders.enqueue(
                    format!(
                        "当前上下文使用量已达 {:.0}% ({} tokens)。如后续步骤继续失败，请精简输出或要求 Leader 拆分任务。",
                        ratio * 100.0, usage
                    ),
                    3,
                    ReminderPriority::High,
                    ReminderCategory::DeviationCorrect,
                );
            }

            let iter_duration = iter_timer.elapsed().as_millis();
            iter_timer = std::time::Instant::now();
            tracing::debug!(
                "[TIMING] iteration {} total loop = {}ms, tool_calls={}",
                iteration,
                iter_duration,
                tool_calls.len()
            );
        }

        if !self.suppress_lifecycle_events {
            self.emit(NuphusEvent::ExecutionError {
                step_index: 0,
                error: "达到最大迭代次数".to_string(),
            });
        }
        let total_duration = self.execution_started_at.elapsed().as_millis() as u64;
        let result_msg = format!(
            "达到最大迭代次数（已执行 {} 次工具调用）",
            self.tool_call_total_count
        );
        if let Err(e) = self.learn_from_success(total_duration, &result_msg).await {
            tracing::warn!("Failed to learn from iteration limit: {}", e);
        }
        Ok((false, result_msg, vec![]))
    }

    /// LLM call (with network retry)
    async fn llm_stream_retry(
        &mut self,
        cancel_flag: &AtomicBool,
    ) -> crate::Result<Vec<AssistantEvent>> {
        const MAX_RETRIES: u32 = 2;

        // Tool schemas are generated only once per dispatch, keeping bytes consistent to match API cache prefix
        let tools = {
            let t = self.cached_tools.get_or_insert_with(|| {
                let mut schemas = self.tools.get_schemas();
                let blocked = crate::agent::prompt::exec_blocked_tools();
                schemas.retain(|t| {
                    let name = t.function.name.as_str();
                    !blocked.contains(&name)
                        && !name.starts_with("desktop_")
                        && !name.starts_with("browser_")
                });
                schemas
            });
            t.clone()
        };

        // Stable prompt prefix is built only once per dispatch (without reminders)
        let base = {
            let b = self
                .cached_base_prompt
                .get_or_insert_with(|| self.step_prompt.clone());
            b.clone()
        };

        for attempt in 0..=MAX_RETRIES {
            // First push reminders as user message to keep system_prompt stable
            if let Some(rem) = self.reminders.format_for_prompt() {
                if !rem.is_empty() {
                    self.session.push_user(rem);
                }
            }
            let messages = self.session.to_api_messages(self.supports_vision);
            let request = MessageRequest::new("", messages)
                .with_system(base.clone())
                .with_tools(tools.clone());

            // ── Streaming with real-time thinking delta emission ──
            let think_depth = Arc::new(AtomicU32::new(0));
            let collected = Arc::new(std::sync::Mutex::new(Vec::new()));
            let collected_clone = collected.clone();
            let think_state = think_depth.clone();
            let exec_emitter = self.event_emitter.clone();
            let is_task = self.is_task;

            let emitter = Box::new(move |event: AssistantEvent| {
                if let AssistantEvent::TextDelta(text) = &event {
                    let (reasoning, text_clean) =
                        crate::utils::process_text_delta(text, &think_state);
                    if let Some(ref em) = exec_emitter {
                        if let Some(r) = reasoning {
                            em.emit(NuphusEvent::LlmTextDelta {
                                text: r,
                                is_thinking: true,
                                from_task: is_task,
                            });
                        }
                        if !text_clean.is_empty() {
                            em.emit(NuphusEvent::LlmTextDelta {
                                text: text_clean,
                                is_thinking: false,
                                from_task: is_task,
                            });
                        }
                    }
                }
                if let AssistantEvent::Reasoning(text) = &event {
                    // reasoning_content deltas arrive as Reasoning events (not TextDelta).
                    // Forward in real-time — 与 react_loop/workflow_agent 对齐，
                    // 否则 dispatch 的 thinking 永远不会到达执行面板。
                    if let Some(ref em) = exec_emitter {
                        em.emit(NuphusEvent::LlmTextDelta {
                            text: text.clone(),
                            is_thinking: true,
                            from_task: is_task,
                        });
                    }
                }
                collected_clone
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(event);
            });

            match self
                .llm
                .stream_with_emitter(request, cancel_flag, emitter)
                .await
            {
                Ok(()) => {
                    let events = collected.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    if events
                        .iter()
                        .any(|e| matches!(e, AssistantEvent::Cancelled))
                    {
                        return Err(crate::NuphusError::LLM(crate::LLMError::Cancelled));
                    }
                    return Ok(events);
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if attempt == MAX_RETRIES || !crate::agent::common::is_network_error(&err_str) {
                        return Err(e);
                    }
                    tracing::info!(
                        "LLM network error, retry {}/{}: {}",
                        attempt + 1,
                        MAX_RETRIES + 1,
                        err_str,
                    );
                    self.emit(NuphusEvent::Warning {
                        code: "llm_network_retry".to_string(),
                        message: format!(
                            "Network connection timeout, retrying ({}/{})",
                            attempt + 1,
                            MAX_RETRIES + 1,
                        ),
                    });
                    tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
                }
            }
        }
        // All paths inside the loop return, but this is a defense-in-depth fallback
        Err(crate::NuphusError::LLM(
            crate::LLMError::RetryLoopExhausted {
                last_error:
                    "LLM stream retry loop exhausted without returning — unexpected control flow"
                        .to_string(),
            },
        ))
    }

    /// Inject progressive warnings based on current iteration progress
    fn inject_iteration_warning(&mut self, iteration: usize) -> bool {
        let max_iter = self.config.max_iterations.max(1);
        let ratio = iteration as f64 / max_iter as f64;

        let level = if ratio >= Self::WARN_FORBID {
            4
        } else if ratio >= Self::WARN_REDLINE {
            3
        } else if ratio >= Self::WARN_EMPHASIS {
            2
        } else if ratio >= Self::WARN_REMIND {
            1
        } else {
            0
        };

        if level <= self.max_warning_injected {
            return false;
        }
        self.max_warning_injected = level;

        match level {
            1 => self.reminders.enqueue(
                format!(
                    "已迭代 {} 次。如已收集足够信息请停止探索，输出目标结果。",
                    iteration
                ),
                2,
                ReminderPriority::Normal,
                ReminderCategory::DeviationCorrect,
            ),
            2 => self.reminders.enqueue(
                format!(
                    "已迭代 {} 次。复杂任务可后续分轮处理，请输出当前结果。",
                    iteration
                ),
                2,
                ReminderPriority::High,
                ReminderCategory::DeviationCorrect,
            ),
            3 => self.reminders.enqueue(
                "已达到迭代临界点，必须输出最终结果，否则强制终止。".to_string(),
                2,
                ReminderPriority::Critical,
                ReminderCategory::DeviationCorrect,
            ),
            4 => {
                self.reminders.enqueue(
                    "已达到迭代上限，系统将关闭工具调用，立即产出回复。".to_string(),
                    2,
                    ReminderPriority::Critical,
                    ReminderCategory::DeviationCorrect,
                );
            }
            _ => {}
        }
        level >= 4
    }

    /// Process streaming events and emit TokenUsage
    fn process_events(&mut self, events: Vec<AssistantEvent>) -> Vec<ContentBlock> {
        let content_tool_tags = crate::config::registry::ProviderRegistry::builtin()
            .get(self.llm.provider_kind().as_str())
            .map(|p| p.quirks().content_tool_tags)
            .unwrap_or(&[]);
        let result = crate::agent::common::process_events(events, content_tool_tags);
        if let Some((input, output)) = &result.usage {
            self.session.update_api_input_tokens(*input as u64);
            self.emit(NuphusEvent::TokenUsage {
                input_tokens: *input,
                output_tokens: *output,
                cache_hit_tokens: result.cache_hit_tokens,
                source: "exec".to_string(),
            });
        }
        result.blocks
    }

    /// Learn from successful execution
    async fn learn_from_success(
        &mut self,
        _total_duration_ms: u64,
        result_msg: &str,
    ) -> crate::Result<()> {
        use crate::memory::entry::PersistedStep;

        let mut steps: Vec<PersistedStep> = Vec::new();
        let mut step_number: u32 = 0;
        let mut tools_used: Vec<String> = Vec::new();

        for msg in self.session.messages() {
            for block in &msg.content {
                if let crate::session::ContentBlock::ToolUse { id, name, input } = block {
                    step_number += 1;
                    if !tools_used.contains(name) {
                        tools_used.push(name.clone());
                    }
                    let result_content = self.find_tool_result(id);
                    // 紧凑轨迹：params/result 摘要化（构造时即截断 120/300）
                    steps.push(PersistedStep::new(
                        name.clone(),
                        input,
                        result_content.as_deref(),
                        result_content.is_some(),
                        None,
                    ));
                }
            }
        }

        let tools_desc = tools_used.join(", ");

        // Skip memory storage only when zero steps (no tool calls)
        if step_number == 0 {
            tracing::debug!("learn_from_success: skipped (no tool calls)");
            return Ok(());
        }

        let use_session_id = self
            .leader_session_id
            .as_deref()
            .unwrap_or(&self.session.id);
        let use_turn_id = self.leader_turn_id.as_deref().unwrap_or(&self.session.id);
        // kind=task_trace：紧凑步骤（≤20 步）+ output 截断 2000，在 entry_from_exec_steps 内完成
        let mut entry = crate::memory::entry::entry_from_exec_steps(
            use_session_id,
            use_turn_id,
            self.tool_call_total_count,
            &steps,
            tools_used,
            true,
            None,
            self.goal_type.as_ref().map(|g| g.id().to_string()),
            None,
        );
        entry.intent = crate::memory::entry::truncate(&self.user_message, 100);
        entry.user_message = crate::memory::entry::truncate(&self.user_message, 2000);
        // assistant_message: 用 ExecAgent 实际返回的 result_msg，而非工具执行摘要
        entry.assistant_message = crate::memory::entry::truncate(result_msg, 2000);
        // EXEC_SUMMARY_PRESERVE: preserve semantic summary from entry_from_exec_steps
        if entry.summary.is_empty() {
            entry.summary = format!("Executed {} tools: {}", step_number, tools_desc);
        }

        if let Err(e) = crate::store::memory::insert_entry(&entry) {
            tracing::warn!("Failed to save exec success entry: {}", e);
        }

        tracing::info!(
            "learn_from_success: recorded {} tool steps from session {}",
            entry.tools_used.len(),
            entry.session_id
        );

        Ok(())
    }
}
