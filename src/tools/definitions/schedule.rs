//! 定时任务工具定义
//!
//! schedule_cron — WorkflowAgent 用此工具管理 workflow 定时调度。
//! 所有调度统一由 SchedulerEngine 执行，持久化到 .nuphus/schedules.json。
//! 工具仅负责磁盘 CRUD，不直接操作 OS 调度器。

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::workflow::scheduler::SchedulerEngine;
use crate::ToolResult;

impl ToolRegistry {
    pub(crate) fn register_schedule_cron(&mut self) {
        self.register(ToolDef {
            name: "schedule_cron".to_string(),
            description: "Manage workflow cron schedules (list/add/remove). Changes persist to disk and take effect after restart.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "add", "remove"], "description": "list: 列出所有调度 / add: 新增调度 / remove: 删除调度" },
                    "workflow_id": { "type": "string", "description": "工作流 ID（add/remove 时必填）" },
                    "cron": { "type": "string", "description": "5-field cron 表达式，如 '0 9 * * *'（add 时必填）" },
                    "timezone": { "type": "string", "description": "时区，默认 UTC（add 时可选）" }
                },
                "required": ["action"]
            }),
            category: ToolCategory::SystemAutomation,
            executor: |params, _ctx| {
                let action = params.get("action").and_then(|v| v.as_str()).unwrap_or("list");

                match action {
                    "list" => {
                        let data = SchedulerEngine::load_persisted();
                        if data.schedules.is_empty() {
                            return Ok(ToolResult::success("暂无定时调度任务。".to_string()));
                        }
                        let lines: Vec<String> = data.schedules.iter().map(|(id, cfg)| {
                            let status = if cfg.enabled { "启用" } else { "禁用" };
                            format!("- `{}` — cron: {} (tz: {}, {})",
                                id, cfg.cron, cfg.timezone, status)
                        }).collect();
                        Ok(ToolResult::success(format!(
                            "定时调度 ({} 个):\n{}\n\n调度由 SchedulerEngine 执行，重启后自动恢复。",
                            data.schedules.len(),
                            lines.join("\n")
                        )))
                    },
                    "add" => {
                        let workflow_id = match params.get("workflow_id").and_then(|v| v.as_str()) {
                            Some(id) => id,
                            None => return Ok(ToolResult::failure("缺少 workflow_id 参数".to_string())),
                        };
                        let cron = match params.get("cron").and_then(|v| v.as_str()) {
                            Some(c) => c,
                            None => return Ok(ToolResult::failure("缺少 cron 参数".to_string())),
                        };
                        let timezone = params.get("timezone")
                            .and_then(|v| v.as_str())
                            .unwrap_or("UTC");

                        let mut data = SchedulerEngine::load_persisted();
                        let config = crate::workflow::types::ScheduleConfig {
                            enabled: true,
                            cron: cron.to_string(),
                            timezone: timezone.to_string(),
                            label: None,
                        };
                        data.schedules.insert(workflow_id.to_string(), config);

                        let path = SchedulerEngine::persist_path();
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let json = serde_json::to_string_pretty(&data)
                            .map_err(|e| format!("序列化失败: {}", e))?;
                        std::fs::write(&path, &json)
                            .map_err(|e| format!("写入失败: {}", e))?;

                        Ok(ToolResult::success(format!(
                            "已为工作流 `{}` 添加定时调度: cron={} (tz={})。\n调度将在下次重启 Nuphus 后生效。",
                            workflow_id, cron, timezone
                        )))
                    },
                    "remove" => {
                        let workflow_id = match params.get("workflow_id").and_then(|v| v.as_str()) {
                            Some(id) => id,
                            None => return Ok(ToolResult::failure("缺少 workflow_id 参数".to_string())),
                        };

                        let mut data = SchedulerEngine::load_persisted();
                        if data.schedules.remove(workflow_id).is_none() {
                            return Ok(ToolResult::success(format!(
                                "工作流 `{}` 没有定时调度，无需删除。", workflow_id
                            )));
                        }

                        let path = SchedulerEngine::persist_path();
                        let json = serde_json::to_string_pretty(&data)
                            .map_err(|e| format!("序列化失败: {}", e))?;
                        std::fs::write(&path, &json)
                            .map_err(|e| format!("写入失败: {}", e))?;

                        Ok(ToolResult::success(format!(
                            "已移除工作流 `{}` 的定时调度。", workflow_id
                        )))
                    },
                    _ => Ok(ToolResult::failure(format!("未知操作: {}。可用: list/add/remove", action))),
                }
            },
            depends_on: vec![],
        });
    }
}
