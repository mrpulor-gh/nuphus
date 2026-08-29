//! dispatch — actual handling of task_dispatch tool calls
//!
//! When ReactAgent calls task_dispatch, the main loop intercepts the call and routes to `handle_task_dispatch`,
//! It builds the Exec prompt and launches ExecuteAgent to run subtasks.
//!
//! Migrated from agent/leader/dispatch.rs, now a free function receiving &mut ReactAgent.

use crate::agent::events::NuphusEvent;
use crate::agent::goal_types::{self, GoalType};
use crate::agent::prompt;
use crate::agent::ReactAgent;
use crate::runtime::SubTaskRunner;
use crate::{ExecutionStep, ToolCall, ToolResult};
use std::sync::atomic::AtomicBool;

/// Handle task_dispatch — create ExecuteAgent to run subtask and return result
///
/// About plan.md auto-injection: if context parameter is an existing .plan.md file path
/// (e.g. 'src-tauri/tasks/nuphus/plans/XX.plan.md'), the system automatically reads and injects its content.
///
/// Return structure:
/// ```json
/// {
///   "status": "success|failed",
///   "task": "task description",
///   "summary": "Exec LLM final text output (full content, no truncation)"
/// }
/// ```
/// `summary` is the final text output from Exec LLM after all tools execute, it is Exec's answer delivery to Leader.
pub(crate) async fn handle_task_dispatch(
    agent: &mut ReactAgent,
    call: &ToolCall,
    cancel_flag: &AtomicBool,
    post_warnings: &mut Vec<String>,
) -> crate::Result<(ToolResult, GoalType, Vec<ExecutionStep>)> {
    let description = call
        .params
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let goal_type_str = call
        .params
        .get("goal_type")
        .and_then(|v| v.as_str())
        .unwrap_or("file_operation");
    let goal_type = GoalType::from_id(goal_type_str).unwrap_or(GoalType::FileOperation);
    let explicit_plan_path = call
        .params
        .get("plan_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let task_id = call
        .params
        .get("task_id")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as usize;
    let total_tasks = call
        .params
        .get("total_tasks")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as usize;

    if description.is_empty() {
        return Ok((
            ToolResult::failure("task_dispatch missing description parameter"),
            goal_type,
            vec![],
        ));
    }

    // Generate task chain ID
    let task_chain_id = uuid::Uuid::new_v4().to_string();

    // Need exec_tools + exec_llm to dispatch
    let exec_tools = agent.exec_tools.as_ref().cloned().ok_or_else(|| {
        crate::NuphusError::Tool(
            "task_dispatch unavailable: execution resources not configured".to_string(),
        )
    })?;
    let exec_llm = agent.exec_llm.as_ref().cloned().ok_or_else(|| {
        crate::NuphusError::Tool("task_dispatch unavailable: LLM not configured".to_string())
    })?;

    // Build execution prompt using goal_type specified at dispatch
    let model_label = format!("{} ({})", agent.config.model, agent.config.provider);
    let tool_schemas = exec_tools.render_tools_for_prompt();

    // Assemble execution context block
    let current_turn = agent.session.current_turn_id();
    let mut exec_context = String::new();
    exec_context.push_str(&format!(
        "## 执行上下文\n\
         - 任务链ID: {}\n\
         - 链步骤号: 1\n\
         - 来源对话 turn_id: {}\n\
         - 来源 session_id: {}\n",
        task_chain_id, current_turn, agent.session.id
    ));

    // Plan file injection: read and inject content from plan_path parameter
    let mut plan_path: Option<&str> = None;
    if let Some(p) = explicit_plan_path {
        let path = std::path::Path::new(p);
        if path.exists() {
            plan_path = Some(p);
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    let focused = extract_focused_plan_context(&content, task_id, description);
                    exec_context.push_str(&format!("\n### 计划文档（分层注入）\n{}\n", focused));
                }
                Err(e) => {
                    tracing::warn!("[dispatch] 无法读取计划文档 {}: {}", p, e);
                }
            }
        } else {
            tracing::warn!("[dispatch] plan 文件不存在: {}", p);
        }
    }

    // Mechanized: if plan document is used, remind Exec this is a reference document for understanding, not an instruction checklist
    if let Some(path) = plan_path {
        exec_context.push_str(&format!(
            "\n### 计划文档执行提醒\n\
             你正在执行计划文档中的任务（{}）。\n\
             注意：这份计划是 Leader 传递的**理解文档**，不是必须按步骤执行的指令清单。\n\
             你应基于[现状理解]和[当前方向]的约束与建议，**自主决策**最佳执行路径。\n\
             完成后按「最终交付要求」输出结构化摘要，并附加 plan_update JSON。\n",
            path
        ));
    }

    // Memory retrieval injection: inject relevant experience into exec_context
    // bm25 返回负值（越小越好）：以最佳命中为基准归一化相关度百分比。
    // （历史 bug：曾用 score > 0.0 过滤，bm25 恒为负 → lessons 永远为空，注入失效）
    let lessons: Vec<String> =
        match crate::store::memory::search_entries_scored(description, 5, None, false, true) {
            Ok(entries) => {
                let best = entries.first().map(|(_, s)| *s).unwrap_or(0.0);
                entries
                    .into_iter()
                    .filter(|(_, score)| score.is_finite() && *score < 0.0)
                    .map(|(entry, score)| {
                        let rel = if best < 0.0 { score / best } else { 0.0 };
                        let pct = (rel * 100.0).clamp(0.0, 100.0) as u32;
                        format!("[相关度 {}%] {}: {}", pct, entry.intent, entry.summary)
                    })
                    .collect()
            }
            Err(e) => {
                tracing::warn!("[dispatch] memory search for lessons failed: {}", e);
                vec![]
            }
        };

    if !lessons.is_empty() {
        let lessons_text = lessons.join("\n---\n");
        exec_context.push_str(&format!("\n### 相关经验\n{}\n", lessons_text));
    }

    let system_prompt = prompt::build_exec_prompt(
        &model_label,
        &tool_schemas,
        goal_type,
        &agent.leader_ctx.soul,
        agent.leader_ctx.relation.as_ref(),
        agent.config.supports_vision,
        agent.config.vision_model.as_deref(),
    );

    // Build dynamic user message: task + context (kept outside system prompt to maximize cache hit)
    let mut dynamic_msg = String::new();
    dynamic_msg.push_str(&format!("## 任务\n{}\n", description));
    if !exec_context.is_empty() {
        dynamic_msg.push_str(&format!("\n## 参考上下文\n{}", exec_context));
    }

    // P2 — Exec Agent pool reuse: same turn shares raw session, cross-turn isolates
    let pool_key = plan_path.unwrap_or("").to_string();
    let reuse_key = if pool_key.is_empty() {
        format!("adhoc:{}:{}", goal_type_str, current_turn)
    } else {
        format!("{}:{}", pool_key, current_turn)
    };

    let mut exec_agent = if let Some(agent_ref) = agent.exec_agent_pool.remove(&reuse_key) {
        tracing::info!(
            "[ExecPool] 复用 Exec agent (key={}, task={})",
            reuse_key,
            description
        );
        let mut agent_ref = agent_ref;
        agent_ref.prepare_for_next_task(description.to_string());
        agent_ref.set_tool_permissions(crate::permissions::ToolPermissions::all());
        if let Some(ref pause_flag) = agent.pause_flag {
            agent_ref.set_pause_flag(pause_flag.clone());
        }
        // ── 上下文压缩：用前序任务回复替换原始 session ──
        if agent_ref.needs_compress {
            let replies = agent_ref.turn_replies.join("\n\n---\n\n");
            let compressed = format!("## 同 turn 前序任务结果\n\n{}", replies);
            agent_ref.session.replace_with_distill(&compressed);
            agent_ref.needs_compress = false;
            tracing::info!(
                "[ExecPool] 压缩完成: turn_replies={}, compressed={} chars",
                agent_ref.turn_replies.len(),
                compressed.len()
            );
        }
        agent_ref.session.push_user(dynamic_msg);
        agent_ref
    } else {
        tracing::info!(
            "[ExecPool] 新建 Exec agent (key={}, task={})",
            reuse_key,
            description
        );
        let mut new_agent = SubTaskRunner::new_free(
            exec_llm.clone(),
            exec_tools,
            system_prompt,
            description.to_string(),
        );
        new_agent.session.push_user(dynamic_msg);
        new_agent.set_context_window(goal_types::get_context_window(exec_llm.model_name()));
        new_agent.set_goal_type(goal_type);
        // Inject goal-type warmup reminders (subtask quality guardrails)
        let warmups = goal_types::get_warmup_reminders(goal_type);
        for w in warmups {
            new_agent.reminders.enqueue(
                w,
                3, // max_deliveries: show for first 3 iterations
                crate::agent::reminders::ReminderPriority::High,
                crate::agent::reminders::ReminderCategory::DeviationCorrect,
            );
        }
        new_agent.set_tool_permissions(crate::permissions::ToolPermissions::all());
        if let Some(ref emitter) = agent.exec_emitter {
            new_agent.set_event_emitter(emitter.clone());
        }
        new_agent.set_suppress_lifecycle_events(true);
        new_agent.set_task_mode(true);
        if let Some(ref pause_flag) = agent.pause_flag {
            new_agent.set_pause_flag(pause_flag.clone());
        }
        new_agent
    };

    // Set leader session/turn for causal chain linking in memory entries
    exec_agent.leader_session_id = Some(agent.session.id.clone());
    exec_agent.leader_turn_id = Some(agent.session.current_turn_id());

    // Send TaskStarted event
    if let Some(ref emitter) = agent.exec_emitter {
        emitter.emit(NuphusEvent::TaskStarted {
            task_id,
            total_tasks,
            description: description.to_string(),
        });
    }

    let run_result = exec_agent.run_free(cancel_flag).await;

    // Propagate user termination signal to Leader via shared cancel_flag
    if exec_agent.user_terminated {
        cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        tracing::info!("[dispatch] user terminated subtask, setting cancel_flag for Leader");
    }

    let (run_success, summary, exec_steps) = match run_result {
        Ok((ok, msg, steps)) => (ok, msg, steps),
        Err(e) => {
            if let Some(ref emitter) = agent.exec_emitter {
                emitter.emit(NuphusEvent::TaskCompleted {
                    task_id,
                    total_tasks,
                    success: false,
                    description: description.to_string(),
                    summary: format!("Executor error: {}", e).chars().take(300).collect(),
                });
            }
            // Return to pool on error (consistent with pre-compress behavior)
            agent.exec_agent_pool.insert(reuse_key, exec_agent);
            return Ok((
                ToolResult::failure(
                    serde_json::to_string(&serde_json::json!({
                        "status": "error",
                        "summary": format!("{}", e)
                    }))
                    .unwrap_or_else(|_| format!("Executor error:{}", e)),
                ),
                goal_type,
                vec![],
            ));
        }
    };

    // ── 收集同 turn 回复 + 上下文压缩检查 ──
    exec_agent.turn_replies.push(summary.clone());
    let cw = goal_types::get_context_window(exec_llm.model_name());
    let usage = exec_agent.session.estimate_token_usage();
    let compress_threshold = (cw as f64 * 0.50) as usize;
    if usage >= compress_threshold && usage >= 300_000 {
        exec_agent.needs_compress = true;
        tracing::info!(
            "[ExecPool] 标记压缩: turn_replies={}, usage={}, threshold={}",
            exec_agent.turn_replies.len(),
            usage,
            compress_threshold
        );
    }

    // P2 — Exec returns to pool, same-turn tasks share raw session
    agent.exec_agent_pool.insert(reuse_key, exec_agent);

    // Check if dispatch subtask was aborted due to circuit breaker
    if !run_success && summary.contains("安全检查未通过") {
        let reminder = "task_dispatch terminated due to consecutive security permission check failures, please re-evaluate the cause and switch strategy, do not continue using task_dispatch for this task.".to_string();
        post_warnings.push(reminder);
        tracing::warn!(
            "[dispatch] circuit breaker triggered, leader instructed to switch strategy"
        );
    }

    // Attempt to extract plan_update JSON from summary
    let plan_update = extract_plan_update_from_summary(&summary)
        .or_else(|| build_plan_update_from_steps(&exec_steps));

    let mut result = serde_json::json!({
        "status": if run_success { "success" } else { "failed" },
        "summary": summary,
    });
    if let Some(pu) = plan_update {
        result["plan_update"] = pu;
    }

    // Send TaskCompleted event
    if let Some(ref emitter) = agent.exec_emitter {
        emitter.emit(NuphusEvent::TaskCompleted {
            task_id,
            total_tasks,
            success: run_success,
            description: description.to_string(),
            summary: summary.chars().take(300).collect::<String>(),
        });
    }

    let result_str = serde_json::to_string(&result).unwrap_or(summary);
    if run_success {
        Ok((ToolResult::success(result_str), goal_type, exec_steps))
    } else {
        Ok((ToolResult::failure(result_str), goal_type, exec_steps))
    }
}

