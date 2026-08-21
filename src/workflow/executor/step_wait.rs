//! 等待人工/模型介入步骤
use super::*;

impl Executor {
    /// 等待人工/模型介入步骤
    pub(super) async fn execute_wait_step<F, Fut>(
        &self,
        step: &Step,
        prompt: &str,
        auto: &[Step],
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
        // Wait for user confirmation (HUD pause/resume)
        // 如果 prompt 非空，则暂停执行直到用户通过 HUD 点击"继续"
        if !prompt.is_empty() {
            // 如果尚未暂停（HUD 未主动暂停），则创建暂停通知器
            let already_paused = self.pause_notifies.read().await.contains_key(workflow_id);
            if !already_paused {
                let notify = Arc::new(tokio::sync::Notify::new());
                self.pause_notifies
                    .write()
                    .await
                    .insert(workflow_id.to_string(), notify);
            }

            // Emit HUD: 显示等待提示
            if let Some(emitter) = emitter {
                emitter.emit(NuphusEvent::HudUpdate {
                    text: format!("等待: {}", prompt),
                    phase: "workflow_wait".into(),
                    step_kind: Some("wait".to_string()),
                });
            }
            events.emit(WorkflowEvent::StepRunPaused {
                step_id: step.id.clone(),
                step_name: step.name.clone(),
                reason: prompt.to_string(),
            });

            // 轮询等待：直到用户点击 HUD"继续"（resume 移除通知器）或取消
            let start = std::time::Instant::now();
            loop {
                // 检查取消
                self.check_cancel(workflow_id).await?;

                // 检查是否恢复
                if !self.pause_notifies.read().await.contains_key(workflow_id) {
                    break;
                }

                // 超时保护（最长等待 30 分钟）
                if start.elapsed() > std::time::Duration::from_secs(1800) {
                    return Err(crate::NuphusError::agent(format!(
                        "Wait step '{}': timeout after 30min",
                        step.name
                    )));
                }

                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }

            // 恢复后重新发送 started 事件
            events.emit(WorkflowEvent::StepRunStarted {
                step_id: step.id.clone(),
                step_name: step.name.clone(),
                depth,
                kind: "wait".to_string(),
            });
        }

        // 执行子步骤（如果有）
        let mut scope = variables.clone();
        for sub in auto {
            if let Err(e) = self
                .execute_step(
                    sub,
                    depth + 1,
                    store,
                    events,
                    tool_exec,
                    &mut scope,
                    workflow_id,
                    llm,
                    emitter,
                    tool_schemas,
                    completed_ids,
                    run_record,
                )
                .await
            {
                // On error: log and continue (no early abort)
                events.emit(WorkflowEvent::Error {
                    message: format!("WaitStep '{}' step failed: {}", step.name, e),
                });
            }
        }
        for (k, v) in &scope {
            variables.insert(k.clone(), v.clone());
        }
        Ok("wait_completed".to_string())
    }
}
