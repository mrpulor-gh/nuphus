//! planner — 计划管理工具（理解传递介质）
//!
//! 核心理念：.plan.md 是 Leader 与 Exec 之间的理解传递文档，不是指令清单。
//! - Leader 传递的是"现状理解 + 约束 + 建议"，Exec 基于理解自主决策。
//! - Task 从"步骤规定"变为"理解片段"（背景 / 已知信息 / 约束 / 建议）。
//! - Exec 返回结构化摘要，Leader 审计时一眼扫完。

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolCtx, ToolDef, ToolRegistry};
use crate::utils::resolve_project_root;
use crate::ToolResult;
use std::path::PathBuf;

// ── 路径解析 ──

fn resolve_plan_path(raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        p
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    }
}

fn get_plan_dir(root: &std::path::Path, project: &str) -> PathBuf {
    // 优先从环境变量读取
    if let Ok(dir) = std::env::var("NUPHUS_PLAN_DIR") {
        return PathBuf::from(dir).join(project).join("plans");
    }

    // 从项目根下的 config.toml 读取 [planner] dir
    let config_path = root.join("config.toml");
    if config_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            if let Ok(doc) = content.parse::<toml::Value>() {
                if let Some(dir) = doc
                    .get("planner")
                    .and_then(|p| p.get("dir"))
                    .and_then(|d| d.as_str())
                {
                    return PathBuf::from(dir).join(project).join("plans");
                }
            }
        }
    }

    // fallback 默认路径
    root.join("src-tauri")
        .join("tasks")
        .join(project)
        .join("plans")
}

fn ensure_plan_dir(path: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir failed: {}", e))?;
    }
    Ok(())
}

// ── 内部数据结构（理解传递模型） ──

#[derive(Debug, Default)]
struct ParsedPlan {
    project: String,
    topic: String,
    requirement: String,
    status: String,
    goal_type: String,
    context: String,
    tasks: Vec<ParsedTask>,
}

#[derive(Debug, Default)]
struct ParsedTask {
    id: usize,
    name: String,
    understanding: String,
}

// 元信息字段定义: (字段名, Markdown前缀, 默认值)
const META_FIELDS: &[(&str, &str, &str)] = &[
    ("requirement", "- 需求来源:", ""),
    ("status", "- 状态:", "active"),
    ("project", "- 项目:", ""),
    ("goal_type", "- 目标类型:", "code_generation"),
];

fn meta_prefix(field: &str) -> &'static str {
    META_FIELDS
        .iter()
        .find(|(f, _, _)| *f == field)
        .map(|(_, p, _)| *p)
        .unwrap_or("")
}

fn meta_default(field: &str) -> &'static str {
    META_FIELDS
        .iter()
        .find(|(f, _, _)| *f == field)
        .map(|(_, _, d)| *d)
        .unwrap_or("")
}

// ── Markdown 解析器（新格式 + 向后兼容） ──

