//! SignalState — pause/security/workflow 会话级信号状态（显式注入，无全局 static）
//!
//! 原为 `pub static APP_STATE: LazyLock<RwLock<AppState>>` 全局单例；
//! PR-2（AppState 合并，设计见 docs/internal/2026-08-06-appstate-merge-design.md）
//! 改为 `SharedSignals = Arc<RwLock<SignalState>>` 共享句柄：
//! - 唯一实例由 src-tauri `AppState.signals` 持有并注入各子系统
//! - ToolRegistry 携带句柄（Clone 共享 Arc），ReactAgent/SubTaskRunner/WorkflowAgent
//!   经 `tools.signals()` 访问；工具 handler 经 `ToolCtx.signals` 访问
//! - 锁粒度与字段结构与原 APP_STATE 完全一致（一把 RwLock 管全部字段）
//!
//! ## 不纳入 SignalState 的全局 static（保持独立的理由）
//!
//! - WORKFLOW_USER_CANCELLED (AtomicBool) — Tauri 命令层 → Core 单向信号
//! - 所有 OnceLock 基础设施（EMBEDDER_LOCK, JIEBA, DB_PATH, POOL 等）

use crate::agent::pause::PauseDecision;
use crate::security::approval::PendingApproval;
use crate::security::user_input::PendingInput;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Instant;

/// 共享信号句柄 — 全进程唯一实例由 src-tauri AppState 持有
pub type SharedSignals = Arc<RwLock<SignalState>>;

/// 创建新的共享信号句柄（src-tauri AppState 构造时调用一次；
/// 测试/CLI 可各自创建独立实例）
pub fn new_shared_signals() -> SharedSignals {
    Arc::new(RwLock::new(SignalState::default()))
}

/// 执行阶段 —— **全链路唯一权威执行态**。
///
/// 收敛前的执行态分散在 5 个互不相通的来源（前端 `isProcessing` / 前端 `completed` /
/// 后端 `busy` / 后端 `guard_switch` 的 `can_switch` / rail 的 OR 派生），导致
/// 「主循环已退出、收尾仍在进行」这一窗口内，追加指令被静默入队却永不被消费。
/// 现在所有读写都走本枚举（存储位置见 [`SignalState::execution_stage`]）：
///
/// - `Idle` —— 无执行占用（旧 `busy == false`）
/// - `Running` —— 主循环在迭代中：**追加指令可被注入**（轮次边界 drain 仍在跑）
/// - `Finalizing` —— 主循环已退出、正在收尾（记忆落盘 / 自动提炼 / 镜像回填）：
///   追加指令**不可注入**（没有消费方），必须显式拒绝而不是静默入队
///
/// 置位规则（唯一写入方，勿在别处直接赋值）：
/// 1. 进入执行体主循环 → `Running`（react_loop / sub_task_loop / workflow_agent）
/// 2. 顶层轮次主循环退出、收尾工作开始之前 → `Finalizing`（process.rs / retry.rs 轮次所有者）
/// 3. 轮次任务结束（TaskBusyGuard/BusyGuard/RefineGuard drop）→ `Idle`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionStage {
    #[default]
    Idle,
    Running,
    Finalizing,
}

impl ExecutionStage {
    /// 稳定字符串标识（前端 / HTTP 下发用，勿随意改名）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Finalizing => "finalizing",
        }
    }

    /// 后端是否仍被任务占用（= 旧 `busy` 语义）：`Running ∨ Finalizing`。
    pub fn is_busy(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// 追加指令是否可被注入：**仅 `Running`**——只有主循环迭代边界会 drain，
    /// `Finalizing` 阶段入队即永久滞留（收尾结束后的残留还会锁死 guard_switch）。
    pub fn accepts_append(&self) -> bool {
        matches!(self, Self::Running)
    }

    /// 提交消息时的处置分流 —— 双入口（桌面 `submit_user_message` / 手机 `/message`）
    /// 共用的**唯一判定入口**，与 [`Self::accepts_append`] 同源。
    pub fn submit_disposition(&self) -> SubmitDisposition {
        match self {
            Self::Idle => SubmitDisposition::NewRound,
            Self::Running => SubmitDisposition::Append,
            Self::Finalizing => SubmitDisposition::RejectFinalizing,
        }
    }
}

/// 提交消息时按执行阶段得到的处置（见 [`ExecutionStage::submit_disposition`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitDisposition {
    /// 空闲：正常受理，开启新一轮执行。
    NewRound,
    /// 主循环在迭代中：入队为追加指令，由当前迭代边界 drain 注入。
    Append,
    /// 主循环已退出、正在收尾：**拒绝**——无消费方会再 drain，入队即永久滞留。
    /// 调用方必须显式告知用户（前端把原文退回输入框），且**不得**写去重基准
    /// （`guard.last_message`），否则用户重发会被判重复而丢弃。
    RejectFinalizing,
}

