//! 步骤调度与分发
//!
//! Recursive step executor — 主入口分发到各步骤处理器。
use super::*;

// ── Recursive step executor ──

impl Executor {
    /// Sleep helper — delegates to system_sleep tool via tool_exec
    async fn execute_sleep_step(&self, seconds: f64) -> crate::Result<String> {
        tokio::time::sleep(std::time::Duration::from_secs_f64(seconds)).await;
        Ok(format!("slept {}s", seconds))
    }

    /// Recursively execute a Step tree (main entry point)
    #[async_recursion::async_recursion]
    pub async fn execute_step<F, Fut>(
        &self,
        step: &Step,
        depth: u32,
        store: &WorkflowStore,
        events: &EventBus,
        tool_exec: &F,
        variables: &mut HashMap<String, serde_json::Value>,
        workflow_id: &str,
        llm: Option<&dyn ApiClient>,
        emitter: Option<&dyn EventEmitter>,
        tool_schemas: Option<&[ToolDefinition]>,
        completed_ids: &std::collections::HashSet<String>,
        run_record: &mut RunRecord,
    ) -> crate::Result<String>
    where
        F: Fn(String, serde_json::Value) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = std::result::Result<String, String>> + Send,
    {
        let step_id = step.id();
        if completed_ids.contains(&step_id) {
            tracing::info!(
                "[executor] Skipping completed step: {} ({})",
                step_id,
                step.name()
            );
            return Ok(format!("step_skipped:{}", step_id));
        }

        // ── 生命周期控制：每步执行前检查取消/暂停信号 ──
        self.check_cancel(workflow_id).await?;
        self.check_pause(
            workflow_id,
            events,
            &step.id(),
            &step.name(),
            depth,
            step.kind_str(),
            Some(store),
            Some(variables),
        )
        .await?;

        // ── HUD: step entry ──
        if let Some(emitter) = emitter {
            emitter.emit(NuphusEvent::HudUpdate {
                text: step.name(),
                phase: "workflow".into(),
                step_kind: Some(step.kind_str().to_string()),
            });
        }

        // ── 事件：步骤开始 ──
        events.emit(WorkflowEvent::StepRunStarted {
            step_id: step.id(),
            step_name: step.name(),
            depth,
            kind: step.kind_str().to_string(),
        });

        // ── 分发到各步骤处理器 ──
        let started_at = chrono::Utc::now();
        let recorder = crate::workflow::trace::current();
        let invocation_id = if let Some(recorder) = &recorder {
            let inputs = match &step.action {
                Action::Tool { with, .. } | Action::Call { with, .. } => {
                    Self::resolve_vars(with, variables)
                }
                Action::Mcp { mcp } => Self::resolve_vars(&mcp.with, variables),
                Action::Script { script } => {
                    serde_json::json!({"runtime":script.runtime, "code":super::variables::resolve_vars_str(&script.code, variables), "cwd":script.cwd.as_ref().map(|cwd| super::variables::resolve_vars_str(cwd, variables))})
                }
                Action::Chat { chat, with } => {
                    serde_json::json!({"message":super::variables::resolve_vars_str(chat, variables), "options":with})
                }
                Action::Wait { wait, .. } => {
                    serde_json::json!({"message":super::variables::resolve_vars_str(wait, variables)})
                }
                Action::Seq { .. } => serde_json::json!({}),
                Action::If { def } => serde_json::json!({"condition":def.condition}),
                Action::Loop { def } => {
                    serde_json::json!({"for_each":def.for_each, "repeat":def.repeat, "until":def.until, "max":def.max})
                }
                action => serde_json::to_value(action).unwrap_or_default(),
            };
            Some(recorder.begin(workflow_id, step, inputs, variables).await)
        } else {
            None
        };
        let result = if let Err(error) =
            crate::workflow::references::validate_step_fields(step, variables)
        {
            Err(crate::NuphusError::agent(error))
        } else {
            match &step.action {
                Action::Tool { tool, with } => {
                    self.execute_tool_step(
                        step,
                        tool,
                        with,
                        tool_exec,
                        variables,
                        workflow_id,
                        events,
                        llm,
                        emitter,
                    )
                    .await
                }
                Action::Seq { seq } => {
                    self.execute_seq_step(
                        step,
                        seq,
                        depth,
                        store,
                        events,
                        tool_exec,
                        variables,
                        workflow_id,
                        llm,
                        emitter,
                        tool_schemas,
                        completed_ids,
                        run_record,
                    )
                    .await
                }
                Action::Loop { def } => {
                    self.execute_loop_step(
                        step,
                        def,
                        depth,
                        store,
                        events,
                        tool_exec,
                        variables,
                        workflow_id,
                        llm,
                        emitter,
                        tool_schemas,
                        completed_ids,
                        run_record,
                    )
                    .await
                }
                Action::If { def } => {
                    self.execute_if_step(
                        step,
                        def,
                        depth,
                        store,
                        events,
                        tool_exec,
                        variables,
                        workflow_id,
                        llm,
                        emitter,
                        tool_schemas,
                        completed_ids,
                        run_record,
                    )
                    .await
                }
                Action::Call { call, with } => {
                    self.execute_call_step(
                        step,
                        call,
                        with,
                        depth,
                        store,
                        events,
                        tool_exec,
                        variables,
                        llm,
                        emitter,
                        tool_schemas,
                        completed_ids,
                        run_record,
                    )
                    .await
                }
                Action::Wait { wait, auto } => {
                    // 与其它 action 对齐：提示语先做 {{var}} 模板替换再展示。
                    // chat（step_chat_agent）/ script / tool / mcp 都走 resolve_vars_str，
                    // 只有 wait 漏了 —— 于是提示语把占位符原样显示出来。实测 HUD 上是
                    // "等待: 已读取到 {{pending_raw}}…"：用户既看不出在等什么，也看不到
                    // 该步骤本想汇报的内容（如待学课程清单）。
                    //
                    // 注：`auto` 子步骤与取消/恢复语义均不受影响；判据仍是替换后的文案
                    // 是否为空，与其它 action 的"先 resolve 再使用"保持一致。
                    let resolved_wait = super::variables::resolve_vars_str(wait, variables);
                    self.execute_wait_step(
                        step,
                        &resolved_wait,
                        auto,
                        depth,
                        store,
                        events,
                        tool_exec,
                        variables,
                        workflow_id,
                        llm,
                        emitter,
                        tool_schemas,
                        completed_ids,
                        run_record,
                    )
                    .await
                }
                Action::Chat { chat, with: opts } => {
                    self.execute_chat_step(
                        step,
                        chat,
                        opts,
                        variables,
                        llm,
                        emitter,
                        tool_exec,
                        workflow_id,
                        tool_schemas,
                    )
                    .await
                }
                Action::Script { script } => {
                    self.execute_script_step(step, script, variables).await
                }
                Action::Assert { assert } => self.execute_assert_step(assert, variables).await,
                Action::Mcp { mcp } => self.execute_mcp_step(step, mcp, variables).await,
                Action::Sleep { sleep } => self.execute_sleep_step(*sleep).await,
                Action::Break { .. } => Ok("break".to_string()),
                Action::Continue { .. } => Ok("continue".to_string()),
                Action::Custom(_) => Err(crate::NuphusError::agent(
                    "custom step kind not yet supported".to_string(),
                )),
            }
        };

        if let (Some(recorder), Some(id)) = (&recorder, invocation_id) {
            let verification = if matches!(&step.action, Action::Assert { .. }) {
                Some(serde_json::json!({"kind":"assert", "passed":result.is_ok()}))
            } else {
                result
                    .as_ref()
                    .ok()
                    .and_then(|output| crate::workflow::trace::verification(output))
            };
            let trace_result = result.as_deref().map_err(|error| error.to_string());
            recorder
                .finish(
                    id,
                    variables,
                    trace_result
                        .as_ref()
                        .map(|value| *value)
                        .map_err(|error| error.as_str()),
                    verification,
                )
                .await;
        }

        let safe_output = match &result {
            Ok(message) => Some(self.redact_run_text(workflow_id, message).await),
            Err(_) => None,
        };
        let safe_error = match &result {
            Ok(_) => None,
            Err(error) => Some(self.redact_run_text(workflow_id, &error.to_string()).await),
        };

        // ── 记录步骤执行结果到 RunRecord（断点续连 / has_skipped / completed_steps 数据源）──
        {
            let finished_at = chrono::Utc::now();
            let record = match &result {
                Ok(_) => StepRunRecord {
                    step_id: step_id.clone(),
                    started_at,
                    finished_at: Some(finished_at),
                    status: StepRunStatus::Success,
                    output_summary: safe_output
                        .as_deref()
                        .map(|message| message.chars().take(200).collect()),
                },
                Err(_) => StepRunRecord {
                    step_id: step_id.clone(),
                    started_at,
                    finished_at: Some(finished_at),
                    status: StepRunStatus::Error(safe_error.clone().unwrap_or_default()),
                    output_summary: None,
                },
            };
            run_record.steps.push(record);
        }

        // ── 事件：步骤完成 ──
        // 成功 → StepRunCompleted{Success}；失败 → Error（message 横幅）+ StepRunCompleted{Error}
        // （补发后者：前端据此把该步骤标记为 failed 红叉；旧实现只发 Error 无 step_id，
        // 前端无法定位失败步骤，导致 run_completed 时失败步骤被误收敛为绿色 completed）。
        match &result {
            Ok(_) => {
                if let Some(message) = safe_output.filter(|message| !message.is_empty()) {
                    events.emit(WorkflowEvent::StepRunOutput {
                        step_id: step.id(),
                        text: message.chars().take(200).collect(),
                    });
                }
                events.emit(WorkflowEvent::StepRunCompleted {
                    step_id: step.id(),
                    step_name: step.name(),
                    status: StepRunStatus::Success,
                    depth,
                });
            }
            Err(_) => {
                let message = safe_error.unwrap_or_else(|| "执行失败".to_string());
                events.emit(WorkflowEvent::Error {
                    message: format!("Step '{}' failed: {}", step.name(), message),
                });
                events.emit(WorkflowEvent::StepRunCompleted {
                    step_id: step.id(),
                    step_name: step.name(),
                    status: StepRunStatus::Error(message),
                    depth,
                });
            }
        }

        if result.is_ok() {
            if let Some(session) = crate::workflow::debug::current() {
                if session.should_break(workflow_id, &step.id) {
                    self.pause(&session.workflow_id).await;
                    session.recorder.status("paused").await;
                    events.emit(WorkflowEvent::StepRunPaused {
                        step_id: step.id.clone(),
                        step_name: step.name.clone(),
                        reason: "debug_after_step".into(),
                    });
                    let notify = self
                        .pause_notifies
                        .read()
                        .await
                        .get(&session.workflow_id)
                        .cloned();
                    if let Some(notify) = notify {
                        notify.notified().await;
                    }
                    self.check_cancel(&session.workflow_id).await?;
                    session.recorder.status("running").await;
                }
            }
        }
        result
    }
}