fn parse_plan_md(content: &str) -> Result<ParsedPlan, String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut plan = ParsedPlan::default();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // 标题行
        if line.starts_with("# 计划：") {
            let rest = line.trim_start_matches("# 计划：").trim();
            if let Some((proj, top)) = rest.split_once(" / ") {
                plan.project = proj.trim().to_string();
                plan.topic = top.trim().to_string();
            } else {
                plan.topic = rest.to_string();
            }
            i += 1;
            continue;
        }

        // 元信息
        if line == "## 元信息" {
            i += 1;
            while i < lines.len() {
                let l = lines[i].trim();
                if l == "---" {
                    break;
                }
                for (field, prefix, _) in META_FIELDS {
                    if let Some(v) = l.strip_prefix(prefix) {
                        match *field {
                            "requirement" => plan.requirement = v.trim().to_string(),
                            "status" => plan.status = v.trim().to_string(),
                            "project" => plan.project = v.trim().to_string(),
                            "goal_type" => plan.goal_type = v.trim().to_string(),
                            _ => {}
                        }
                        break;
                    }
                }
                i += 1;
            }
            continue;
        }

        // 验收（可选）
        if line == "## 验收" {
            i += 1;
            let mut goal_lines = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim();
                if l == "---" {
                    break;
                }
                goal_lines.push(lines[i]);
                i += 1;
            }
            // 目标内容合并到 requirement（如果没有单独的需求来源）
            let goal_text = goal_lines.join("\n").trim().to_string();
            if plan.requirement.is_empty() {
                plan.requirement = goal_text;
            }
            continue;
        }

        // 现状理解（核心）
        if line == "## 现状理解（核心）" || line == "## 现状理解" {
            i += 1;
            let mut ctx_lines = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim();
                if l == "---" {
                    break;
                }
                ctx_lines.push(lines[i]);
                i += 1;
            }
            plan.context = ctx_lines.join("\n").trim().to_string();
            continue;
        }

        // backward compatibility: old format uses "任务拆解" header
        if line == "## 执行范围" || line == "## 任务拆解" {
            i += 1;
            while i < lines.len() {
                let l = lines[i].trim();
                if l.starts_with("### ") {
                    let (task, new_i) = parse_direction_block(&lines, i)?;
                    plan.tasks.push(task);
                    i = new_i;
                } else if l.starts_with("## ") && !l.starts_with("### ") {
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        i += 1;
    }

    plan.tasks.sort_by_key(|t| t.id);
    Ok(plan)
}

fn parse_direction_block(lines: &[&str], start: usize) -> Result<(ParsedTask, usize), String> {
    let mut task = ParsedTask::default();
    let header = lines[start].trim();

    // 解析标题: ### 方向N: name  或  ### Task-N: name
    if let Some(rest) = header.strip_prefix("### ") {
        let rest = rest.trim();
        // 尝试找冒号分隔 id 和 name
        if let Some((id_part, name_part)) = rest.split_once(':') {
            let id_part = id_part.trim();
            let name = name_part.trim().to_string();

            // 提取 id: "方向1" 或 "Task-1" 或 "1"
            let id_str = id_part
                .strip_prefix("方向")
                .or_else(|| id_part.strip_prefix("Task-"))
                .or_else(|| id_part.strip_prefix("Task"))
                .unwrap_or(id_part);

            task.id = id_str.parse::<usize>().unwrap_or(0);
            task.name = name;
        } else {
            // 没有冒号，整行作为 name
            task.name = rest.to_string();
        }
    }

    let mut i = start + 1;
    let mut understanding_lines = Vec::new();

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // 下一个方向或新 section
        if trimmed.starts_with("### ")
            || (trimmed.starts_with("## ") && !trimmed.starts_with("### "))
        {
            break;
        }

        // 分隔线：只有当下一个非空行是新的方向/section 时才终止
        if trimmed == "---" {
            if i + 1 < lines.len() {
                let next = lines[i + 1].trim();
                if next.starts_with("### ")
                    || (next.starts_with("## ") && !next.starts_with("### "))
                    || next.is_empty()
                {
                    i += 1;
                    break;
                }
            }
            i += 1;
            continue;
        }

        understanding_lines.push(line);
        i += 1;
    }

    task.understanding = understanding_lines.join("\n").trim().to_string();
    Ok((task, i))
}

// ── Markdown 生成器 ──

