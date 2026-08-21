//! 工作流入口执行
use super::*;

impl Executor {
    /// 执行工作流入口
    ///
    /// 将 store 中的 Step 树递归执行。
    pub async fn execute_v2<F, Fut>(
        &self,
        workflow_id: &str,
        store: &WorkflowStore,
        events: &EventBus,
        tool_exec: F,
        llm: Option<&dyn ApiClient>,
        emitter: Option<&dyn EventEmitter>,
        tool_schemas: Option<&[ToolDefinition]>,
        inputs: Option<std::collections::HashMap<String, serde_json::Value>>,
    ) -> Result<String>
    where
        F: Fn(String, serde_json::Value) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = std::result::Result<String, String>> + Send,
    {
        let wf = store.get(workflow_id).await.ok_or_else(|| {
            crate::NuphusError::Agent(crate::AgentError::NotFound {
                what: "Workflow".to_string(),
                id: workflow_id.to_string(),
            })
        })?;

        // Compiler 静态验证（含工具注册表校验；未提供工具表时降级为基础校验）
        let report = match tool_schemas {
            Some(schemas) => Compiler::validate_workflow_with_tools(&wf, schemas),
            None => Compiler::validate_workflow(&wf),
        };
        if !report.passed {
            return Err(crate::NuphusError::agent(format!(
                "Workflow validation failed: {:?}",
                report.errors
            )));
        }
        for w in &report.warnings {
            tracing::warn!("[compiler] {}", w);
        }

        // Call 目标存在性 + 循环调用链静态检测
        let call_errors = Compiler::validate_calls(&wf, store).await;
        if !call_errors.is_empty() {
            return Err(crate::NuphusError::agent(format!(
                "Workflow call validation failed: {:?}",
                call_errors
            )));
        }

        // ── 初始化取消标志 ──
        {
            let mut flags = self.cancel_flags.write().await;
            if !flags.contains_key(workflow_id) {
                flags.insert(workflow_id.to_string(), Arc::new(AtomicBool::new(false)));
            }
        }

        // ── dry-run：仅编译校验，不执行 ──
        if wf.dry_run {
            return Ok(format!("工作流 '{}' 编译校验通过 (dry-run)", wf.name));
        }

        // ── 断点续连：跳过已完成步骤（Success + Skipped）──
        let completed_ids: std::collections::HashSet<String> = wf
            .last_run()
            .filter(|r| r.status == RunStatus::Paused || matches!(r.status, RunStatus::Error(_)))
            .map(|r| {
                r.steps
                    .iter()
                    .filter(|s| {
                        s.status == StepRunStatus::Success || s.status == StepRunStatus::Skipped
                    })
                    .map(|s| s.step_id.clone())
                    .collect()
            })
            .unwrap_or_default();

        // 恢复/重试时继承上次已完成步骤记录，保持本次 run_record 视图完整
        // （对应步骤经 completed_ids 跳过不再执行，故无重复记录）
        let seed_steps: Vec<StepRunRecord> = if completed_ids.is_empty() {
            Vec::new()
        } else {
            wf.last_run()
                .map(|r| {
                    r.steps
                        .iter()
                        .filter(|s| {
                            s.status == StepRunStatus::Success || s.status == StepRunStatus::Skipped
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        };

        // ── ChatAgent 会话生命周期 ──
        // 全新执行（非恢复/非重试）→ 清理旧会话，确保干净开始
        // 暂停恢复 / 失败重试 → 保留会话，维持上下文连续性
        let is_resume = !completed_ids.is_empty();
        let resume_snapshot = if is_resume {
            wf.last_run().map(|r| r.variables_snapshot.clone())
        } else {
            None
        };
        if !is_resume {
            let mut sessions = self.chat_sessions.write().await;
            sessions.retain(|k, _| !k.starts_with(&format!("{}:", workflow_id)));
        }

        let mut run_record = RunRecord {
            run_id: uuid::Uuid::new_v4().to_string(),
            started_at: chrono::Utc::now(),
            finished_at: None,
            status: RunStatus::Running,
            steps: seed_steps,
            error: None,
            variables_snapshot: std::collections::HashMap::new(),
        };

        // Emit workflow-event: run_started — 前端据此设置 workflowRunId
        events.emit(WorkflowEvent::RunStarted {
            run_id: run_record.run_id.clone(),
            workflow_id: workflow_id.to_string(),
        });

        // 标记 run 开始：之后所有 save() 只第一次会写备份
        store.begin_run_backup_window().await;

        let total_steps = wf.steps.len();

        let active_steps: Vec<Step> = if completed_ids.is_empty() {
            wf.steps
        } else {
            wf.steps
                .into_iter()
                .filter(|s| !completed_ids.contains(&s.id()))
                .collect()
        };

        let root = Step::new_seq(workflow_id, &wf.name, active_steps);

        let mut variables = HashMap::new();

        // ── 加载 params.json 固化参数到变量池（兑现 {params.xxx} 引用）──
        // 路径约定：plugin/workflows/{wf.id}/params.json
        {
            let params_path = crate::utils::workspace_root()
                .join("plugin")
                .join("workflows")
                .join(&wf.id)
                .join("params.json");
            match tokio::fs::read_to_string(&params_path).await {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(v) => {
                        variables.insert("params".to_string(), v);
                    }
                    Err(e) => tracing::warn!(
                        "[executor] params.json 解析失败，{{params.xxx}} 引用将不可用: {}",
                        e
                    ),
                },
                Err(_) => {
                    // 文件不存在属正常情况（后台工作流无 params.json）
                }
            }
        }

        // ── 注入运行时 inputs（workflow_run(inputs) → 变量池顶层）──
        if let Some(inp) = inputs {
            for (k, v) in inp {
                tracing::debug!("[executor] inputs.{} = {:?}", k, v);
                variables.insert(k, v);
            }
        }

        // ── 断点续连：从快照恢复变量池 ──
        if let Some(snapshot) = &resume_snapshot {
            for (k, v) in snapshot {
                variables.insert(k.clone(), v.clone());
            }
            tracing::info!(
                "[executor] Restored {} variables from pause snapshot",
                snapshot.len()
            );
        }

        // ── HUD: register active workflow ──
        crate::workflow::hud_control::set_active(&self.signals, workflow_id);

        // Emit HUD: workflow started
        if let Some(emitter) = emitter {
            emitter.emit(NuphusEvent::HudUpdate {
                text: format!("工作流: {} — 执行中", wf.name),
                phase: "workflow".into(),
                step_kind: None,
            });
        }

        // ── 将当前 run 写入 store（Running）──
        // 执行期间 step_seq 节流保存会同步步骤进度；pause 快照（check_pause）也写入本记录
        if let Some(mut store_wf) = store.get(workflow_id).await {
            step_seq::sync_run_record(&mut store_wf, &run_record);
            let _ = store.save(&store_wf).await;
        }

        // ── 工作流级超时 ──
        let exec_future = self.execute_step(
            &root,
            0,
            store,
            events,
            &tool_exec,
            &mut variables,
            workflow_id,
            llm,
            emitter,
            tool_schemas,
            &completed_ids,
            &mut run_record,
        );
        let result = if let Some(timeout) = wf.timeout_secs {
            match tokio::time::timeout(std::time::Duration::from_secs(timeout), exec_future).await {
                Ok(r) => r,
                Err(_) => {
                    // 超时：清理 chat_sessions 避免残留
                    {
                        let mut sessions = self.chat_sessions.write().await;
                        sessions.retain(|k, _| !k.starts_with(&format!("{}:", workflow_id)));
                    }
                    Err(crate::NuphusError::agent(format!(
                        "工作流 '{}' 超时（{}秒）",
                        wf.name, timeout
                    )))
                }
            }
        } else {
            exec_future.await
        };

        // Emit HUD: workflow done/error
        if let Some(emitter) = emitter {
            let (phase, desc) = match &result {
                Ok(_) => ("done", "完成"),
                Err(_) => ("error", "失败"),
            };
            emitter.emit(NuphusEvent::HudUpdate {
                text: format!("工作流: {} — {}", wf.name, desc),
                phase: phase.into(),
                step_kind: None,
            });
        }

        // Emit workflow-event: run_completed — 前端据此重置 isWorkflowPaused 并标记剩余步骤 completed
        // ── 基于内存 run_record 判断最终状态（含 Skipped/Failed 步骤记录）──
        let run_status = match &result {
            Ok(_) => {
                let has_skipped = run_record
                    .steps
                    .iter()
                    .any(|s| s.status == StepRunStatus::Skipped);
                if has_skipped {
                    RunStatus::Error("some steps skipped".into())
                } else {
                    RunStatus::Success
                }
            }
            Err(_) => RunStatus::Error("execution failed".into()),
        };
        events.emit(WorkflowEvent::RunCompleted {
            run_id: run_record.run_id.clone(),
            status: run_status,
        });

        crate::workflow::hud_control::clear_active(&self.signals);

        // ── 保存运行记录 ──
        // 统一：将当前 run 定稿（Success/Error）后同步回 store。
        // 成功/失败都产生新记录（含本次 steps），不修改上一条（可能成功）记录。
        match &result {
            Ok(_) => {
                // ── ChatAgent 会话清理：工作流成功完成，清除所有会话 ──
                {
                    let mut sessions = self.chat_sessions.write().await;
                    sessions.retain(|k, _| !k.starts_with(&format!("{}:", workflow_id)));
                }

                run_record.finished_at = Some(chrono::Utc::now());
                run_record.status = RunStatus::Success;
                // ── 完成时保存变量快照 ——
                let snapshot: Option<std::collections::HashMap<String, serde_json::Value>> = {
                    let mut m = std::collections::HashMap::new();
                    for (k, v) in &variables {
                        m.insert(k.clone(), v.clone());
                    }
                    if m.is_empty() {
                        None
                    } else {
                        Some(m)
                    }
                };
                run_record.variables_snapshot = snapshot.unwrap_or_default();
                if let Some(mut wf) = store.get(workflow_id).await {
                    step_seq::sync_run_record(&mut wf, &run_record);
                    let _ = store.save(&wf).await;
                }
            }
            Err(e) => {
                // 失败：定稿为新 Error 记录并同步（push_run 语义，绝不改动上一次成功记录）
                run_record.finished_at = Some(chrono::Utc::now());
                run_record.status = RunStatus::Error(e.to_string());
                // ── 失败时同样保存变量快照 ——
                let snapshot: Option<std::collections::HashMap<String, serde_json::Value>> = {
                    let mut m = std::collections::HashMap::new();
                    for (k, v) in &variables {
                        m.insert(k.clone(), v.clone());
                    }
                    if m.is_empty() {
                        None
                    } else {
                        Some(m)
                    }
                };
                run_record.variables_snapshot = snapshot.unwrap_or_default();
                if let Some(mut wf) = store.get(workflow_id).await {
                    step_seq::sync_run_record(&mut wf, &run_record);
                    let _ = store.save(&wf).await;
                }
            }
        }

        // ── 清理取消标志 ──
        self.cancel_flags.write().await.remove(workflow_id);

        let completed_on_error: Vec<String> = if result.is_err() {
            run_record
                .steps
                .iter()
                .filter(|s| s.status == StepRunStatus::Success)
                .map(|s| s.step_id.clone())
                .collect()
        } else {
            vec![]
        };

        result
            .map(|_msg| format!("工作流完成，共 {} 步", total_steps))
            .map_err(|e| {
                // ── 结构化错误 ──
                // resume_hint 显式引导调用方：解决阻塞后重调 workflow_run 同 id 即断点续连
                let err_msg = e.to_string();
                crate::NuphusError::agent(format!(
                    "{{\"failed\":true,\"error\":{},\"completed_steps\":{},\"resume_hint\":{}}}",
                    serde_json::to_string(&err_msg).unwrap_or_else(|_| "\"unknown\"".into()),
                    serde_json::to_string(&completed_on_error).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string("解决阻塞后调用 workflow_run 传同一 id 即可断点续连（自动跳过已完成步骤，从失败步骤继续）").unwrap_or_else(|_| "\"\"".into())
                ))
            })
    }
}
