//! Nuphus Workflow Engine
//!
//! Streamlined architecture: single-pass planning → deterministic execution.
//! Removed multi-turn dialogue, step-by-step debugging, simulation, and self-healing state machines.

pub mod chat_agent;
pub mod compiler;
pub mod events;
pub mod executor;
pub mod hud_control;
pub mod scheduler;
pub mod store;
pub mod types;

#[cfg(test)]
mod tests;

use crate::api::ApiClient;
use crate::api::ToolDefinition;
use crate::tools::ToolRegistry;
use crate::workflow::compiler::Compiler;
use crate::workflow::events::{EventBus, WorkflowEvent};
use crate::workflow::executor::Executor;
use crate::workflow::scheduler::SchedulerEngine;
use crate::workflow::store::WorkflowStore;
use crate::workflow::types::{Action, ScheduleConfig, Workflow};
use crate::Result;
use std::sync::Arc;

/// Schedule-triggered execution callback — injected by Tauri command layer (with ToolRegistry)
pub type ScheduleExecCallback = Arc<
    dyn Fn(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync,
>;

/// WorkflowEngine — workflow orchestration core
pub struct WorkflowEngine {
    pub store: WorkflowStore,
    pub events: EventBus,
    pub executor: Executor,
    pub scheduler: SchedulerEngine,
    llm_client: Option<Arc<dyn ApiClient>>,
    /// Global tool registry — used by ChatAgent steps to build tool definitions
    /// when no explicit tool_schemas are passed. Injected via set_tools().
    tools: Option<Arc<ToolRegistry>>,
    /// Schedule execution callback (injected externally)
    /// Uses std::sync::Mutex — written once during setup, read on set_schedule clone
    schedule_exec: std::sync::Mutex<Option<ScheduleExecCallback>>,
}

impl WorkflowEngine {
    pub fn new() -> Self {
        Self {
            store: WorkflowStore::new(),
            events: EventBus::new(),
            executor: Executor::new(),
            scheduler: SchedulerEngine::new(),
            llm_client: None,
            tools: None,
            schedule_exec: std::sync::Mutex::new(None),
        }
    }

    /// Set the schedule execution callback (called once at Tauri startup)
    pub fn set_schedule_exec_callback(&self, cb: ScheduleExecCallback) {
        *self.schedule_exec.lock().unwrap() = Some(cb);
    }

    /// Set the LLM client
    pub fn set_llm_client(&mut self, client: Arc<dyn ApiClient>) {
        self.llm_client = Some(client);
    }

    /// 注入共享信号句柄（透传到 Executor，desktop shell 启动时调用一次）
    pub fn set_signals(&mut self, signals: crate::state::SharedSignals) {
        self.executor.set_signals(signals);
    }

    /// 注入模型客户端工厂（透传到 Executor，chat 步骤 per-step 模型路由依赖）
    pub fn set_client_factory(&mut self, factory: crate::llm::ClientFactory) {
        self.executor.set_client_factory(factory);
    }

    /// Inject global tool registry for ChatAgent steps and sub-workflow execution.
    /// Must be called before any workflow execution that uses Talk steps.
    pub fn set_tools(&mut self, tools: Arc<ToolRegistry>) {
        self.tools = Some(tools);
    }

    /// Get LLM client reference (for external use like workflow_run)
    pub fn llm_client(&self) -> Option<&dyn ApiClient> {
        self.llm_client.as_deref()
    }

    /// Get tool registry reference
    pub fn tools(&self) -> Option<&Arc<ToolRegistry>> {
        self.tools.as_ref()
    }

    pub async fn init(&self) -> Result<()> {
        self.store.load_all().await
    }

    pub fn event_bus(&self) -> &EventBus {
        &self.events
    }

    // ── CRUD ──

    pub async fn list_workflows(&self) -> Vec<Workflow> {
        let summaries = self.store.list().await;
        let mut workflows = Vec::new();
        for s in &summaries {
            if let Some(wf) = self.store.get(&s.id).await {
                workflows.push(wf);
            }
        }
        workflows
    }

    pub async fn get_workflow(&self, id: &str) -> Option<Workflow> {
        self.store.get(id).await
    }

    pub async fn create_workflow(&self, name: &str) -> Result<Workflow> {
        let wf = Workflow::new(name);
        // Create dedicated directory structure
        self.store.ensure_dirs(&wf.id).await?;
        self.store.save(&wf).await?;
        self.events.emit(WorkflowEvent::StatusChange {
            status: "created".to_string(),
        });
        Ok(wf)
    }

    pub async fn delete_workflow(&self, id: &str) -> Result<()> {
        // 清理关联的 ChatAgent 配置：运行时按 opts.agent_id（ID）加载，删除必须按 ID。
        // 无 agent_id 的 Chat 步骤使用全局 active agent，删除工作流不应删全局配置 → 跳过。
        if let Some(wf) = self.store.get(id).await {
            for step in &wf.steps {
                if let Action::Chat { with: ref opts, .. } = &step.action {
                    if let Some(ref agent_id) = opts.agent_id {
                        let _ = chat_agent::ChatAgentStore::delete_by_id(agent_id);
                    }
                }
            }
        }
        self.store.delete(id).await
    }

    // ── Execution ──

    /// Execute a workflow (delegates to executor.execute_v2)
    ///
    /// Runs Compiler validation before execution to ensure basic correctness.
    pub async fn execute_workflow<F, Fut>(
        &self,
        workflow_id: &str,
        tool_exec: F,
        tool_schemas: Option<Vec<ToolDefinition>>,
        emitter: Option<&dyn crate::agent::events::EventEmitter>,
        inputs: Option<std::collections::HashMap<String, serde_json::Value>>,
    ) -> Result<String>
    where
        F: Fn(String, serde_json::Value) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = std::result::Result<String, String>> + Send,
    {
        // ── Pre-execution validation（与 execute_v2 内部校验同源）──
        // tool_schemas 提前构建，供校验与执行共用
        let tool_schemas = tool_schemas.or_else(|| self.tools.as_ref().map(|t| t.get_schemas()));
        if let Some(wf) = self.store.get(workflow_id).await {
            let report = match tool_schemas.as_deref() {
                Some(schemas) => Compiler::validate_workflow_with_tools(&wf, schemas),
                None => Compiler::validate_workflow(&wf),
            };
            // Call 目标存在性 + 循环调用链静态检测
            let mut errors = report.errors;
            errors.extend(Compiler::validate_calls(&wf, &self.store).await);
            for w in &report.warnings {
                tracing::warn!("Workflow validation warning: {}", w);
            }
            if !errors.is_empty() {
                return Err(crate::NuphusError::agent(format!(
                    "Workflow validation failed: {}",
                    errors.join("; ")
                )));
            }
        }

        let llm = self.llm_client.as_deref();
        let result = self
            .executor
            .execute_v2(
                workflow_id,
                &self.store,
                &self.events,
                tool_exec,
                llm,
                emitter,
                tool_schemas.as_deref(),
                inputs,
            )
            .await;

        // ── Auto-export dual artifacts after successful execution ──
        if result.is_ok() {
            if let Err(e) = self.export_workflow(workflow_id).await {
                tracing::warn!("Auto-export failed for workflow '{}': {e}", workflow_id);
            }
        }

        result
    }

    /// Cancel execution
    pub async fn cancel_workflow(&self, id: &str) {
        self.executor.cancel(id).await;
    }

    /// Pause execution
    pub async fn pause_workflow(&self, id: &str) {
        self.executor.pause(id).await;
    }

    /// Check if paused
    pub async fn is_paused(&self, id: &str) -> bool {
        self.executor.is_paused(id).await
    }

    /// Resume execution
    pub async fn resume_workflow(&self, id: &str) {
        self.executor.resume(id).await;
    }

    // ── Scheduling ──

    /// Set workflow cron schedule
    pub async fn set_schedule(&self, workflow_id: &str, config: ScheduleConfig) -> Result<()> {
        // Persist to store
        if let Some(mut wf) = self.store.get(workflow_id).await {
            wf.schedule = Some(config.clone());
            wf.updated_at = Some(chrono::Utc::now());
            self.store.save(&wf).await?;

            // Start scheduler timer
            let wf_id = workflow_id.to_string();
            let store_ref = &self.store;
            let exec_cb = self.schedule_exec.lock().unwrap().clone();
            self.scheduler
                .set_schedule(&wf_id.clone(), config, store_ref, move || {
                    let wf_id = wf_id.clone();
                    let exec_cb = exec_cb.clone();
                    async move {
                        tracing::info!("[Scheduler] Cron fired for workflow: {}", wf_id);
                        if let Some(ref cb) = exec_cb {
                            cb(wf_id).await;
                        } else {
                            tracing::warn!(
                                "[Scheduler] No exec callback registered for workflow: {}",
                                wf_id
                            );
                        }
                    }
                })
                .await?;
        }
        self.events.emit(WorkflowEvent::StatusChange {
            status: "schedule_set".to_string(),
        });
        Ok(())
    }

    /// Remove schedule
    pub async fn remove_schedule(&self, workflow_id: &str) {
        if let Some(mut wf) = self.store.get(workflow_id).await {
            wf.schedule = None;
            let _ = self.store.save(&wf).await;
        }
        self.scheduler.remove_schedule(workflow_id).await;
        self.events.emit(WorkflowEvent::StatusChange {
            status: "schedule_removed".to_string(),
        });
    }

    /// 启动时恢复所有持久化的调度任务
    /// 需在 set_schedule_exec_callback 之后调用
    pub async fn restore_schedules(&self) {
        let persisted = crate::workflow::scheduler::SchedulerEngine::load_persisted();
        if persisted.schedules.is_empty() {
            return;
        }
        tracing::info!(
            "[Scheduler] Restoring {} persisted schedule(s)",
            persisted.schedules.len()
        );
        for (wf_id, config) in &persisted.schedules {
            if !config.enabled {
                continue;
            }
            let wf_id = wf_id.clone();
            let config = config.clone();
            if let Err(e) = self.set_schedule(&wf_id, config).await {
                tracing::warn!(
                    "[Scheduler] Failed to restore schedule for '{}': {}",
                    wf_id,
                    e
                );
            }
        }
    }

    // ── Dual artifact export ──

    /// Export workflow dual artifacts to plugin/workflows/ directory
    ///
    /// Returns (json_path, md_path)
    pub async fn export_workflow(&self, workflow_id: &str) -> Result<(String, String)> {
        let wf = self.store.get(workflow_id).await.ok_or_else(|| {
            crate::NuphusError::agent(format!("Workflow not found: {workflow_id}"))
        })?;

        let export_dir = crate::utils::workspace_root()
            .join("plugin")
            .join("workflows")
            .join(&wf.id);
        tokio::fs::create_dir_all(&export_dir).await?;

        let json_content = self.store.export_json(&wf);
        let md_content = self.store.export_md(&wf);

        // Sanitize filename: use workflow name, replace illegal chars
        let safe_name: String = wf
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        let json_path = export_dir.join(format!("{}.json", safe_name));
        let md_path = export_dir.join(format!("{}.md", safe_name));

        tokio::fs::write(&json_path, &json_content).await?;
        tokio::fs::write(&md_path, &md_content).await?;

        Ok((
            json_path.to_string_lossy().to_string(),
            md_path.to_string_lossy().to_string(),
        ))
    }
}

impl Default for WorkflowEngine {
    fn default() -> Self {
        Self::new()
    }
}