fn generate_plan_md(plan: &ParsedPlan) -> String {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut md = String::new();

    md.push_str(&format!("# 计划：{} / {}\n\n", plan.project, plan.topic));

    md.push_str("## 元信息\n");
    md.push_str("- 版本: v1\n");
    md.push_str(&format!("- 创建: {}\n", today));
    md.push_str(&format!(
        "{} {}\n",
        meta_prefix("status"),
        if plan.status.is_empty() {
            meta_default("status")
        } else {
            &plan.status
        }
    ));
    md.push_str(&format!("{} {}\n", meta_prefix("project"), plan.project));
    md.push_str(&format!(
        "{} {}\n",
        meta_prefix("requirement"),
        plan.requirement
    ));
    md.push_str(&format!(
        "{} {}\n",
        meta_prefix("goal_type"),
        if plan.goal_type.is_empty() {
            meta_default("goal_type")
        } else {
            &plan.goal_type
        }
    ));
    md.push_str("\n---\n\n");

    md.push_str("## 验收\n\n");
    if plan.requirement.is_empty() {
        md.push_str("（待 Leader 补充）\n");
    } else {
        md.push_str(&plan.requirement);
        md.push('\n');
    }
    md.push_str("\n---\n\n");

    md.push_str("## 现状理解（核心）\n\n");
    if plan.context.is_empty() {
        md.push_str("（待 Leader 补充）\n");
    } else {
        md.push_str(&plan.context);
        md.push('\n');
    }
    md.push_str("\n---\n\n");

    md.push_str("## 执行范围\n\n");
    for task in &plan.tasks {
        md.push_str(&format!("### 方向{}: {}\n\n", task.id, task.name));
        if task.understanding.is_empty() {
            md.push_str("（待 Leader 补充）\n");
        } else {
            md.push_str(&task.understanding);
            md.push('\n');
        }
        md.push('\n');
    }

    md.push_str("## 执行日志\n\n");
    md.push_str("| 时间 | 事件 | 详情 |\n");
    md.push_str("|------|------|------|\n");

    md
}

// ── planner_create ──

fn planner_create(params: &serde_json::Value, _ctx: &ToolCtx) -> Result<ToolResult, String> {
    let project = params
        .get("project")
        .and_then(|v| v.as_str())
        .unwrap_or("nuphus");
    let topic = params
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or("untitled");
    let requirement = params
        .get("requirement")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let goal_type = params
        .get("goal_type")
        .and_then(|v| v.as_str())
        .unwrap_or("code_generation");
    let context = params.get("context").and_then(|v| v.as_str()).unwrap_or("");
    let tasks_arr = params
        .get("tasks")
        .and_then(|v| v.as_array())
        .unwrap_or(&vec![])
        .clone();

    if topic.is_empty() {
        return Ok(ToolResult::failure("topic cannot be empty"));
    }

    let safe_topic: String = topic
        .chars()
        .filter(|c| !c.is_control() && !['<', '>', ':', '"', '/', '\\', '|', '?', '*'].contains(c))
        .collect();
    let root = resolve_project_root();
    let plan_dir = get_plan_dir(&root, project);
    let mut filename = format!("{}.plan.md", safe_topic);
    let mut plan_path = plan_dir.join(&filename);
    let mut counter = 1;
    while plan_path.exists() {
        filename = format!("{}({}).plan.md", safe_topic, counter);
        plan_path = plan_dir.join(&filename);
        counter += 1;
    }

    ensure_plan_dir(&plan_path)?;

    let mut plan = ParsedPlan {
        project: project.to_string(),
        topic: topic.to_string(),
        requirement: requirement.to_string(),
        status: "active".to_string(),
        goal_type: goal_type.to_string(),
        context: context.to_string(),
        tasks: Vec::new(),
    };

    for (idx, task_val) in tasks_arr.iter().enumerate() {
        let id = idx + 1;
        let name = task_val
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let understanding = task_val
            .get("understanding")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        plan.tasks.push(ParsedTask {
            id,
            name,
            understanding,
        });
    }

    let md = generate_plan_md(&plan);
    std::fs::write(&plan_path, md).map_err(|e| format!("write plan file failed: {}", e))?;

    let path_str = plan_path.to_string_lossy().to_string();

    // 构建返回的 plan 结构，避免前端再调 planner_parse
    let tasks_json: Vec<serde_json::Value> = plan
        .tasks
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "name": t.name,
                "understanding": t.understanding,
            })
        })
        .collect();

    Ok(ToolResult::success(
        serde_json::json!({
            "plan_path": path_str,
            "plan": {
                "project": plan.project,
                "topic": plan.topic,
                "requirement": plan.requirement,
                "status": plan.status,
                "goal_type": plan.goal_type,
                "context": plan.context,
                "tasks": tasks_json,
            }
        })
        .to_string(),
    ))
}

// ═══════════════════════════════════════════════════════════════
// planner_parse
// ═══════════════════════════════════════════════════════════════

