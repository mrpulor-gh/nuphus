//! tool_lifecycle.rs — 工具调用生命周期守卫（跨执行体复用）
//!
//! ## 事故（2026-10-26）
//!
//! HUD 计时永不退表：`system_shell` 设了数十秒 timeout、超时转后台之后，HUD 停在
//! running 相位不再消失。
//!
//! ## 机制与根因
//!
//! HUD 仲裁（`hud_arbiter`）把 `ToolCallStart` / `ToolCallEnd` 当作工具生命周期的
//! **唯一真相源**：Start 注册进活跃集，End 注销并裁决。若某个 Start 没有配对 End
//! （早退 `?` 传播 / `continue` / 提前 `return` 都会绕开循环末尾的 End 发送点），
//! 活跃集永久残留 —— 之后**每一次** ToolCallEnd 都会把显示切回残留条目的 running
//! （`ToolEndAction::SwitchRunning`；补发帧 seq 更大，必然被前端 seq 单调消费接受），
//! 而 running 相位没有 auto-hide（只有终态帧才排 15s 自动隐藏）→ HUD 一直显示。
//!
//! ## 守卫语义
//!
//! guard 紧跟 Start 创建（`arm`），正常路径 emit 真实 End 后 `disarm`；任何提前离开
//! 作用域（含 Err 传播、循环 `continue`）都会在 `Drop` 时补发一条 stub End，保证
//! Start/End 永远配对 —— 包括未来新增的早退分支。
//!
//! ## 适用范围（重要）
//!
//! 只适用于「Start 与 End 在同一个作用域内」的执行体：
//!   · `react_loop`（主轮）—— Start/End 同一次迭代体内；
//!   · `workflow_agent`（工作流轮）—— 同一次工具循环内。
//!
//! 不适用于 `sub_task_loop`：它的 Start 在第一个循环发出，End 在随后的执行循环 /
//! 缓存重放循环发出，守卫无法跨越循环边界，那里的配对必须逐个出口显式保证。

use std::sync::Arc;
use std::time::Instant;

use crate::agent::events::{EventEmitter, NuphusEvent};

/// 工具调用生命周期守卫。详见模块文档。
pub(crate) struct ToolEndGuard {
    emitter: Option<Arc<dyn EventEmitter>>,
    call_id: String,
    tool: String,
    started: Instant,
    armed: bool,
}

impl ToolEndGuard {
    /// 在 `ToolCallStart` 发出之后立刻挂守卫。
    pub(crate) fn arm(
        emitter: Option<&Arc<dyn EventEmitter>>,
        call_id: &str,
        tool: &str,
        started: Instant,
    ) -> Self {
        Self {
            emitter: emitter.cloned(),
            call_id: call_id.to_string(),
            tool: tool.to_string(),
            started,
            armed: true,
        }
    }

    /// 正常路径已发真实 End → 撤销兜底（防双发）。
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ToolEndGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(emitter) = &self.emitter else {
            return;
        };
        emitter.emit(NuphusEvent::ToolCallEnd {
            call_id: self.call_id.clone(),
            tool_name: self.tool.clone(),
            success: false,
            duration_ms: self.started.elapsed().as_millis() as u64,
            output_preview: "tool call aborted before completion".to_string(),
            output_full_size: 0,
            is_truncated: false,
            error: Some("aborted before completion".to_string()),
            from_task: false,
        });
        tracing::warn!(
            tool = %self.tool,
            "tool call exited early — stub ToolCallEnd emitted by lifecycle guard"
        );
    }
}
