//! 定时任务工具定义
//!
//! schedule_cron — WorkflowAgent 用此工具管理 workflow 定时调度。
//! 所有调度统一由 SchedulerEngine 执行，持久化到 .nuphus/schedules.json。
//! 工具仅负责磁盘 CRUD，不直接操作 OS 调度器。

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;

impl ToolRegistry {
    pub(crate) fn register_schedule_cron(&mut self) {
        self.register(ToolDef {
            name: "schedule_cron".to_string(),
            description: "Manage workflow cron schedules (list/add/remove). Changes persist and take effect immediately.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "add", "remove"], "description": "list: 列出所有调度 / add: 新增调度 / remove: 删除调度" },
                    "workflow_id": { "type": "string", "description": "工作流 ID（add/remove 时必填）" },
                    "cron": { "type": "string", "description": "5-field cron 表达式，如 '0 9 * * *'（add 时必填）" },
                    "timezone": { "type": "string", "description": "IANA 时区，默认 UTC（add 时可选）" },
                    "inputs": { "type": "object", "description": "目标工作流已声明的输入快照（add 时可选；默认值在触发时读取）" }
                },
                "required": ["action"]
            }),
            category: ToolCategory::SystemAutomation,
            executor: |params, ctx| {
                let Some(callback) = &ctx.schedule_tool else {
                    return Ok(ToolResult::failure(
                        "调度引擎尚未连接，无法管理定时任务".to_string(),
                    ));
                };
                callback(params)
            },
            depends_on: vec![],
        });
    }
}
