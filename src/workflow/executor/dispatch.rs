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
        let result = match &step.action {
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
                self.execute_wait_step(
                    step,
                    wait,
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
            Action::Script { script } => self.execute_script_step(step, script, variables).await,
            Action::Assert { assert } => self.execute_assert_step(assert, variables).await,
            Action::Mcp { mcp } => self.execute_mcp_step(step, mcp, variables).await,
            Action::Sleep { sleep } => self.execute_sleep_step(*sleep).await,
            Action::Break { .. } => Ok("break".to_string()),
            Action::Continue { .. } => Ok("continue".to_string()),
            Action::Custom(_) => Err(crate::NuphusError::agent(
                "custom step kind not yet supported".to_string(),
            )),
        };

        // ── 记录步骤执行结果到 RunRecord（断点续连 / has_skipped / completed_steps 数据源）──
        {
            let finished_at = chrono::Utc::now();
            let record = match &result {
                Ok(msg) => StepRunRecord {
                    step_id: step_id.clone(),
                    started_at,
                    finished_at: Some(finished_at),
                    status: StepRunStatus::Success,
                    output_summary: Some(msg.chars().take(200).collect()),
                },
                Err(e) => StepRunRecord {
                    step_id: step_id.clone(),
                    started_at,
                    finished_at: Some(finished_at),
                    status: StepRunStatus::Error(e.to_string()),
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
                events.emit(WorkflowEvent::StepRunCompleted {
                    step_id: step.id(),
                    step_name: step.name(),
                    status: StepRunStatus::Success,
                    depth,
                });
            }
            Err(e) => {
                events.emit(WorkflowEvent::Error {
                    message: format!("Step '{}' failed: {}", step.name(), e),
                });
                events.emit(WorkflowEvent::StepRunCompleted {
                    step_id: step.id(),
                    step_name: step.name(),
                    status: StepRunStatus::Error(e.to_string()),
                    depth,
                });
            }
        }

        result
    }
}
