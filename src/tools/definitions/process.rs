//! 进程/任务工具定义
//!
//! 包含任务派发、进程列表、进程终止等 ToolDef 注册方法。

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;

impl ToolRegistry {
    pub(crate) fn register_task_dispatch(&mut self) {
        self.register(ToolDef {
            name: "task_dispatch".to_string(),
            description: "Dispatch a sub-task to ExecAgent. Returns status, summary and the ledger run id. ExecAgent has NO desktop/browser automation tools — use for long/multi-step tasks, project_analysis, code_generation, debug_diagnose, file_operation, research_query, and scripting_exec. Structure the brief per the dispatch spec (task/context/quality-baseline/anti-patterns/output).".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short human-readable task title (shown in the task panel). Required for a readable panel; omit only for throwaway one-liners." },
                    "brief": { "type": "string", "description": "Full directive for Exec: what to achieve, constraints, expected output format. Be thorough — this is Exec's primary directive. Formerly named `description` (still accepted)." },
                    "goal_type": { "type": "string", "enum": ["project_analysis", "code_generation", "debug_diagnose", "file_operation", "research_query", "scripting_exec"], "default": "file_operation", "description": "project_analysis: understand codebase structure. code_generation: implement or modify code. debug_diagnose: fix bugs. file_operation: manage files. research_query: web search. scripting_exec: run commands/tests (ExecAgent cannot use desktop/browser tools)" },
                    "plan_path": { "type": "string", "description": "Explicit .plan.md path to inject as execution guide" },
                    "task_no": { "type": "integer", "description": "Optional provenance label: which numbered direction of the plan this run belongs to. Display-only — omit if unsure, it never affects execution or lifecycle." }
                },
                "required": ["brief"]
            }),
            category: ToolCategory::SystemAutomation,
            executor: |_params, _ctx| {
                Ok(ToolResult::failure("task_dispatch is handled by Leader, not available in ExecAgent context. Sub-tasks are dispatched at the Leader level only."))
            },
            depends_on: vec![],
        });
    }

    pub(crate) fn register_workflow_validate(&mut self) {
        self.register(ToolDef {
            name: "workflow_validate".to_string(),
            description: "静态校验工作流（步骤合法性 / 工具名 / 必填参数 / 变量引用 / call 环），返回 JSON 报告。设计完成后先 validate 再 run。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "工作流 ID" }
                },
                "required": ["id"]
            }),
            category: ToolCategory::Core,
            executor: |_params, _ctx| {
                Ok(ToolResult::success("workflow_validate handled by react_loop"))
            },
            depends_on: vec![],
        });
    }

    pub(crate) fn register_workflow_run(&mut self) {
        self.register(ToolDef {
            name: "workflow_run".to_string(),
            description: "按 id 运行已保存工作流，inputs 注入变量池（可参数化复用同一工作流）。失败返回 failed/error/completed_steps；同一 id 复跑会自动跳过已完成步骤续跑——解决阻塞后重跑同一 id，不要新建或复制工作流。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "工作流 ID" },
                    "inputs": { "type": "object", "description": "运行时参数（key→value），注入工作流变量池。如 {\"contact\": \"张三\"} 使步骤中 {{contact}} 可用" }
                },
                "required": ["id"]
            }),
            category: ToolCategory::Core,
            executor: |_params, _ctx| {
                Ok(ToolResult::success("workflow_run handled by react_loop"))
            },
            depends_on: vec![],
        });
    }

    pub(crate) fn register_process_list(&mut self) {
        self.register(ToolDef {
            name: "process_list".to_string(),
            description: "List running OS processes".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "filter": { "type": "string", "description": "Optional: filter by process name (case-insensitive substring match)" },
                    "limit": { "type": "integer", "default": 50, "description": "Max number of processes to return" }
                }
            }),
            category: ToolCategory::SystemAutomation,
            executor: |params, _ctx| {
                let filter = params.get("filter").and_then(|v| v.as_str());
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;

                #[cfg(target_os = "windows")]
                let output = std::process::Command::new("tasklist")
                    .args(["/FO", "CSV", "/NH"])
                    .output()
                    .map_err(|e| format!("tasklist failed: {}", e))?;

                #[cfg(target_os = "linux")]
                let output = std::process::Command::new("ps")
                    .args(["-eo", "pid,comm,pcpu,pmem", "--sort=-pcpu"])
                    .output()
                    .map_err(|e| format!("ps failed: {}", e))?;

                #[cfg(target_os = "macos")]
                let output = std::process::Command::new("ps")
                    .args(["-eo", "pid,comm,%cpu,%mem", "-r"])
                    .output()
                    .map_err(|e| format!("ps failed: {}", e))?;

                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if !output.status.success() && !stderr.is_empty() {
                    return Ok(ToolResult::failure(format!("process list error: {}", stderr)));
                }

                #[cfg(target_os = "windows")]
                let processes: Vec<String> = {
                    stdout.lines()
                        .filter_map(|line| {
                            let parts: Vec<&str> = line.split(',').collect();
                            if parts.len() >= 2 {
                                let name = parts[0].trim_matches('"');
                                let pid = parts[1].trim_matches('"');
                                let mem = parts.get(4).map(|s| s.trim_matches('"')).unwrap_or("N/A");
                                Some(format!("{} | PID:{} | Mem:{}", name, pid, mem))
                            } else {
                                None
                            }
                        })
                        .filter(|p| {
                            if let Some(f) = filter {
                                p.to_lowercase().contains(&f.to_lowercase())
                            } else {
                                true
                            }
                        })
                        .take(limit)
                        .collect()
                };

                #[cfg(not(target_os = "windows"))]
                let processes: Vec<String> = {
                    stdout.lines().skip(1)
                        .filter_map(|line| {
                            let parts: Vec<&str> = line.split_whitespace().collect();
                            if parts.len() >= 4 {
                                let pid = parts[0];
                                let name = parts[1];
                                let cpu = parts[2];
                                let mem = parts[3];
                                Some(format!("{} | PID:{} | CPU:{}% | MEM:{}%", name, pid, cpu, mem))
                            } else {
                                None
                            }
                        })
                        .filter(|p| {
                            if let Some(f) = filter {
                                p.to_lowercase().contains(&f.to_lowercase())
                            } else {
                                true
                            }
                        })
                        .take(limit)
                        .collect()
                };

                if processes.is_empty() {
                    Ok(ToolResult::success("No matching processes found.".to_string()))
                } else {
                    Ok(ToolResult::success(processes.join("\n")))
                }
            },
            depends_on: vec![],
        });
    }

    pub(crate) fn register_process_kill(&mut self) {
        self.register(ToolDef {
            name: "process_kill".to_string(),
            description: "Terminate a process by PID or name. 禁止kill nuphus自身进程".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pid": { "type": "integer", "description": "Process ID to kill" },
                    "name": { "type": "string", "description": "Process name to kill (kills all matching)" },
                    "force": { "type": "boolean", "default": false, "description": "Force kill (/F on Windows, -9 on Unix)" }
                }
            }),
            category: ToolCategory::SystemAutomation,
            executor: |params, _ctx| {
                let pid = params.get("pid").and_then(|v| v.as_u64());
                let name = params.get("name").and_then(|v| v.as_str());
                let force = params.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

                // ── Protect nuphus自身进程 ──
                let self_pid = std::process::id() as u64;
                if let Some(pid) = pid {
                    if pid == self_pid {
                        return Ok(ToolResult::failure("禁止kill自身进程，请遵守提示词规则。"));
                    }
                }
                if let Some(name) = name {
                    if let Ok(exe) = std::env::current_exe() {
                        if let Some(exe_name) = exe.file_stem().and_then(|n| n.to_str()) {
                            // Compare case-insensitively on Windows
                            let name_lower = name.to_lowercase();
                            let exe_lower = exe_name.to_lowercase();
                            if name_lower == exe_lower
                                || name_lower == format!("{}.exe", exe_lower)
                                || name_lower.contains(&exe_lower)
                            {
                                return Ok(ToolResult::failure("禁止kill自身进程，请遵守提示词规则。"));
                            }
                        }
                    }
                }

                #[cfg(target_os = "windows")]
                {
                    if let Some(pid) = pid {
                        let mut cmd = std::process::Command::new("taskkill");
                        cmd.arg("/PID").arg(pid.to_string());
                        if force { cmd.arg("/F"); }
                        let output = cmd.output()
                            .map_err(|e| format!("taskkill failed: {}", e))?;
                        if output.status.success() {
                            let msg = String::from_utf8_lossy(&output.stdout);
                            Ok(ToolResult::success(format!("Killed PID {}: {}", pid, msg.trim())))
                        } else {
                            // 修复前不检查退出码：taskkill ERROR（进程不存在/拒绝访问）
                            // 混在 stdout 里也回成功文本，调用方收到假成功
                            let err = String::from_utf8_lossy(&output.stderr);
                            Ok(ToolResult::failure(format!("taskkill failed for PID {}: {}", pid, err.trim())))
                        }
                    } else if let Some(name) = name {
                        let mut cmd = std::process::Command::new("taskkill");
                        cmd.arg("/IM").arg(name);
                        if force { cmd.arg("/F"); }
                        let output = cmd.output()
                            .map_err(|e| format!("taskkill failed: {}", e))?;
                        if output.status.success() {
                            let msg = String::from_utf8_lossy(&output.stdout);
                            Ok(ToolResult::success(format!("Killed process '{}': {}", name, msg.trim())))
                        } else {
                            let err = String::from_utf8_lossy(&output.stderr);
                            Ok(ToolResult::failure(format!("taskkill failed for '{}': {}", name, err.trim())))
                        }
                    } else {
                        Ok(ToolResult::failure("Either pid or name must be provided"))
                    }
                }

                #[cfg(not(target_os = "windows"))]
                {
                    if let Some(pid) = pid {
                        let mut cmd = std::process::Command::new("kill");
                        if force { cmd.arg("-9"); }
                        cmd.arg(pid.to_string());
                        let output = cmd.output()
                            .map_err(|e| format!("kill failed: {}", e))?;
                        if output.status.success() {
                            Ok(ToolResult::success(format!("Killed PID {}", pid)))
                        } else {
                            let err = String::from_utf8_lossy(&output.stderr);
                            Ok(ToolResult::failure(format!("kill failed for PID {}: {}", pid, err.trim())))
                        }
                    } else if let Some(name) = name {
                        let output = std::process::Command::new("pkill")
                            .args(if force { vec!["-9", name] } else { vec![name] })
                            .output()
                            .map_err(|e| format!("pkill failed: {}", e))?;
                        if output.status.success() {
                            Ok(ToolResult::success(format!("Killed process '{}'", name)))
                        } else {
                            // pkill 无匹配进程时退出码 1——如实报失败，杜绝假成功
                            let err = String::from_utf8_lossy(&output.stderr);
                            Ok(ToolResult::failure(format!("pkill found no process '{}' or failed: {}", name, err.trim())))
                        }
                    } else {
                        Ok(ToolResult::failure("Either pid or name must be provided"))
                    }
                }
            },
            depends_on: vec![],
        });
    }
}