fn planner_parse(params: &serde_json::Value, _ctx: &ToolCtx) -> Result<ToolResult, String> {
    let plan_path_raw = params
        .get("plan_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if plan_path_raw.is_empty() {
        return Ok(ToolResult::failure("plan_path cannot be empty"));
    }

    let path = resolve_plan_path(plan_path_raw);
    if !path.exists() {
        return Ok(ToolResult::failure(format!(
            "plan file not found: {}",
            plan_path_raw
        )));
    }

    let md_content =
        std::fs::read_to_string(&path).map_err(|e| format!("read plan failed: {}", e))?;

    let plan = parse_plan_md(&md_content).map_err(|e| format!("parse plan failed: {}", e))?;

    let tasks_json: Vec<serde_json::Value> = plan
        .tasks
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "name": t.name,
                "understanding": t.understanding,
            })
        })
        .collect();

    let result = serde_json::json!({
        "project": plan.project,
        "topic": plan.topic,
        "requirement": plan.requirement,
        "status": plan.status,
        "goal_type": plan.goal_type,
        "context": plan.context,
        "tasks": tasks_json,
        "plan_path": plan_path_raw,
    });

    Ok(ToolResult::success(result.to_string()))
}

// ═══════════════════════════════════════════════════════════════
// planner_complete
// ═══════════════════════════════════════════════════════════════

fn planner_complete(params: &serde_json::Value, _ctx: &ToolCtx) -> Result<ToolResult, String> {
    let plan_path = params
        .get("plan_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let audit_note = params
        .get("audit_note")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if plan_path.is_empty() {
        return Ok(ToolResult::failure("plan_path cannot be empty"));
    }

    let path = resolve_plan_path(plan_path);
    if !path.exists() {
        return Ok(ToolResult::failure(format!(
            "plan file not found: {}",
            plan_path
        )));
    }

    let mut content =
        std::fs::read_to_string(&path).map_err(|e| format!("read plan failed: {}", e))?;

    if !audit_note.is_empty() {
        let note_block = format!("\n### 执行审计\n\n{}\n", audit_note);
        content.push_str(&note_block);
    }

    content = content.replace("- 状态: active", "- 状态: archived");

    // 写入 archive 子目录
    let parent = path
        .parent()
        .ok_or_else(|| "cannot determine parent dir".to_string())?;
    let archive_dir = parent.join("archive");
    std::fs::create_dir_all(&archive_dir)
        .map_err(|e| format!("create archive dir failed: {}", e))?;

    let archive_path = archive_dir.join(
        path.file_name()
            .ok_or_else(|| "invalid filename".to_string())?,
    );
    std::fs::write(&archive_path, content)
        .map_err(|e| format!("write archived plan failed: {}", e))?;

    // 删除原文件
    std::fs::remove_file(&path).map_err(|e| format!("remove original plan failed: {}", e))?;

    let archive_path_str = archive_path.to_string_lossy().to_string();
    let result = serde_json::json!({
        "plan_path": plan_path,
        "archive_path": archive_path_str,
        "status": "archived",
        "message": "Plan 已归档到 archive 子目录。",
    });

    Ok(ToolResult::success(result.to_string()))
}

// ═══════════════════════════════════════════════════════════════
// planner_list
// ═══════════════════════════════════════════════════════════════

fn planner_list(params: &serde_json::Value, _ctx: &ToolCtx) -> Result<ToolResult, String> {
    let project = params
        .get("project")
        .and_then(|v| v.as_str())
        .unwrap_or("nuphus");
    let status_filter = params.get("status").and_then(|v| v.as_str());

    let root = resolve_project_root();
    let plan_dir = get_plan_dir(&root, project);

    let mut plans = Vec::new();

    if plan_dir.exists() {
        let entries =
            std::fs::read_dir(&plan_dir).map_err(|e| format!("read dir failed: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("dir entry error: {}", e))?;
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            if !name.ends_with(".plan.md") {
                continue;
            }

            let meta = std::fs::metadata(&path).ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Local> = t.into();
                    dt.format("%Y-%m-%d %H:%M").to_string()
                })
                .unwrap_or_default();

            let file_content = std::fs::read_to_string(&path).unwrap_or_default();
            let state = if file_content.contains("- 状态: archived") {
                "archived"
            } else if file_content.contains("- 状态: active") {
                "active"
            } else {
                "unknown"
            };

            if let Some(filter) = status_filter {
                if state != filter {
                    continue;
                }
            }

            plans.push(serde_json::json!({
                "name": name,
                "path": path.to_string_lossy().to_string(),
                "status": state,
                "modified": modified,
                "size": size,
            }));
        }
    }

    plans.sort_by(|a, b| {
        let ma = a.get("modified").and_then(|v| v.as_str()).unwrap_or("");
        let mb = b.get("modified").and_then(|v| v.as_str()).unwrap_or("");
        mb.cmp(ma)
    });

    Ok(ToolResult::success(
        serde_json::to_string_pretty(&serde_json::json!({
            "project": project,
            "count": plans.len(),
            "plans": plans,
        }))
        .unwrap_or_default(),
    ))
}

