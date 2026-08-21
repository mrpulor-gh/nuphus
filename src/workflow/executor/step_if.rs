//! 条件分支执行
use super::*;

impl Executor {
    /// 条件分支执行
    pub(super) async fn execute_if_step<F, Fut>(
        &self,
        step: &Step,
        def: &IfDef,
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
        let matched = super::variables::eval_condition(&def.condition, variables);

        let target = if matched { &def.then } else { &def.else_branch };
        let timeout_secs = step.timeout_secs;

        let body = async {
            let mut scope = variables.clone();
            for sub in target {
                self.execute_step(
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
                .await?;
            }
            for (k, v) in &scope {
                variables.insert(k.clone(), v.clone());
            }
            Ok("if_completed".to_string())
        };

        if let Some(secs) = timeout_secs {
            match tokio::time::timeout(std::time::Duration::from_secs(secs), body).await {
                Ok(result) => result,
                Err(_) => Err(crate::NuphusError::agent(format!(
                    "If container '{}' timed out after {}s",
                    step.name, secs
                ))),
            }
        } else {
            body.await
        }
    }
}
