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

/// Agent 执行态读取器 — 由 Tauri shell 注入读 AppState.busy 的闭包（无宿主 = 视为空闲）
pub type BusyProvider = Arc<dyn Fn() -> bool + Send + Sync>;

/// workflow 执行发起方（全局执行闸门语义）
///
/// 大王铁律：无并行机制、禁止并行、系统操作更禁止并行。
/// - `Agent`：发起 workflow_run 的 Agent 本身已持有 busy（Leader/Workflow/Custom 任一在执行），
///   豁免 busy 检查；但已有 active workflow 时仍拒绝（防同 wf 重复 / 双 run）。
/// - `Ui` / `Schedule` / `Plugin`：busy 或 active workflow 任一存在即拒绝。
#[derive(Debug, Clone, Copy)]
pub enum WorkflowRunSource {
    /// Agent（Leader/Workflow/Custom 运行中）发起 workflow_run
    Agent { owner: crate::runtime::Mode },
    /// 画布 / 前台用户直接触发（wf_run 命令）
    Ui,
    /// 定时调度触发（cron）
    Schedule,
    /// 插件宿主触发
    Plugin,
}

impl WorkflowRunSource {
    pub fn is_agent(&self) -> bool {
        matches!(self, WorkflowRunSource::Agent { .. })
    }

    /// 登记 owner label（active_run 查询 / 错误提示用）
    pub fn owner_label(&self) -> &'static str {
        match self {
            WorkflowRunSource::Agent { owner } => owner.as_str(),
            WorkflowRunSource::Ui => "ui",
            WorkflowRunSource::Schedule => "schedule",
            WorkflowRunSource::Plugin => "plugin",
        }
    }
}

/// 全局 active run 注册信息（闸门登记使用；execute_v2 内部另有真实 run_id）
#[derive(Debug, Clone)]
pub struct ActiveRunInfo {
    pub workflow_id: String,
    pub run_id: String,
    /// 发起方 label（leader / workflow / custom / ui / schedule / plugin）
    pub owner: String,
    pub started_at_ms: u64,
}

fn gate_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

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
    /// 全局执行闸门：当前 active workflow run（大王铁律：同一时刻至多一个）。
    /// std::sync::Mutex —— 登记/释放/查询均短临界区（无跨 await 持锁）；
    /// RAII Drop 保证 execute_workflow 无论 Ok/Err/panic/task-abort/超时被 drop
    /// 都释放（async 无法在 Drop 中 await，故不用 tokio RwLock）。
    active_run: std::sync::Mutex<Option<ActiveRunInfo>>,
    /// Agent 执行态读取器（读 AppState.busy），由 Tauri setup 注入；None = 无宿主（视为空闲）。
    /// std::sync::Mutex — 写入一次（setup），每次闸门/查询时读取。
    busy_provider: std::sync::Mutex<Option<BusyProvider>>,
}

/// RAII：持有期间全局 active workflow run 生效；Drop 无条件释放。
///
/// 为什么不用 async finally：execute_workflow 的 future 可能被外部整体 drop
/// （plugin 300s 硬超时经 tokio::time::timeout 丢弃 inner future / task abort / panic），
/// 这些路径上 await 之后的清理代码不会执行；Drop 必然执行，且 std Mutex 可在 Drop
/// 中同步加锁（tokio RwLock 无法在 Drop 中 await）。
struct ActiveRunGuard<'a> {
    engine: &'a WorkflowEngine,
}

impl<'a> ActiveRunGuard<'a> {
    /// 闸门检查 + 登记（check-then-set 在同一 std Mutex 临界区内，并发仅一路成功）
    fn acquire(
        engine: &'a WorkflowEngine,
        workflow_id: &str,
        source: &WorkflowRunSource,
    ) -> crate::Result<ActiveRunGuard<'a>> {
        let mut slot = engine.active_run.lock().unwrap();
        if let Some(existing) = slot.as_ref() {
            if source.is_agent() {
                return Err(crate::NuphusError::agent(format!(
                    "工作流正在执行中，请等待完成后重试（当前：{}）",
                    existing.owner
                )));
            }
            return Err(crate::NuphusError::agent(
                "当前有任务执行中，暂不可用！".to_string(),
            ));
        }
        // Agent 自身发起：豁免 busy 检查（发起者即 busy 持有者）；Ui/Schedule/Plugin 双查
        if !source.is_agent() {
            let busy_guard = engine.busy_provider.lock().unwrap();
            if let Some(cb) = busy_guard.as_ref() {
                if cb() {
                    return Err(crate::NuphusError::agent(
                        "当前有任务执行中，暂不可用！".to_string(),
                    ));
                }
            }
        }
        *slot = Some(ActiveRunInfo {
            workflow_id: workflow_id.to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            owner: source.owner_label().to_string(),
            started_at_ms: gate_now_ms(),
        });
        Ok(ActiveRunGuard { engine })
    }
}