// ═══════════════════════════════════════════════════════════════
// 注册到 ToolRegistry
// ═══════════════════════════════════════════════════════════════

impl ToolRegistry {
    pub(crate) fn register_planner_create(&mut self) {
        self.register(ToolDef {
            name: "planner_create".to_string(),
            description: "Create a plan doc. All same-goal-type tasks in one dispatch. Call leader_memory_update after each phase.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": { "type": "string", "default": "nuphus", "description": "Project name" },
                    "topic": { "type": "string", "description": "Plan topic" },
                    "requirement": { "type": "string", "description": "Original requirement / success criteria" },
                    "goal_type": { "type": "string", "enum": ["project_analysis", "code_generation", "debug_diagnose", "file_operation", "research_query", "general"], "default": "code_generation", "description": "Goal type for dispatch" },
                    "context": { "type": "string", "description": "Leader's understanding: current situation, constraints, known risks, dependencies" },
                    "tasks": {
                        "type": "array",
                        "description": "All tasks of same goal_type — execute as single dispatch, not per-task",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string", "description": "Direction name" },
                                "understanding": { "type": "string", "description": "Background, constraints, risks, suggestions for Exec" }
                            },
                            "required": ["name"]
                        }
                    }
                },
                "required": ["topic"]
            }),
            category: ToolCategory::Core,
            executor: planner_create,
            depends_on: vec![],
        });
    }

    pub(crate) fn register_planner_parse(&mut self) {
        self.register(ToolDef {
            name: "planner_parse".to_string(),
            description: "Parse .plan.md into JSON for programmatic read".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "plan_path": { "type": "string", "description": "Path to .plan.md file" }
                },
                "required": ["plan_path"]
            }),
            category: ToolCategory::Core,
            executor: planner_parse,
            depends_on: vec![],
        });
    }

    pub(crate) fn register_planner_complete(&mut self) {
        self.register(ToolDef {
            name: "planner_complete".to_string(),
            description: "Archive plan after all tasks done — writes audit note and sets status to archived. 调用前必须先 leader_memory_update。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "plan_path": { "type": "string", "description": "Path to .plan.md file" },
                    "task_results": {
                        "type": "array",
                        "description": "Audit results per direction",
                        "items": {
                            "type": "object",
                            "properties": {
                                "task_id": { "type": "integer" },
                                "passed": { "type": "boolean" },
                                "note": { "type": "string" }
                            }
                        }
                    },
                    "audit_note": { "type": "string", "description": "Leader's overall audit note" }
                },
                "required": ["plan_path"]
            }),
            category: ToolCategory::Core,
            executor: planner_complete,
            depends_on: vec![],
        });
    }

    pub(crate) fn register_planner_list(&mut self) {
        self.register(ToolDef {
            name: "planner_list".to_string(),
            description: "List plan docs for project".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "project": { "type": "string", "default": "nuphus", "description": "Project name" },
                    "status": { "type": "string", "enum": ["active", "archived", "unknown"], "description": "Filter by status" }
                }
            }),
            category: ToolCategory::Core,
            executor: planner_list,
            depends_on: vec![],
        });
    }
}
