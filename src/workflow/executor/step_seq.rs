//! 顺序执行子步骤
use super::*;

impl Executor {
    /// 顺序执行子步骤
    pub(super) async fn execute_seq_step<F, Fut>(
        &self,
        step: &Step,
        children: &[Step],
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
        // 子步骤使用独立变量作用域（继承父变量）
        let total = children.len();
        let mut scope = variables.clone();
        let timeout_secs = step.timeout_secs;

        let body = async {
            // ── 批量保存节流：每 10 步或每 5 秒保存一次 ──
            let mut save_counter: usize = 0;
            let mut last_save = std::time::Instant::now();
            const SAVE_EVERY_N: usize = 10;
            const SAVE_EVERY_SECS: u64 = 5;

            for (i, sub) in children.iter().enumerate() {
                // ── HUD: update progress ──
                if let Some(emitter) = emitter {
                    let step_label = if depth == 0 {
                        format!("Step {}/{}: {}", i + 1, total, sub.name())
                    } else {
                        sub.name()
                    };
                    emitter.emit(NuphusEvent::HudUpdate {
                        text: format!("{} — 执行中...", step_label),
                        phase: "workflow".into(),
                        step_kind: Some(sub.kind_str().to_string()),
                    });
                }

                // ── 执行子步骤 ──
                match self
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
                    Ok(_) => {}
                    Err(e) => {
                        // 根据 on_error 策略处理
                        let err_msg = e.to_string();
                        if sub.on_error_strategy().is_skip() {
                            events.emit(WorkflowEvent::Error {
                                message: format!(
                                    "步骤 '{}' ({}) 失败，跳过继续: {}",
                                    sub.name(),
                                    sub.kind_str(),
                                    err_msg
                                ),
                            });
                            // dispatch 已记录 Error，这里纠正为 Skipped（has_skipped 语义）
                            if let Some(rec) = run_record
                                .steps
                                .iter_mut()
                                .rev()
                                .find(|s| s.step_id == sub.id())
                            {
                                rec.status = StepRunStatus::Skipped;
                                rec.finished_at = Some(chrono::Utc::now());
                            }
                            continue;
                        }
                        // Abort or other: 立即传播
                        return Err(e);
                    }
                }

                // 提升子步骤变量
                for (k, v) in &scope {
                    variables.insert(k.clone(), v.clone());
                }

                // ── 批量保存（节流）：同步当前 run_record 步骤进度 ──
                save_counter += 1;
                let elapsed = last_save.elapsed().as_secs();
                if save_counter >= SAVE_EVERY_N || elapsed >= SAVE_EVERY_SECS {
                    if let Some(mut wf) = store.get(workflow_id).await {
                        sync_run_record(&mut wf, run_record);
                        let _ = store.save(&wf).await;
                    }
                    save_counter = 0;
                    last_save = std::time::Instant::now();
                }
            }

            // ── 强制最终保存（确保不丢数据）──
            if let Some(mut wf) = store.get(workflow_id).await {
                sync_run_record(&mut wf, run_record);
                let _ = store.save(&wf).await;
            }

            Ok("seq_completed".to_string())
        };

        if let Some(secs) = timeout_secs {
            match tokio::time::timeout(std::time::Duration::from_secs(secs), body).await {
                Ok(result) => result,
                Err(_) => Err(crate::NuphusError::agent(format!(
                    "Seq container '{}' timed out after {}s",
                    step.name, secs
                ))),
            }
        } else {
            body.await
        }
    }
}
/// 将内存中的 run_record 同步到工作流运行历史。
/// 同 run_id 就地覆盖（同一 run 的增量进度），不同 run_id push 为新记录 —— 避免重复历史条目。
pub(super) fn sync_run_record(wf: &mut crate::workflow::types::Workflow, run_record: &RunRecord) {
    if let Some(rec) = wf.last_run_mut() {
        if rec.run_id == run_record.run_id {
            *rec = run_record.clone();
            return;
        }
    }
    wf.push_run(run_record.clone());
}