// ═══════════════════════════════════════════════════════════════
// Plan document layered injection
// ═══════════════════════════════════════════════════════════════

fn extract_focused_plan_context(content: &str, task_id: usize, description: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut output = String::new();

    let mut goal_lines = Vec::new();
    let mut in_goal = false;
    for line in &lines {
        let trimmed = line.trim();
        if trimmed == "## 验收" {
            in_goal = true;
            continue;
        }
        if in_goal {
            if trimmed == "---" {
                break;
            }
            if !trimmed.starts_with("## ") {
                goal_lines.push(*line);
            }
        }
    }
    let goal_text = goal_lines.join("\n").trim().to_string();
    if !goal_text.is_empty() {
        output.push_str("## 目标与成功标准\n");
        output.push_str(&goal_text);
        output.push_str("\n\n");
    }

    let mut context_lines = Vec::new();
    let mut in_context = false;
    for line in &lines {
        let trimmed = line.trim();
        if trimmed == "## 现状理解（核心）" || trimmed == "## 现状理解" {
            in_context = true;
            continue;
        }
        if in_context {
            if trimmed == "---" {
                break;
            }
            if !trimmed.starts_with("## ") {
                context_lines.push(*line);
            }
        }
    }
    let context_text = context_lines.join("\n").trim().to_string();
    if !context_text.is_empty() {
        output.push_str("## 现状理解（核心）\n");
        output.push_str(&context_text);
        output.push_str("\n\n");
    }

    let task_header_prefixes = [
        format!("### 方向{}:", task_id),
        format!("### Task-{}:", task_id),
        format!("### task-{}:", task_id),
    ];
    let mut task_lines = Vec::new();
    let mut in_task = false;
    for line in &lines {
        let trimmed = line.trim();
        if task_header_prefixes.iter().any(|p| trimmed.starts_with(p)) {
            in_task = true;
            if let Some((_prefix, name)) = trimmed.split_once(':') {
                output.push_str(&format!("## 当前执行方向{}: {}\n", task_id, name.trim()));
            }
            continue;
        }
        if in_task {
            if trimmed.starts_with("### ")
                || (trimmed.starts_with("## ") && !trimmed.starts_with("### "))
                || trimmed == "---"
            {
                break;
            }
            task_lines.push(*line);
        }
    }
    let task_text = task_lines.join("\n").trim().to_string();
    if !task_text.is_empty() {
        output.push_str(&task_text);
        output.push('\n');
    } else {
        output.push_str("## 当前执行任务\n");
        output.push_str(description);
        output.push('\n');
    }

    output
}

