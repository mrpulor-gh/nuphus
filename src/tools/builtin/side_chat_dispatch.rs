//! side_chat_dispatch — Leader 派发临时伙伴工具（Leader 独占）
//!
//! 内部长任务拆解并行时，Leader 把子任务 brief 经本工具直接注入伙伴执行体：
//! 选壳 / 取壳 / 执行 / 交付闭环全在桌面壳（src-tauri/src/commands/side_chat.rs
//! 的 `side_chat_dispatch`），本模块只注册工具定义并通过 side_chat_bridge
//! 桥接调用——伙伴结论经既有交付门禁自动回 Leader。
//!
//! 权限：注册归 ToolCategory::SystemAutomation（同 task_dispatch / agent_dispatch
//! 的派发族口径），且只在 `register_leader_only_tools` 注册——Exec 与伙伴的
//! 工具面（register_base_tools）天然不含它，`exec_blocked_tools` 再留一道双保险。

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};

impl ToolRegistry {
    pub(crate) fn register_side_chat_dispatch(&mut self) {
        self.register(ToolDef {
            name: "side_chat_dispatch".to_string(),
            description: "把拆解出的子任务派给临时伙伴并行执行。仅当三者同时成立：内部长任务可拆解、并行确有效率、无用户在用伙伴。回执：{busy:true,reason:user_active}=用户正在伙伴面板执行，改走 Exec 或等待；{appended:true}=伙伴正在执行派发轮，brief 已作为追加任务入队（全系统唯一豁免：伙伴执行中仍接收新任务，非必要不追加）；{interrupted:true}=用户中途停止了伙伴执行（任务未完成），可重派或改走 Exec；其余为正常派发，伙伴结论自动回本会话。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "brief": {
                        "type": "string",
                        "description": "子任务指令：目标+约束+成功标准，自包含（伙伴无跨轮上下文）"
                    }
                },
                "required": ["brief"]
            }),
            category: ToolCategory::SystemAutomation,
            executor: |params, _ctx| {
                match crate::side_chat_bridge::dispatch(params) {
                    Ok(json) => Ok(crate::ToolResult::success(json)),
                    Err(e) => Ok(crate::ToolResult::failure(format!("side_chat_dispatch 失败：{e}"))),
                }
            },
            depends_on: vec![],
        });
    }
}