/// 会话级信号状态（pause/security/workflow）
#[derive(Debug, Default)]
pub struct SignalState {
    // ── 执行态（唯一真相源）──
    /// 当前执行阶段。见 [`ExecutionStage`]；读写统一走
    /// [`SignalState::execution_stage`] / [`SignalState::set_execution_stage`]。
    pub execution_stage: ExecutionStage,

    // ── Pause 子系统 ──
    /// 暂停决策 (action_id → PauseDecision)
    pub pause_decisions: HashMap<String, PauseDecision>,
    /// 当前暂停 action_id
    pub pause_action_id: Option<String>,
    /// 执行中追加消息的真实消费队列，按当前 agent 单一路由。
    pub append_queue: Vec<String>,
    /// 待注入提示（状态变化类：项目目录切换等）。
    ///
    /// 与 append_queue 同构：写入后由下一个轮次边界 drain 注入一次即消费 ——
    /// 因此**执行中**（agent 被 take 出槽）写入同样有效，提示会随下一轮对话的
    /// user 消息带出，既不会丢失也不会重复注入。
    pub pending_notices: Vec<String>,

    // ── Security 子系统 ──
    pub security: SecurityState,

    // ── Workflow 子系统 ──
    /// 当前活跃 workflow ID
    pub active_workflow_id: Option<String>,
}

/// 安全子系统状态
#[derive(Debug, Default)]
pub struct SecurityState {
    /// 安全确认结果 (action_id → (approved, timestamp))
    pub security_results: HashMap<String, (bool, Instant)>,
    /// 会话级授权工具集
    pub session_approved_tools: HashSet<String>,
    /// 待批准操作 (action_id → (PendingApproval, Instant))
    pub pending_approvals: HashMap<String, (PendingApproval, Instant)>,
    /// 待用户输入 (action_id → StoredInput)
    pub pending_inputs: HashMap<String, StoredInput>,
}

/// 内部用的 StoredInput（不导出）
#[derive(Debug)]
pub struct StoredInput {
    pub input: PendingInput,
    pub response: Option<String>,
    pub timestamp: Instant,
}

// ── 便捷访问器（简化调用方代码，自动处理 poison → into_inner） ──

