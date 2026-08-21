//! 委托子工作流调用
use super::*;

impl Executor {
    /// 委托子工作流调用
    pub(super) async fn execute_call_step<F, Fut>(
        &self,
        step: &Step,
        workflow_id_param: &str,
        params: &serde_json::Value,
        depth: u32,
        store: &WorkflowStore,
        events: &EventBus,
        tool_exec: &F,
        variables: &mut HashMap<String, serde_json::Value>,
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
        let _ = step;
        let mut call_params = params.clone();
        // Ensure workflow_id is present
        if call_params.get("workflow_id").is_none() {
            if let Some(obj) = call_params.as_object_mut() {
                obj.insert(
                    "workflow_id".to_string(),
                    serde_json::Value::String(workflow_id_param.to_string()),
                );
            }
        }
        self.execute_subcall(
            depth,
            store,
            events,
            &call_params,
            variables,
            tool_exec,
            llm,
            emitter,
            tool_schemas,
            completed_ids,
            run_record,
        )
        .await
    }
}