impl Drop for ActiveRunGuard<'_> {
    fn drop(&mut self) {
        *self.engine.active_run.lock().unwrap() = None;
    }
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
            active_run: std::sync::Mutex::new(None),
            busy_provider: std::sync::Mutex::new(None),
        }
    }

    /// Set the schedule execution callback (called once at Tauri startup)
    pub fn set_schedule_exec_callback(&self, cb: ScheduleExecCallback) {
        *self.schedule_exec.lock().unwrap() = Some(cb);
    }

    // ── 全局执行闸门（大王铁律：无并行机制、禁止并行、系统操作更禁止并行）──

    /// 注入 Agent 执行态读取器（Tauri shell setup 注入读 AppState.busy 的闭包；
    /// 无宿主（单元测试 / headless）时为 None，视为空闲）
    pub fn set_busy_provider(&self, cb: BusyProvider) {
        *self.busy_provider.lock().unwrap() = Some(cb);
    }

    /// 当前 active workflow run（None = 空闲）
    pub fn active_run_info(&self) -> Option<ActiveRunInfo> {
        self.active_run.lock().unwrap().clone()
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
    ///
    /// `source` 标识发起方，驱动全局执行闸门（大王铁律）：
    /// Agent 自身发起豁免 busy；Ui/Schedule/Plugin 在 busy 或已有 active workflow 时拒绝。
    pub async fn execute_workflow<F, Fut>(
        &self,
        workflow_id: &str,
        tool_exec: F,
        tool_schemas: Option<Vec<ToolDefinition>>,
        emitter: Option<&dyn crate::agent::events::EventEmitter>,
        inputs: Option<std::collections::HashMap<String, serde_json::Value>>,
        force_fresh: bool,
        source: WorkflowRunSource,
    ) -> Result<String>
    where
        F: Fn(String, serde_json::Value) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = std::result::Result<String, String>> + Send,
    {
        // ── 全局执行闸门 ──
        // validation 之前：资源校验不必要先跑；guard 持有期间本 run 独占执行权，
        // guard drop（含 validation 失败早退 / Ok / Err / panic / future 被 drop）自动释放。
        let _gate = ActiveRunGuard::acquire(self, workflow_id, &source)?;

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
                force_fresh,
            )
            .await;

        // ── Auto-export dual artifacts after successful execution ──
        if result.is_ok() {
            if let Err(e) = self.export_workflow(workflow_id).await {
                tracing::warn!("Auto-export failed for workflow '{}': {e}", workflow_id);
            }
        }

        // guard 在此作用域末尾 drop → 释放全局 active run（Ok/Err 同路径）
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

#[cfg(test)]
mod gate_tests {
    use super::*;

    fn engine() -> WorkflowEngine {
        WorkflowEngine::new()
    }

    #[tokio::test]
    async fn agent_allowed_when_idle_even_busy() {
        // Agent 自身豁免 busy：busy provider = true 也放行，且登记 active_run
        let e = engine();
        e.set_busy_provider(Arc::new(|| true));
        let guard = ActiveRunGuard::acquire(
            &e,
            "wf-a",
            &WorkflowRunSource::Agent {
                owner: crate::runtime::Mode::Leader,
            },
        )
        .expect("agent should pass gate when idle (busy exempt)");
        let info = e.active_run_info().expect("registered");
        assert_eq!(info.workflow_id, "wf-a");
        assert_eq!(info.owner, "leader");
        drop(guard);
        assert!(e.active_run_info().is_none(), "released after guard drop");
    }

    #[tokio::test]
    async fn non_agent_rejected_when_busy() {
        let e = engine();
        e.set_busy_provider(Arc::new(|| true));
        let err = ActiveRunGuard::acquire(&e, "wf-a", &WorkflowRunSource::Ui)
            .err()
            .expect("gate should reject")
            .to_string();
        assert!(err.contains("当前有任务执行中"), "unexpected: {err}");
        assert!(e.active_run_info().is_none(), "nothing registered");
    }

    #[tokio::test]
    async fn non_agent_rejected_when_active_run_exists() {
        let e = engine();
        let _guard = ActiveRunGuard::acquire(
            &e,
            "wf-a",
            &WorkflowRunSource::Agent {
                owner: crate::runtime::Mode::Workflow,
            },
        )
        .unwrap();
        let err = ActiveRunGuard::acquire(&e, "wf-b", &WorkflowRunSource::Schedule)
            .err()
            .expect("gate should reject")
            .to_string();
        assert!(err.contains("当前有任务执行中"), "unexpected: {err}");
    }

    #[tokio::test]
    async fn agent_rejected_when_active_run_exists() {
        let e = engine();
        let _guard = ActiveRunGuard::acquire(
            &e,
            "wf-a",
            &WorkflowRunSource::Agent {
                owner: crate::runtime::Mode::Leader,
            },
        )
        .unwrap();
        let err = ActiveRunGuard::acquire(
            &e,
            "wf-b",
            &WorkflowRunSource::Agent {
                owner: crate::runtime::Mode::Leader,
            },
        )
        .err()
        .expect("gate should reject")
        .to_string();
        assert!(err.contains("工作流正在执行中"), "unexpected: {err}");
    }

    #[tokio::test]
    async fn concurrent_acquire_only_one_wins() {
        // check-then-set 在同一 std Mutex 临界区内：a 登记期间 b 并发 acquire 必然被拒
        let e = std::sync::Arc::new(engine());
        let (tx1, mut rx1) = tokio::sync::mpsc::channel::<()>(1);
        let (tx2, mut rx2) = tokio::sync::mpsc::channel::<()>(1);
        let e2 = e.clone();
        let a = tokio::spawn(async move {
            let _guard = ActiveRunGuard::acquire(&e, "wf-a", &WorkflowRunSource::Schedule)
                .expect("a should register");
            let _ = tx1.send(()).await; // 告知 b：a 已登记且 guard 仍存活
            let _ = rx2.recv().await; // 保持 guard 直到 b 尝试完
            true
        });
        let b = tokio::spawn(async move {
            let _ = rx1.recv().await;
            let ok = ActiveRunGuard::acquire(&e2, "wf-b", &WorkflowRunSource::Schedule).is_ok();
            let _ = tx2.send(()).await;
            ok
        });
        let (ra, rb) = (a.await.unwrap(), b.await.unwrap());
        assert!(
            ra && !rb,
            "concurrent acquire must admit exactly one run (a={ra}, b={rb})"
        );
    }
}
