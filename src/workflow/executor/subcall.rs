//! 子工作流调用
use super::*;

impl Executor {
    /// 执行子工作流调用（wf_call 工具）
    pub(super) async fn execute_subcall<F, Fut>(
        &self,
        depth: u32,
        store: &WorkflowStore,
        events: &EventBus,
        params: &serde_json::Value,
        variables: &mut HashMap<String, serde_json::Value>,
        tool_exec: &F,
        llm: Option<&dyn ApiClient>,
        _emitter: Option<&dyn EventEmitter>,
        tool_schemas: Option<&[ToolDefinition]>,
        completed_ids: &std::collections::HashSet<String>,
        run_record: &mut RunRecord,
    ) -> crate::Result<String>
    where
        F: Fn(String, serde_json::Value) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = std::result::Result<String, String>> + Send,
    {
        const MAX_CALL_DEPTH: u32 = 10;
        if depth > MAX_CALL_DEPTH {
            return Err(crate::NuphusError::agent(format!(
                "子工作流调用深度超限 (max={})，可能存在循环调用。当前工作流: {}",
                MAX_CALL_DEPTH,
                params
                    .get("workflow_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
            )));
        }

        let wf_id = params
            .get("workflow_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::NuphusError::agent("wf_call: missing workflow_id".to_string()))?;

        let sub_wf = store.get(wf_id).await.ok_or_else(|| {
            crate::NuphusError::agent(format!("wf_call: workflow '{}' not found", wf_id))
        })?;

        events.emit(WorkflowEvent::SubWorkflowStarted {
            workflow_id: wf_id.to_string(),
            workflow_name: sub_wf.name.clone(),
        });

        let mut sub_vars = variables.clone();
        if let Some(inputs) = params.get("inputs").and_then(|v| v.as_object()) {
            for (k, v) in inputs {
                sub_vars.insert(k.clone(), v.clone());
            }
        }

        let root = Step::new_seq(&format!("subcall-{}", wf_id), &sub_wf.name, sub_wf.steps);
        let result = self
            .execute_step(
                &root,
                depth + 1,
                store,
                events,
                tool_exec,
                &mut sub_vars,
                wf_id,
                llm,
                None,
                tool_schemas,
                completed_ids,
                run_record,
            )
            .await;

        if let Some(outputs) = params.get("outputs").and_then(|v| v.as_object()) {
            for (output_key, parent_key_val) in outputs {
                if let Some(parent_key) = parent_key_val.as_str() {
                    if let Some(val) = sub_vars.get(output_key) {
                        variables.insert(parent_key.to_string(), val.clone());
                    }
                }
            }
        }

        match &result {
            Ok(_) => {
                events.emit(WorkflowEvent::SubWorkflowCompleted {
                    workflow_id: wf_id.to_string(),
                    workflow_name: sub_wf.name.clone(),
                    success: true,
                });
            }
            Err(e) => {
                events.emit(WorkflowEvent::SubWorkflowCompleted {
                    workflow_id: wf_id.to_string(),
                    workflow_name: sub_wf.name.clone(),
                    success: false,
                });
                return Err(crate::NuphusError::agent(format!(
                    "子工作流 '{}' 执行失败: {}",
                    sub_wf.name, e
                )));
            }
        }

        result
    }
}
