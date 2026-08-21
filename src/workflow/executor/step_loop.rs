//! 循环执行子步骤
use super::*;

impl Executor {
    /// 循环执行子步骤
    pub(super) async fn execute_loop_step<F, Fut>(
        &self,
        step: &Step,
        def: &LoopDef,
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
        // ── ForEach ──
        if let Some(ref fe) = def.for_each {
            let items_val = resolve_var_ref(&fe.items, variables);
            let items: Vec<serde_json::Value> = match items_val {
                serde_json::Value::Array(arr) => arr,
                serde_json::Value::String(s) => {
                    // Try parsing as JSON array
                    match serde_json::from_str::<Vec<serde_json::Value>>(&s) {
                        Ok(arr) => arr,
                        Err(_) => {
                            return Ok("loop_for_each_empty".to_string());
                        }
                    }
                }
                _ => {
                    return Ok("loop_for_each_empty".to_string());
                }
            };

            if items.is_empty() {
                return Ok("loop_for_each_empty".to_string());
            }

            for (i, item) in items.iter().enumerate() {
                self.check_cancel(workflow_id).await?;
                self.check_pause(
                    workflow_id,
                    events,
                    &step.id,
                    &step.name,
                    depth,
                    "loop",
                    Some(store),
                    Some(variables),
                )
                .await?;

                let mut scope = variables.clone();
                scope.insert(fe.item_var.clone(), item.clone());
                scope.insert("_index".to_string(), serde_json::Value::from(i as i64));

                for sub in &def.steps {
                    if sub.kind_str() == "break" {
                        return Ok("loop_broken".to_string());
                    }
                    if sub.kind_str() == "continue" {
                        break;
                    }
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
            }
            return Ok("loop_completed".to_string());
        }

        // ── Fixed repeat ──
        if let Some(count) = def.repeat {
            for i in 0..count {
                self.check_cancel(workflow_id).await?;
                self.check_pause(
                    workflow_id,
                    events,
                    &step.id,
                    &step.name,
                    depth,
                    "loop",
                    Some(store),
                    Some(variables),
                )
                .await?;

                let mut scope = variables.clone();
                scope.insert("_index".to_string(), serde_json::Value::from(i as i64));

                for sub in &def.steps {
                    if sub.kind_str() == "break" {
                        return Ok("loop_broken".to_string());
                    }
                    if sub.kind_str() == "continue" {
                        break;
                    }
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
            }
            return Ok("loop_completed".to_string());
        }

        // ── Until ──
        if let Some(ref until_cond) = def.until {
            let max = def.max;
            for _i in 0..max {
                self.check_cancel(workflow_id).await?;
                self.check_pause(
                    workflow_id,
                    events,
                    &step.id,
                    &step.name,
                    depth,
                    "loop",
                    Some(store),
                    Some(variables),
                )
                .await?;

                let mut scope = variables.clone();
                for sub in &def.steps {
                    if sub.kind_str() == "break" {
                        return Ok("loop_broken".to_string());
                    }
                    if sub.kind_str() == "continue" {
                        break;
                    }
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

                // 检查终止条件
                if super::variables::eval_condition(until_cond, variables) {
                    return Ok("loop_until_met".to_string());
                }
            }
            return Ok("loop_max_reached".to_string());
        }

        // No loop mode specified → single pass
        let mut scope = variables.clone();
        for sub in &def.steps {
            if sub.kind_str() == "break" {
                return Ok("loop_broken".to_string());
            }
            if sub.kind_str() == "continue" {
                break;
            }
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
        Ok("loop_completed".to_string())
    }
}

/// Resolve a VarRef to a serde_json::Value from the variable pool.
/// 支持点号路径（root.field[.field...]），如 {{panels.list}} → variables["panels"]["list"]
fn resolve_var_ref(
    var_ref: &crate::workflow::types::VarRef,
    variables: &HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    match var_ref {
        crate::workflow::types::VarRef::Var { var } => {
            super::variables::resolve_var_by_path(var, variables)
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        }
        crate::workflow::types::VarRef::Lit(s) => serde_json::Value::String(s.clone()),
    }
}
