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

        let inputs_value = params
            .get("inputs")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let inputs = inputs_value.as_object().ok_or_else(|| {
            crate::NuphusError::agent("wf_call: with.inputs 必须是对象".to_string())
        })?;
        // Input mappings are evaluated against the parent scope before the child scope is built.
        // A full template keeps its JSON type; embedded templates become strings as elsewhere.
        let resolved_value =
            Self::resolve_vars(&serde_json::Value::Object(inputs.clone()), variables);
        let provided: HashMap<String, serde_json::Value> = resolved_value
            .as_object()
            .expect("resolving an object preserves its shape")
            .clone()
            .into_iter()
            .collect();
        let declared = crate::workflow::inputs::resolve_declared_inputs(&sub_wf.inputs, &provided)?;

        let mut sub_vars = variables.clone();
        if !sub_wf.inputs.is_empty() {
            sub_vars.insert(
                "inputs".to_string(),
                serde_json::Value::Object(declared.clone()),
            );
        }
        for (key, value) in &declared {
            sub_vars.insert(key.clone(), value.clone());
        }
        // Compatibility: old sub-workflows may receive undeclared mapping keys. Keep those at
        // the top level, but do not add them to the formal `inputs` namespace.
        for (key, value) in provided {
            if !declared.contains_key(&key) {
                sub_vars.insert(key, value);
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

        if let Some(outputs_value) = params.get("outputs") {
            let outputs = outputs_value.as_object().ok_or_else(|| {
                crate::NuphusError::agent("wf_call: with.outputs 必须是对象".to_string())
            })?;
            for (output_key, parent_key_val) in outputs {
                let parent_key = parent_key_val.as_str().ok_or_else(|| {
                    crate::NuphusError::agent(format!(
                        "wf_call: with.outputs.{} 必须映射到父变量名字符串",
                        output_key
                    ))
                })?;
                if let Some(val) = sub_vars.get(output_key) {
                    variables.insert(parent_key.to_string(), val.clone());
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