// ═══════════════════════════════════════════════════════════════
// plan_update extraction helpers
// ═══════════════════════════════════════════════════════════════

fn extract_plan_update_from_summary(summary: &str) -> Option<serde_json::Value> {
    let mut in_json_block = false;
    let mut json_buffer = String::new();
    for line in summary.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```json") || trimmed == "```" {
            if in_json_block && !json_buffer.is_empty() {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_buffer) {
                    if parsed.get("plan_update").is_some() || parsed.get("tasks").is_some() {
                        return Some(parsed);
                    }
                }
                json_buffer.clear();
            }
            in_json_block = !in_json_block;
            continue;
        }
        if in_json_block {
            json_buffer.push_str(line);
            json_buffer.push('\n');
        }
    }
    if let Some(idx) = summary.find("\"plan_update\"") {
        let start = summary[..idx].rfind('{').unwrap_or(idx);
        let end = summary[idx..]
            .find('}')
            .map(|i| idx + i + 1)
            .unwrap_or(summary.len());
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&summary[start..end]) {
            if parsed.get("plan_update").is_some() {
                return Some(parsed);
            }
        }
    }
    None
}

fn build_plan_update_from_steps(steps: &[ExecutionStep]) -> Option<serde_json::Value> {
    if steps.is_empty() {
        return None;
    }
    let tasks = serde_json::json!([
        {
            "task_id": 1,
            "acceptance": steps.iter().enumerate().map(|(i, s)| {
                serde_json::json!({
                    "index": i,
                    "passed": s.status == crate::StepStatus::Success,
                    "note": if s.status == crate::StepStatus::Success {
                        format!("✓ {} 执行成功", s.tool)
                    } else {
                        format!("✗ {} 执行失败", s.tool)
                    }
                })
            }).collect::<Vec<_>>(),
            "audit_note": format!("Exec 执行了 {} 步工具调用", steps.len())
        }
    ]);
    Some(serde_json::json!({ "tasks": tasks }))
}
