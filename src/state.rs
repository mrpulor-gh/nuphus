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
//! - PENDING_APPEND (Mutex<Vec<String>>) — 跨 session 追加指令队列
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

/// 会话级信号状态（pause/security/workflow）
#[derive(Debug, Default)]
pub struct SignalState {
    // ── Pause 子系统 ──
    /// 暂停决策 (action_id → PauseDecision)
    pub pause_decisions: HashMap<String, PauseDecision>,
    /// 当前暂停 action_id
    pub pause_action_id: Option<String>,

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
}
