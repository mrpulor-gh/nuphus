//! events.rs — Workflow 事件定义和 EventBus
//!
//! 所有 Workflow 内部事件通过 broadcast channel 发送，
//! 前端通过 listen('workflow-event', ...) 接收。

use crate::workflow::types::{RunStatus, StepRunStatus};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// 工作流事件通道容量
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Workflow 事件 — 所有阶段的状态变更通过此枚举流式通知
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WorkflowEvent {
    // ── 运行阶段 ──
    RunStarted {
        run_id: String,
        workflow_id: String,
    },
    StepRunStarted {
        step_id: String,
        step_name: String,
        depth: u32,
        kind: String,
    },
    StepRunOutput {
        step_id: String,
        text: String,
    },
    StepRunCompleted {
        step_id: String,
        step_name: String,
        status: StepRunStatus,
        depth: u32,
    },
    StepRunRetry {
        step_id: String,
        attempt: u32,
    },
    StepRunPaused {
        step_id: String,
        step_name: String,
        reason: String,
    },
    RunCompleted {
        run_id: String,
        status: RunStatus,
    },
    SubWorkflowStarted {
        workflow_id: String,
        workflow_name: String,
    },
    SubWorkflowCompleted {
        workflow_id: String,
        workflow_name: String,
        success: bool,
    },

    // ── 异常 ──
    Error {
        message: String,
    },

    // ── 通用 ──
    StatusChange {
        status: String,
    },
}

/// 事件总线 — 包装 broadcast channel
///
/// WorkflowEngine 内部持有 sender（写），
/// 前端监听侧持有 receiver（读）。
#[derive(Debug)]
pub struct EventBus {
    tx: broadcast::Sender<WorkflowEvent>,
}

impl EventBus {
    /// 创建新的事件总线
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self { tx }
    }

    /// 获取发送端（用于 emit）
    pub fn sender(&self) -> broadcast::Sender<WorkflowEvent> {
        self.tx.clone()
    }

    /// 获取接收端（用于前端事件订阅）
    pub fn subscribe(&self) -> broadcast::Receiver<WorkflowEvent> {
        self.tx.subscribe()
    }

    /// 发射一个事件（所有 receiver 收到）
    pub fn emit(&self, event: WorkflowEvent) {
        // 通道接近满载时预警，避免静默丢弃
        if self.tx.receiver_count() > 0 && self.tx.len() >= 200 {
            tracing::warn!(
                "EventBus channel near capacity: {}/{} events queued",
                self.tx.len(),
                EVENT_CHANNEL_CAPACITY
            );
        }
        if let Err(e) = self.tx.send(event) {
            tracing::warn!("EventBus::emit failed (no active receivers): {e}");
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
