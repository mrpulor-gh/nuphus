//! ext_agent — agent_dispatch 工具（唯一去程工具）
//!
//! Leader 把整条显式交互链（上板 → 进程捕获 → dispatch_steps 工具序列 → await 门铃）
//! 委托给内部机制确定性执行，不经 LLM 逐步调用。编排实现在桌面壳
//! （src-tauri/src/ext_agent/），本模块只注册工具定义并通过 ext_agent_bridge 桥接调用。
//!
//! 权限：注册归 ToolCategory::SystemAutomation（同 desktop_* 类），
//! 首次调用走既有 SecurityCheck 审批流 —— 这是 Nuphus 自己的工具权限，
//! 与外部 Agent 的确认态是两回事。

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};

impl ToolRegistry {
    pub(crate) fn register_agent_dispatch(&mut self) {
        self.register(ToolDef {
            name: "agent_dispatch".to_string(),
            description: "把任务派发给外部 Agent（Claude Code / opencode 等）。前置：Leader 已按 skill §2 启动 SOP 启动/核验外部 Agent 并持有其实况 PID。本工具一次完成：上板 brief（文末自动内嵌门铃契约原文）→ 对当次实况解析目标窗口 → 按 team.toml 的 dispatch_steps 确定性投递。返回 {ok:true, submitted, brief_path, window, steps} 或 {ok:false, failed_step, error, hint}。门铃事件为异步推送，会自动注入 Leader 上下文；工具不做同步等待。仅当需要与外部 Agent 显式交互时使用。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent": {
                        "type": "string",
                        "description": "team.toml 中登记的 agent key（如 opencode / claude-code）"
                    },
                    "task_id": {
                        "type": "string",
                        "description": "任务 id（仅字母数字-_ .，将作为门铃事件 id 后缀 {agent}::{task_id}）"
                    },
                    "brief": {
                        "type": "string",
                        "description": "任务 brief 全文（写入 {handoff}/{agent}/briefs/{task_id}-brief.md，并追加门铃契约）"
                    },
                    "pid": {
                        "type": "integer",
                        "description": "可选但推荐：Leader 手动启动外部 Agent 后记录的进程 PID。提供时按当次窗口列表校验存活并解析 hwnd；缺省时按 window_hint 当次扫描。禁止传入历史缓存值"
                    },
                    "project": {
                        "type": "string",
                        "description": "可选：产物子目录名（对齐 read.md 约定「产物写 projects/{project}/」）"
                    },
                    "message": {
                        "type": "string",
                        "description": "可选：覆盖模板 —— 实际投递给外部 Agent 的指令文本；缺省用 brief 渲染后的任务指令（占位符 {task_id}/{brief_path} 已替换）"
                    }
                },
                "required": ["agent", "task_id", "brief"]
            }),
            category: ToolCategory::SystemAutomation,
            executor: |params, _ctx| {
                match crate::ext_agent_bridge::dispatch(params) {
                    Ok(json) => Ok(crate::ToolResult::success(json)),
                    Err(e) => Ok(crate::ToolResult::failure(format!("agent_dispatch 失败：{e}"))),
                }
            },
            depends_on: vec![],
        });
    }
}