impl SignalState {
    /// 读访问
    pub fn read(signals: &SharedSignals) -> std::sync::RwLockReadGuard<'_, SignalState> {
        signals.read().unwrap_or_else(|e| e.into_inner())
    }

    /// 写访问
    pub fn write(signals: &SharedSignals) -> std::sync::RwLockWriteGuard<'_, SignalState> {
        signals.write().unwrap_or_else(|e| e.into_inner())
    }

    /// 写入一条「待注入提示」（状态变化类，如项目目录切换）。
    ///
    /// 由下一个轮次边界（react_loop / workflow_agent 的 reminders 注入位）drain
    /// 一次即消费 —— 执行中（agent 不在槽）调用同样有效，提示不会丢失。
    pub fn push_notice(signals: &SharedSignals, text: String) {
        Self::write(signals).pending_notices.push(text);
    }

    /// 读取当前执行阶段（唯一真相源）。
    pub fn execution_stage(signals: &SharedSignals) -> ExecutionStage {
        Self::read(signals).execution_stage
    }

    /// 置位执行阶段，返回旧值。
    ///
    /// 互斥由本结构的 `RwLock` 提供：所有转换（含「空闲才抢占」的 CAS 语义）都在
    /// 同一把锁内完成，故不需要额外的原子量。锁内不做任何 await / 跨锁调用。
    pub fn set_execution_stage(signals: &SharedSignals, stage: ExecutionStage) -> ExecutionStage {
        std::mem::replace(&mut Self::write(signals).execution_stage, stage)
    }

    /// 轮询等待执行体**真正退出**（阶段回到 `Idle`）。
    ///
    /// 返回 `true` = 旧执行体已退出；`false` = 超时仍占用。
    /// 供 `force_reset` 的「真终止」语义使用：**只观察、不改阶段** ——
    /// 超时不得把阶段写成 `Idle`（那正是「Idle 但旧任务仍在跑」的假解锁，
    /// 会让用户开出与旧执行体并存的新轮次 → 双跑 → panic）。
    pub async fn wait_until_idle(
        signals: &SharedSignals,
        timeout: std::time::Duration,
        poll_interval: std::time::Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if !Self::execution_stage(signals).is_busy() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(poll_interval).await;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// 执行阶段转换 + 语义：Idle → Running → Finalizing → Idle，
    /// 覆盖「Finalizing 拒收追加」「Running/Finalizing 都算占用」两条核心约束。
    #[test]
    fn execution_stage_transitions_and_semantics() {
        let signals = new_shared_signals();
        assert_eq!(
            SignalState::execution_stage(&signals),
            ExecutionStage::Idle,
            "新建共享信号默认空闲"
        );

        // 空闲：受理新轮次，不算占用
        assert!(!ExecutionStage::Idle.is_busy());
        assert!(!ExecutionStage::Idle.accepts_append());
        assert_eq!(
            ExecutionStage::Idle.submit_disposition(),
            SubmitDisposition::NewRound
        );

        // 进入主循环 → Running（追加可注入）
        let prev = SignalState::set_execution_stage(&signals, ExecutionStage::Running);
        assert_eq!(prev, ExecutionStage::Idle, "置位返回旧阶段");
        assert_eq!(
            SignalState::execution_stage(&signals),
            ExecutionStage::Running
        );
        assert!(ExecutionStage::Running.is_busy());
        assert!(ExecutionStage::Running.accepts_append());
        assert_eq!(
            ExecutionStage::Running.submit_disposition(),
            SubmitDisposition::Append
        );

        // 主循环退出、收尾之前 → Finalizing：仍占用，但**拒收追加**
        SignalState::set_execution_stage(&signals, ExecutionStage::Finalizing);
        let finalizing = SignalState::execution_stage(&signals);
        assert_eq!(finalizing, ExecutionStage::Finalizing);
        assert!(
            finalizing.is_busy(),
            "Finalizing 仍是占用态（终止按钮 / 会话切换守卫语义）"
        );
        assert!(
            !finalizing.accepts_append(),
            "Finalizing 无消费方，必须拒收追加（否则入队即永久滞留）"
        );
        assert_eq!(
            finalizing.submit_disposition(),
            SubmitDisposition::RejectFinalizing,
            "收尾期提交 → 显式拒绝（前端据此退回输入框）"
        );

        // 轮次结束 → Idle
        SignalState::set_execution_stage(&signals, ExecutionStage::Idle);
        assert!(!SignalState::execution_stage(&signals).is_busy());
    }

    /// 阶段字符串标识是前端 / HTTP 的协议取值，改名即破坏双端契约。
    #[test]
    fn execution_stage_str_is_stable_wire_contract() {
        assert_eq!(ExecutionStage::Idle.as_str(), "idle");
        assert_eq!(ExecutionStage::Running.as_str(), "running");
        assert_eq!(ExecutionStage::Finalizing.as_str(), "finalizing");
    }

    /// `wait_until_idle`：旧执行体退出（阶段回 Idle）后才返回 true。
    ///
    /// 这条是 `force_reset` 真终止语义的地基：置 Idle 前必须等到旧执行体退出，
    /// 否则用户能立刻开出与旧轮次并存的新轮次（双跑 → panic）。
    #[tokio::test]
    async fn wait_until_idle_waits_for_body_exit() {
        let signals = new_shared_signals();
        SignalState::set_execution_stage(&signals, ExecutionStage::Running);
        let body_signals = signals.clone();
        let body = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            SignalState::set_execution_stage(&body_signals, ExecutionStage::Idle);
        });

        let started = Instant::now();
        let exited = SignalState::wait_until_idle(
            &signals,
            std::time::Duration::from_secs(3),
            std::time::Duration::from_millis(10),
        )
        .await;
        let waited = started.elapsed();
        body.await.expect("body joins");
        assert!(exited, "旧执行体退出后应返回 true");
        assert!(
            waited >= std::time::Duration::from_millis(80),
            "必须真的等到旧执行体退出（wait = {waited:?}）"
        );
        assert_eq!(SignalState::execution_stage(&signals), ExecutionStage::Idle);
    }

    /// 超时路径：返回 false 且**不改阶段** —— 绝不做假解锁。
    #[tokio::test]
    async fn wait_until_idle_times_out_without_fake_unlock() {
        let signals = new_shared_signals();
        SignalState::set_execution_stage(&signals, ExecutionStage::Finalizing);
        let started = Instant::now();
        let exited = SignalState::wait_until_idle(
            &signals,
            std::time::Duration::from_millis(60),
            std::time::Duration::from_millis(10),
        )
        .await;
        assert!(!exited, "旧执行体未退出 → 必须返回 false");
        assert!(started.elapsed() >= std::time::Duration::from_millis(60));
        assert_eq!(
            SignalState::execution_stage(&signals),
            ExecutionStage::Finalizing,
            "超时不得写阶段：Idle 但旧任务仍在跑 = 双跑入口"
        );
    }

    /// 空闲时立即返回（force_reset 在空闲态不做无谓等待）。
    #[tokio::test]
    async fn wait_until_idle_returns_immediately_when_idle() {
        let signals = new_shared_signals();
        let started = Instant::now();
        assert!(
            SignalState::wait_until_idle(
                &signals,
                std::time::Duration::from_secs(1),
                std::time::Duration::from_millis(10),
            )
            .await
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(50),
            "空闲应零等待"
        );
    }
}
